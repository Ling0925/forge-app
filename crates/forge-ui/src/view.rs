//! Root view. Rendering only: every decision comes from `forge_state::AppState`.

use crate::actions::{
    COMPOSER_CONTEXT, CloseSettings, CloseWindow, FocusComposer, InspectorChanges, InspectorOutput,
    InspectorQueue, Minimize, NewSession, NextSession, OpenSettings, PrevSession, Quit,
    ROOT_CONTEXT, RetryLastRun, SETTINGS_CONTEXT, Steer, StopRun, ToggleInspector, ToggleSidebar,
};
use crate::announce::Announcer;
use crate::backend::{Backend, EventReceiver, StartError};
use crate::timeline_view::render_row;
use crate::tokens::{self, Palette};
use forge_protocol::{EventEnvelope, ProviderId, RunId, SessionId};
use forge_state::i18n::{t, tf};
use forge_state::{
    AppState, BootState, ComposerKey, ComposerMode, DisabledReason, ErrorAction, ErrorEffect,
    HistoryState, KeyOutcome, LocalKeyState, OnboardingStep, PanelLayout, QuitDecision,
    SessionRowVm, SidebarGroup, SidebarResize, SidebarVm, StatusIcon, TimelineRowVm,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::{FocusTrapElement, TestSupportExt};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext, Context, CursorStyle, Entity, EntityInputHandler, FocusHandle,
    Focusable, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement, PromptButton, PromptLevel, Render, Role, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
    px, rgb,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Providers shown in the settings panel (Core's `HttpProviderFactory` set).
const PROVIDERS: [(&str, &str); 2] = [("openai", "OpenAI"), ("anthropic", "Anthropic")];

/// Sidebar divider hit area (§4 "命中区 8pt").
const DIVIDER_HIT: f32 = 8.0;

struct SidebarDrag {
    start_x: f32,
    start_width: f32,
}

pub struct ForgeApp {
    pub state: AppState,
    backend: Rc<dyn Backend>,
    announcer: Rc<dyn Announcer>,
    composer: Entity<TextareaState>,
    scroller: Entity<MessageScrollerState>,
    rows: Rc<RefCell<Vec<TimelineRowVm>>>,
    rows_session: Option<SessionId>,
    focus: FocusHandle,
    started: Instant,
    clock_base_ms: u64,
    exiting: bool,
    drag: Option<SidebarDrag>,
    placeholder: &'static str,
    /// Inspector tab: 0 改动, 1 队列, 2 工具输出 (UI-only).
    inspector_tab: usize,
    /// Settings panel key fields (masked), one per provider.
    key_inputs: Vec<(ProviderId, Entity<InputState>)>,
    /// "重试" next to "无法读取钥匙串", one per provider (caller-owned so
    /// focus stays on the button while the retry snapshot is in flight).
    key_retry_focus: Vec<FocusHandle>,
    /// Settings panel container (focus-trap scope) and its 关闭 button.
    settings_focus: FocusHandle,
    settings_close_focus: FocusHandle,
    /// Focus to restore when the settings panel closes (Esc or 关闭).
    settings_return_focus: Option<FocusHandle>,
    /// `ui_prefs` as last loaded or sent; a change is persisted once the
    /// drag ends (sidebar width / visibility, inspector, appearance).
    persisted_prefs: Option<BTreeMap<String, String>>,
    /// The 外观 → 主题 value currently applied to the gpui theme; a change
    /// (settings click or a Core-pushed ui_prefs update) re-applies it.
    applied_appearance: forge_state::Appearance,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ForgeApp {
    pub fn new(
        backend: Rc<dyn Backend>,
        events: Option<EventReceiver>,
        boot_error: Option<StartError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_announcer(
            backend,
            crate::announce::platform(),
            events,
            boot_error,
            window,
            cx,
        )
    }

    pub fn with_announcer(
        backend: Rc<dyn Backend>,
        announcer: Rc<dyn Announcer>,
        events: Option<EventReceiver>,
        boot_error: Option<StartError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10)
                .placeholder(t("composer.placeholder.idle"))
                .submit_on_enter(true)
        });
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let key_inputs = PROVIDERS
            .iter()
            .map(|(id, _)| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .masked(true)
                        .placeholder(t("settings.key.placeholder"))
                });
                (ProviderId::from(*id), input)
            })
            .collect();
        let key_retry_focus = PROVIDERS.iter().map(|_| cx.focus_handle()).collect();
        let subscriptions = vec![
            cx.subscribe_in(&composer, window, Self::on_composer_event),
            // Scrolled to the top of the timeline → next older history page.
            cx.observe_in(&scroller, window, |this, scroller, window, cx| {
                let at_top = scroller.read(cx).logical_scroll_top().item_ix == 0;
                let has_more = this
                    .state
                    .selected()
                    .is_some_and(|vm| vm.history == HistoryState::HasMore);
                if at_top && has_more && !this.rows.borrow().is_empty() {
                    this.state.load_older();
                    this.after_change(window, cx);
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                let pref = this.state.ui.appearance;
                crate::theme::apply_appearance(pref, Some(window), cx);
            }),
        ];

        let mut state = AppState::default();
        match boot_error {
            Some(error) => {
                state.boot = BootState::StorageError {
                    code: error.code,
                    path: error.path,
                }
            }
            None => state.boot = BootState::Ready,
        }

        // P1 boot: ask Core for the snapshot (sessions, settings, key flags,
        // active runs); events below the snapshot's last_seq are dropped.
        if events.is_some() && state.boot == BootState::Ready {
            state.request_snapshot();
        }
        let mut tasks = Vec::new();
        if let Some(mut events) = events {
            tasks.push(cx.spawn_in(window, async move |this, cx| {
                while let Some(envelope) = events.recv().await {
                    let applied =
                        this.update_in(cx, |app, window, cx| app.apply_event(envelope, window, cx));
                    if applied.is_err() {
                        break;
                    }
                }
            }));
        }
        tasks.push(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update_in(cx, |app, window, cx| app.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));

        // Closing the window goes through the same quit flow as ⌘Q.
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |app, cx| app.request_quit(window, cx))
                .unwrap_or(true)
        });

        state.set_utc_offset_ms(crate::clock::local_utc_offset_ms(
            crate::clock::wall_now_ms(),
        ));
        let focus = cx.focus_handle();
        let mut app = Self {
            state,
            backend,
            announcer,
            composer,
            scroller,
            rows: Rc::new(RefCell::new(Vec::new())),
            rows_session: None,
            focus,
            started: Instant::now(),
            clock_base_ms: crate::clock::wall_now_ms(),
            exiting: false,
            drag: None,
            placeholder: t("composer.placeholder.idle"),
            inspector_tab: 0,
            key_inputs,
            key_retry_focus,
            settings_focus: cx.focus_handle(),
            settings_close_focus: cx.focus_handle(),
            settings_return_focus: None,
            persisted_prefs: None,
            applied_appearance: forge_state::Appearance::System,
            _tasks: tasks,
            _subscriptions: subscriptions,
        };
        app.composer.read(cx).focus_handle(cx).focus(window, cx);
        app.after_change(window, cx);
        app
    }

    /// Composer text-area handle (headless tests type into it).
    pub fn composer(&self) -> &Entity<TextareaState> {
        &self.composer
    }

    /// Timeline rows as last rendered.
    pub fn rows(&self) -> Vec<TimelineRowVm> {
        self.rows.borrow().clone()
    }

    pub fn now_ms(&self) -> u64 {
        self.clock_base_ms + self.started.elapsed().as_millis() as u64
    }

    /// Continue the UI clock from `ms` (fixtures replay events at fake times).
    pub fn set_clock_base(&mut self, ms: u64) {
        self.clock_base_ms = ms.saturating_sub(self.started.elapsed().as_millis() as u64);
        self.state.set_now_ms(self.now_ms());
    }

    // ------------------------------------------------------------- plumbing

    pub fn apply_event(
        &mut self,
        envelope: EventEnvelope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.set_now_ms(self.now_ms());
        self.state.apply(envelope);
        self.after_change(window, cx);
    }

    pub fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.set_now_ms(self.now_ms());
        self.state.poll_quit_timeout();
        self.after_change(window, cx);
    }

    /// Ships queued commands, routes announcements, mirrors the draft and the
    /// timeline into their gpui entities, exits if the quit flow finished.
    pub fn after_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persist_prefs_if_changed();
        // 外观 → 主题 (v2.1 settings card): re-apply when the pref changed —
        // from the settings segmented control or a Core `Settings` update.
        let appearance = self.state.ui.appearance;
        if appearance != self.applied_appearance {
            self.applied_appearance = appearance;
            crate::theme::apply_appearance(appearance, Some(window), cx);
        }
        for outgoing in self.state.take_outbox() {
            let token = outgoing.token;
            let future = self.backend.execute(outgoing.command);
            cx.spawn_in(window, async move |this, cx| {
                let result = future.await;
                this.update_in(cx, |app, window, cx| {
                    match result {
                        Some(result) => app.state.apply_result(token, result),
                        None => app.state.transport_failed(token),
                    }
                    app.after_change(window, cx);
                })
                .ok();
            })
            .detach();
        }
        for announcement in self.state.take_announcements() {
            self.announcer
                .announce(&announcement.text, announcement.priority);
        }
        self.sync_composer(window, cx);
        self.sync_timeline(cx);
        self.sync_key_retry_focus(window, cx);
        if self.state.should_exit() && !self.exiting {
            self.exiting = true;
            cx.quit();
        }
        cx.notify();
    }

    /// `ui_prefs` go to Core through `UpdateSettings` once synced, never
    /// mid-drag, and only when they differ from what Core last stored (and
    /// from what was last sent, while Core's `SettingsUpdated` is pending).
    fn persist_prefs_if_changed(&mut self) {
        if !self.state.synced || self.drag.is_some() {
            return;
        }
        let Some(settings) = self.state.settings.settings.as_ref() else {
            return;
        };
        let normalize = |map: &BTreeMap<String, String>| {
            let mut ui = forge_state::UiPrefs::default();
            ui.apply_map(map);
            let mut out = BTreeMap::new();
            ui.write_map(&mut out);
            out
        };
        let stored = normalize(&settings.ui_prefs);
        let mut current = BTreeMap::new();
        self.state.ui.write_map(&mut current);
        if current != stored && self.persisted_prefs.as_ref() != Some(&current) {
            self.persisted_prefs = Some(current);
            self.state.persist_ui_prefs();
        }
    }

    fn sync_timeline(&mut self, cx: &mut Context<Self>) {
        let rows = self.state.timeline_rows();
        let session = self.state.selected_session.clone();
        let old_len = self.rows.borrow().len();
        let changed = *self.rows.borrow() != rows;
        let new_len = rows.len();
        let same_session = session == self.rows_session;
        // An older history page lands at the top: the previous first row
        // reappears `added` rows down. Prepend so the viewport stays put.
        let prepended = same_session && new_len > old_len && old_len > 0 && {
            let old = self.rows.borrow();
            let added = new_len - old_len;
            rows.get(added) == old.first() && rows.first() != old.first()
        };
        *self.rows.borrow_mut() = rows;
        self.rows_session = session;
        self.scroller.update(cx, |scroller, cx| {
            if !same_session || new_len < old_len {
                scroller.reset(new_len, cx);
            } else if prepended {
                scroller.prepend(new_len - old_len, cx);
            } else if new_len > old_len {
                scroller.append(new_len - old_len, cx);
            }
            if changed {
                scroller.remeasure(cx);
            }
        });
    }

    fn sync_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self
            .state
            .selected()
            .map(|vm| vm.composer.draft.clone())
            .unwrap_or_default();
        if self.composer.read(cx).value().as_ref() != draft {
            self.composer
                .update(cx, |input, cx| input.set_value(draft, window, cx));
        }
        // §6.9: placeholder follows the state (no key → "先在设置里添加 API Key";
        // v2.1 running → "运行中"). No key wins even before a session exists.
        let no_key = self.state.settings.key_missing()
            || self.state.composer_mode() == ComposerMode::Disabled(DisabledReason::NoKey);
        let busy = matches!(
            self.state.composer_mode(),
            ComposerMode::Running | ComposerMode::Cancelling
        );
        let placeholder = if no_key {
            t("composer.no_key")
        } else if busy {
            t("composer.placeholder.running")
        } else {
            t("composer.placeholder.idle")
        };
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            self.composer.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
    }

    fn on_composer_event(
        &mut self,
        input: &Entity<TextareaState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let value = input.read(cx).value().to_string();
                self.state.set_draft(value);
                self.sync_composer(window, cx);
                cx.notify();
            }
            // ⇧↩ already inserted a newline inside the text area; ⌘↩ is not a
            // send shortcut in the spec.
            InputEvent::PressEnter {
                shift: false,
                secondary: false,
            } => {
                self.composer_key(ComposerKey::Enter, window, cx);
            }
            _ => {}
        }
    }

    fn composer_key(
        &mut self,
        key: ComposerKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> KeyOutcome {
        // IME rule (UI-CP-02): while marked text exists Enter belongs to the IME.
        let composing = self.composer.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        });
        self.state.set_ime_composing(composing);
        self.state.set_now_ms(self.now_ms());
        let outcome = self.state.composer_key(key);
        self.after_change(window, cx);
        outcome
    }

    /// Returns whether the window may close right away.
    fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.state.request_quit() {
            QuitDecision::ExitNow => {
                self.exiting = true;
                cx.quit();
                true
            }
            QuitDecision::InProgress => false,
            QuitDecision::NeedsConfirm { active_runs } => {
                let title = tf("quit.confirm.title", &[("n", &active_runs.to_string())]);
                // 取消 first: it is the default button (§6.15).
                let answers = [
                    PromptButton::cancel(t("quit.confirm.cancel")),
                    PromptButton::ok(t("quit.confirm.ok")),
                ];
                let answer = window.prompt(
                    PromptLevel::Warning,
                    &title,
                    Some(t("quit.confirm.body")),
                    &answers,
                    cx,
                );
                cx.spawn_in(window, async move |this, cx| {
                    let confirmed = matches!(answer.await, Ok(1));
                    this.update_in(cx, |app, window, cx| {
                        if confirmed {
                            app.state.set_now_ms(app.now_ms());
                            app.state.confirm_quit();
                        } else {
                            app.state.cancel_quit();
                        }
                        app.after_change(window, cx);
                    })
                    .ok();
                })
                .detach();
                cx.notify();
                false
            }
        }
    }

    // ------------------------------------------------------------ intents

    pub fn toggle_reasoning(&mut self, run: &RunId, window: &mut Window, cx: &mut Context<Self>) {
        self.state.toggle_reasoning(run);
        self.after_change(window, cx);
    }

    pub fn error_action(
        &mut self,
        run: &RunId,
        action: ErrorAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.set_now_ms(self.now_ms());
        match self.state.error_action(run, action) {
            ErrorEffect::None => {}
            ErrorEffect::OpenInspectorChanges(_) => {
                self.state.ui.inspector_visible = true;
            }
            ErrorEffect::OpenSettings => self.open_settings(window, cx),
        }
        self.after_change(window, cx);
    }

    pub fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.stop_run();
        self.after_change(window, cx);
    }

    /// Example `n` (1-based, matching `empty.example.{n}`) fills its text.
    fn fill_example(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // UI-TL-04: fill only, focus the composer, never send.
        self.state.fill_example(index);
        self.after_change(window, cx);
        self.composer.read(cx).focus_handle(cx).focus(window, cx);
    }

    // -------------------------------------------------------------- actions

    fn on_toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.toggle_sidebar();
        self.after_change(window, cx);
    }

    fn on_toggle_inspector(
        &mut self,
        _: &ToggleInspector,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.ui.inspector_visible = !self.state.ui.inspector_visible;
        self.after_change(window, cx);
    }

    /// ⌥⌘1/2/3: open the Inspector on 改动 / 队列 / 工具输出 (§6.10).
    fn show_inspector_tab(&mut self, tab: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.inspector_tab = tab;
        self.state.ui.inspector_visible = true;
        self.after_change(window, cx);
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.settings.open {
            self.settings_return_focus = window.focused(cx);
        }
        self.state.open_settings();
        self.after_change(window, cx);
        self.focus_first_settings_control(window, cx);
    }

    /// Initial focus: the first actionable control of the panel's content,
    /// i.e. the first provider row's first control (重试 when its keychain is
    /// unreadable, otherwise its key field). 关闭 is the last tab stop.
    fn focus_first_settings_control(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((provider, input)) = self.key_inputs.first() else {
            return;
        };
        if self.state.settings.key_unavailable(provider) {
            self.key_retry_focus[0].focus(window, cx);
        } else {
            input.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    /// Whether the key field of provider `index` holds focus (tests).
    pub fn key_input_focused(&self, index: usize, window: &Window, cx: &gpui_kit::App) -> bool {
        self.key_inputs
            .get(index)
            .is_some_and(|(_, input)| input.read(cx).focus_handle(cx).is_focused(window))
    }

    /// Timeline scroll position (first visible row, offset in it) for tests.
    pub fn timeline_scroll_top(&self, cx: &gpui_kit::App) -> (usize, gpui_kit::Pixels) {
        let top = self.scroller.read(cx).logical_scroll_top();
        (top.item_ix, top.offset_in_item)
    }

    /// Whether the settings panel's 关闭 button holds focus (tests).
    pub fn settings_close_focused(&self, window: &Window) -> bool {
        self.settings_close_focus.is_focused(window)
    }

    /// Whether focus is anywhere inside the settings panel (tests).
    pub fn settings_contains_focus(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.settings_focus.contains_focused(window, cx)
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.close_settings();
        for (_, input) in &self.key_inputs {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.after_change(window, cx);
        if let Some(handle) = self.settings_return_focus.take() {
            handle.focus(window, cx);
        } else {
            self.composer.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    fn on_close_settings(
        &mut self,
        _: &CloseSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.settings.open {
            self.close_settings(window, cx);
        } else {
            cx.propagate();
        }
    }

    /// Escape from a settings key field (Input maps Esc to this action and
    /// propagates when it has nothing of its own to dismiss).
    fn on_settings_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.settings.open {
            self.close_settings(window, cx);
        } else {
            cx.propagate();
        }
    }

    /// "保存": the key goes to Core (`SetProviderKey`) and the field is
    /// cleared at once; the UI never keeps it. `ProviderKeyConfigured`
    /// flips the status.
    /// "重试" on an unavailable keychain row: re-sends `GetSnapshot`; focus
    /// stays on the button (the result is announced politely).
    fn retry_key_status(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.state.retry_key_status();
        if let Some(handle) = self.key_retry_focus.get(index) {
            handle.focus(window, cx);
        }
        self.after_change(window, cx);
    }

    /// The retry button disappears once the keychain is readable again;
    /// move focus to that provider's key field instead of losing it.
    fn sync_key_retry_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (handle, (provider, input)) in self.key_retry_focus.iter().zip(&self.key_inputs) {
            if handle.is_focused(window)
                && (!self.state.settings.open || !self.state.settings.key_unavailable(provider))
            {
                input.read(cx).focus_handle(cx).focus(window, cx);
            }
        }
    }

    /// Moves focus to the keychain retry button of provider `index` (demo).
    pub fn focus_key_retry(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.key_retry_focus.get(index) {
            handle.focus(window, cx);
        }
    }

    /// Whether the keychain retry button for `index` holds focus (tests).
    pub fn key_retry_focused(&self, index: usize, window: &Window) -> bool {
        self.key_retry_focus
            .get(index)
            .is_some_and(|handle| handle.is_focused(window))
    }

    fn save_key(
        &mut self,
        index: usize,
        remove: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((provider, input)) = self.key_inputs.get(index).cloned() else {
            return;
        };
        let key = if remove {
            None
        } else {
            let value = input.read(cx).value().to_string();
            if value.trim().is_empty() {
                return;
            }
            Some(value)
        };
        input.update(cx, |input, cx| input.set_value("", window, cx));
        self.state.set_provider_key(provider, key);
        self.after_change(window, cx);
    }

    fn on_new_session(&mut self, _: &NewSession, window: &mut Window, cx: &mut Context<Self>) {
        self.state.create_session();
        self.after_change(window, cx);
    }

    fn on_focus_composer(
        &mut self,
        _: &FocusComposer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn on_stop(&mut self, _: &StopRun, window: &mut Window, cx: &mut Context<Self>) {
        self.stop(window, cx);
    }

    fn on_retry_last(&mut self, _: &RetryLastRun, window: &mut Window, cx: &mut Context<Self>) {
        self.state.set_now_ms(self.now_ms());
        self.state.retry_latest();
        self.after_change(window, cx);
    }

    fn on_prev(&mut self, _: &PrevSession, window: &mut Window, cx: &mut Context<Self>) {
        self.state.select_adjacent(false);
        self.after_change(window, cx);
    }

    fn on_next(&mut self, _: &NextSession, window: &mut Window, cx: &mut Context<Self>) {
        self.state.select_adjacent(true);
        self.after_change(window, cx);
    }

    fn on_quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.request_quit(window, cx);
    }

    fn on_close(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_quit(window, cx) {
            window.remove_window();
        }
    }

    fn on_minimize(&mut self, _: &Minimize, window: &mut Window, _: &mut Context<Self>) {
        window.minimize_window();
    }

    fn on_steer(&mut self, _: &Steer, window: &mut Window, cx: &mut Context<Self>) {
        self.composer_key(ComposerKey::AltEnter, window, cx);
    }

    fn select(&mut self, id: &SessionId, window: &mut Window, cx: &mut Context<Self>) {
        self.state.select_session(id);
        self.after_change(window, cx);
    }

    fn primary_button(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.set_now_ms(self.now_ms());
        self.state.primary_button();
        self.after_change(window, cx);
    }

    // ------------------------------------------------------- sidebar resize

    fn on_divider_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.click_count >= 2 {
            self.drag = None;
            self.state.ui.reset_sidebar_width();
            self.after_change(window, cx);
            return;
        }
        self.drag = Some(SidebarDrag {
            start_x: f32::from(event.position.x),
            start_width: self.state.ui.sidebar_width,
        });
    }

    fn on_root_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = &self.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let width = drag.start_width + (f32::from(event.position.x) - drag.start_x);
        if self.state.ui.drag_sidebar(width) == SidebarResize::Collapsed {
            self.drag = None;
        }
        self.after_change(window, cx);
    }

    // --------------------------------------------------------------- render

    fn render_sidebar(&self, sidebar: SidebarVm, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let palette = *Palette::for_dark(theme.is_dark());
        // §6.1 + §0: "＋ 新会话" is a full-width row (32pt, radius-sm) with
        // its ⌘N hint on the right, 8pt from the sidebar edge.
        let new_session = Button::new("new-session")
            .ghost()
            .small()
            .w_full()
            .h(px(tokens::component::sidebar::NEW_SESSION_HEIGHT))
            .px_3()
            .mb_2()
            .justify_start()
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .accessibility_label(t("sidebar.new_session"))
            .tooltip(t("menu.file.new_session"))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(Icon::new(IconName::Plus))
                    .child(t("sidebar.new_session"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(tokens::CAPTION.size))
                            .line_height(px(tokens::CAPTION.line_height))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(rgb(palette.fg3))
                            .child("⌘N"),
                    ),
            )
            .on_click(
                cx.listener(|this, _, window, cx| this.on_new_session(&NewSession, window, cx)),
            );
        let mut list = v_flex()
            .id("sidebar")
            .role(Role::Navigation)
            .aria_label(t("a11y.sidebar"))
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            // Rows sit 8pt from the sidebar edge (v2.1 component sidebar.edge).
            .px(px(tokens::component::sidebar::EDGE))
            .pt_3()
            .pb_3()
            .gap_1()
            .child(new_session);
        if sidebar.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .text_sm()
                    .text_color(rgb(palette.fg2))
                    .child(t("sidebar.empty")),
            );
        }
        // Groups: 需要处理 / 进行中 / 今天 / 昨天 / 过去 7 天 / 更早 / 归档.
        // Headers carry the row count (V2 / UI-REF-03).
        let mut sections: Vec<(String, &'static str, Vec<SessionRowVm>)> = vec![
            (
                "NeedsAttention".into(),
                SidebarGroup::NeedsAttention.label_key(),
                sidebar.needs_attention.clone(),
            ),
            (
                "InProgress".into(),
                SidebarGroup::InProgress.label_key(),
                sidebar.in_progress.clone(),
            ),
        ];
        for (bucket, rows) in sidebar.recent_buckets() {
            sections.push((format!("{bucket:?}"), bucket.label_key(), rows.to_vec()));
        }
        sections.push((
            "Archived".into(),
            SidebarGroup::Archived.label_key(),
            sidebar.archived.clone(),
        ));
        for (key, label_key, rows) in sections {
            if rows.is_empty() {
                continue;
            }
            let count = rows.len();
            let name = t(label_key);
            let label = tf(
                "a11y.sidebar.group",
                &[("group", name), ("n", &count.to_string())],
            );
            // v2.1: caption-size fg3 group titles; the count stays in the
            // group's accessibility label instead of a visual badge.
            let mut section = v_flex()
                .id(SharedString::from(format!("group-{key}")))
                .role(Role::List)
                .aria_label(label)
                .gap_0p5()
                .child(
                    div()
                        .id(SharedString::from(format!("group-{key}-header")))
                        .px_3()
                        .pt_3()
                        .pb_1()
                        .text_size(px(tokens::CAPTION.size))
                        .line_height(px(tokens::CAPTION.line_height))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(rgb(palette.fg3))
                        .child(name)
                        .test_support(),
                );
            for row in rows {
                section = section.child(self.render_row(row, cx));
            }
            list = list.child(section.test_support());
        }
        list.test_support().into_any_element()
    }

    fn render_divider(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        div()
            .id("sidebar-divider")
            .role(Role::Splitter)
            .aria_label(t("a11y.sidebar.resize"))
            .h_full()
            .w(px(DIVIDER_HIT))
            .ml(px(-DIVIDER_HIT / 2.0))
            .mr(px(-DIVIDER_HIT / 2.0))
            .flex_shrink_0()
            .cursor(CursorStyle::ResizeLeftRight)
            .child(div().mx_auto().w(px(1.)).h_full().bg(theme.sidebar_border))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_divider_down))
            .test_support()
            .into_any_element()
    }

    fn render_row(&self, row: SessionRowVm, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let palette = *Palette::for_dark(theme.is_dark());
        // Status colours use the text-safe tokens (≥4.5:1 on sidebar/selected);
        // every state also has its own glyph and text (UI-SB-03).
        let icon: AnyElement = match row.icon {
            StatusIcon::None => div().into_any_element(),
            StatusIcon::Spinner => Spinner::new().xsmall().into_any_element(),
            StatusIcon::Approval => Icon::new(IconName::Shield)
                .text_color(rgb(palette.warning))
                .into_any_element(),
            StatusIcon::Failed => Icon::new(IconName::CircleX)
                .text_color(rgb(palette.danger))
                .into_any_element(),
            StatusIcon::Stopped => Icon::new(IconName::Square)
                .text_color(rgb(palette.fg2))
                .into_any_element(),
            StatusIcon::Interrupted => Icon::new(IconName::TriangleAlert)
                .text_color(rgb(palette.warning))
                .into_any_element(),
            StatusIcon::Uncertain => Icon::new(IconName::CircleAlert)
                .text_color(rgb(palette.warning))
                .into_any_element(),
        };
        let id = row.id.clone();
        let approvals = row.approvals;
        // §6.1 + v2.1: 48pt, two lines, 12pt side padding, radius-sm (8);
        // the selected capsule is neutral `selected` with a Semibold title.
        h_flex()
            .id(SharedString::from(format!("session-{}", row.id.0)))
            .role(Role::ListItem)
            .aria_label(row.a11y_label.clone())
            .aria_selected(row.selected)
            .h(px(tokens::component::sidebar::ROW_HEIGHT))
            .flex_shrink_0()
            .gap_2()
            .px(px(tokens::component::sidebar::ROW_PADDING))
            .py_1()
            .rounded(px(tokens::component::sidebar::ROW_RADIUS))
            .cursor_pointer()
            .when(row.selected, |row| {
                row.bg(rgb(tokens::component::sidebar::selected_bg(&palette)))
                    .text_color(theme.sidebar_accent_foreground)
            })
            .when(!row.selected, |row| {
                row.hover(|s| s.bg(rgb(tokens::component::sidebar::hover_bg(&palette))))
            })
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().size(px(16.)).flex_shrink_0().child(icon))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(tokens::BODY.size))
                                    .line_height(px(tokens::FOOTNOTE.line_height))
                                    .font_weight(if row.selected {
                                        gpui_kit::FontWeight::SEMIBOLD
                                    } else {
                                        gpui_kit::FontWeight::MEDIUM
                                    })
                                    .truncate()
                                    .child(row.title.clone()),
                            )
                            .when(approvals > 0, |line| {
                                line.child(
                                    div()
                                        .px_1p5()
                                        .rounded_full()
                                        .bg(rgb(palette.warning_bg))
                                        .text_color(rgb(palette.warning))
                                        .text_xs()
                                        .child(approvals.to_string()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .pl(px(24.))
                            .text_size(px(tokens::CAPTION.size))
                            .line_height(px(tokens::CAPTION.line_height))
                            .text_color(rgb(palette.fg2))
                            .truncate()
                            .child(row.status_text.clone()),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.select(&id, window, cx)))
            .test_support()
            .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let vm = self.state.selected()?;
        let theme = cx.theme().clone();
        let context = vm
            .context
            .percent()
            .map(|p| tf("header.context", &[("percent", &p.to_string())]));
        let palette = *Palette::for_dark(theme.is_dark());
        // §0: the header is borderless, 52pt tall, 24pt side padding; the
        // model is a `chip` tag next to the title.
        Some(
            h_flex()
                .id("session-header")
                .h(px(tokens::component::header::HEIGHT))
                .flex_shrink_0()
                .px(px(tokens::component::header::PADDING_X))
                .gap_3()
                .items_center()
                .child(
                    div()
                        .id("session-title")
                        .text_size(px(tokens::TITLE3.size))
                        .line_height(px(tokens::TITLE3.line_height))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .truncate()
                        .child(vm.session.title.clone())
                        .test_support(),
                )
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded(px(tokens::RADIUS_SM))
                        .bg(rgb(palette.chip))
                        .text_size(px(tokens::CAPTION.size))
                        .line_height(px(tokens::CAPTION.line_height))
                        .text_color(rgb(palette.fg2))
                        .child(vm.session.model.model.clone()),
                )
                .child(div().flex_1())
                .children(context.map(|c| {
                    div()
                        .text_size(px(tokens::FOOTNOTE.size))
                        .line_height(px(tokens::FOOTNOTE.line_height))
                        .text_color(theme.muted_foreground)
                        .child(c)
                }))
                .test_support()
                .into_any_element(),
        )
    }

    /// Empty screen (v2.1 §5, §0: 48pt bottom pad): title only, the floating
    /// composer and the stacked suggestion rows (icon + Semibold title + →,
    /// descriptions removed). `composer` is the same element the docked
    /// layout renders.
    fn render_empty(&self, cx: &mut Context<Self>, composer: AnyElement) -> AnyElement {
        let theme = cx.theme().clone();
        let palette = *Palette::for_dark(theme.is_dark());
        // `unavailable` is not "未配置 Key" (no banner for an unreadable keychain).
        let no_key = self.state.settings.key_missing();
        let has_session = self.state.selected().is_some();
        let mut col = v_flex()
            .id("timeline-empty")
            .flex_1()
            .items_center()
            .justify_center()
            .px_6()
            .pb(px(tokens::component::empty_state::BOTTOM))
            .child(
                div()
                    .id("empty-title")
                    .text_size(px(tokens::TITLE1.size))
                    .line_height(px(tokens::TITLE1.line_height))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(t("empty.title"))
                    .test_support(),
            );
        if no_key {
            col = col.child(
                h_flex()
                    .id("no-key")
                    .role(Role::Group)
                    .mt_6()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .rounded(px(tokens::RADIUS_SM))
                    .bg(rgb(palette.info_bg))
                    .text_color(rgb(palette.fg))
                    .child(Icon::new(IconName::Info).text_color(rgb(palette.info)))
                    .child(t("empty.no_key"))
                    .child(
                        Button::new("no-key-settings")
                            .label(t("composer.open_settings"))
                            .small()
                            .outline()
                            // §0: the old 28pt button is 32 like the rest.
                            .h(px(tokens::component::button::HEIGHT))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    )
                    .test_support(),
            );
        }
        // Title → composer 24pt; composer → suggestions 16pt.
        col = col.child(div().w_full().mt_6().child(composer));
        if has_session && !no_key {
            let icons = [IconName::Search, IconName::Check, IconName::PenLine];
            col = col.child(
                h_flex().w_full().justify_center().px_8().mt_4().child(
                    v_flex()
                        .w_full()
                        .max_w(px(forge_state::CONTENT_MAX))
                        .gap_2()
                        .children((1..=3).map(|i| {
                            let text = SharedString::from(t(&format!("empty.example.{i}")));
                            Button::new(SharedString::from(format!("example-{i}")))
                                .custom(
                                    gpui_kit::component::button::ButtonCustomVariant::new(cx)
                                        .color(
                                            rgb(tokens::component::card::suggestion::bg(&palette))
                                                .into(),
                                        )
                                        .foreground(rgb(palette.fg).into())
                                        .hover(
                                            rgb(tokens::component::card::suggestion::hover(
                                                &palette,
                                            ))
                                            .into(),
                                        )
                                        .active(
                                            rgb(tokens::component::card::suggestion::hover(
                                                &palette,
                                            ))
                                            .into(),
                                        ),
                                )
                                .w_full()
                                .h_auto()
                                .justify_start()
                                .p(px(tokens::component::card::PADDING))
                                .rounded(px(tokens::RADIUS_MD))
                                .accessibility_label(text.clone())
                                .child(
                                    h_flex()
                                        .w_full()
                                        .gap_3()
                                        .items_center()
                                        .child(
                                            div()
                                                .size(px(16.))
                                                .flex_shrink_0()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(
                                                    Icon::new(icons[i - 1])
                                                        .text_color(rgb(palette.fg)),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(tokens::BODY.size))
                                                .line_height(px(tokens::BODY.line_height))
                                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                                .truncate()
                                                .child(text),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            Icon::new(IconName::ArrowRight)
                                                .text_color(rgb(palette.fg3)),
                                        ),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.fill_example(i, window, cx)
                                }))
                        })),
                ),
            );
        }
        col.test_support().into_any_element()
    }

    fn render_onboarding(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let vm = self.state.onboarding.as_ref()?;
        let theme = cx.theme().clone();
        let steps = [
            (OnboardingStep::Welcome, "onboarding.welcome.title"),
            (OnboardingStep::Workspace, "onboarding.workspace.title"),
            (OnboardingStep::ApiKey, "onboarding.key.title"),
            (OnboardingStep::Permissions, "onboarding.permissions.title"),
        ];
        let title = steps
            .iter()
            .find(|(step, _)| *step == vm.step)
            .map(|(_, key)| t(key))
            .unwrap_or_default();
        let stepper = h_flex().gap_2().children(steps.iter().map(|(step, key)| {
            let current = *step == vm.step;
            let done = *step < vm.step;
            h_flex()
                .gap_1()
                .text_xs()
                .text_color(if current {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .child(
                    div()
                        .size(px(18.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(current || done, |d| {
                            d.bg(theme.primary).text_color(theme.primary_foreground)
                        })
                        .when(!(current || done), |d| {
                            d.border_1().border_color(theme.border)
                        })
                        .child(step.index().to_string()),
                )
                .child(t(key))
        }));
        let body: AnyElement = match vm.step {
            OnboardingStep::Welcome => div()
                .text_color(theme.muted_foreground)
                .child(t("onboarding.welcome.body"))
                .into_any_element(),
            OnboardingStep::Workspace => Button::new("onboarding-pick")
                .label(t("onboarding.workspace.pick"))
                .outline()
                .into_any_element(),
            OnboardingStep::ApiKey => h_flex()
                .gap_2()
                .child(
                    Button::new("onboarding-verify")
                        .label(t("onboarding.key.verify"))
                        .primary(),
                )
                .child(
                    Button::new("onboarding-later")
                        .label(t("onboarding.key.later"))
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.state.onboarding_skip_key();
                            this.after_change(window, cx);
                        })),
                )
                .into_any_element(),
            OnboardingStep::Permissions => div()
                .text_color(theme.muted_foreground)
                .child(t("onboarding.notify"))
                .into_any_element(),
        };
        let can_advance = self.state.onboarding_can_advance();
        let next_label = match vm.step {
            OnboardingStep::Welcome => t("onboarding.start"),
            OnboardingStep::Permissions => t("onboarding.done"),
            _ => t("onboarding.next"),
        };
        Some(
            v_flex()
                .id("onboarding")
                .role(Role::Dialog)
                .aria_label(title.to_owned())
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .w(px(520.))
                        .p_6()
                        .gap_4()
                        .rounded(px(12.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.popover)
                        .shadow_lg()
                        .child(stepper)
                        .child(
                            div()
                                .text_size(px(tokens::TITLE1.size))
                                .line_height(px(tokens::TITLE1.line_height))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(title.to_owned()),
                        )
                        .child(body)
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .when(vm.step != OnboardingStep::Welcome, |row| {
                                    row.child(
                                        Button::new("onboarding-back")
                                            .label(t("onboarding.back"))
                                            .ghost()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.state.onboarding_back();
                                                this.after_change(window, cx);
                                            })),
                                    )
                                })
                                .child(crate::style::disabled_readable(
                                    Button::new("onboarding-next")
                                        .label(next_label)
                                        .primary()
                                        .disabled(!can_advance)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.state.onboarding_next();
                                            this.after_change(window, cx);
                                        })),
                                    !can_advance,
                                    cx,
                                )),
                        ),
                )
                .test_support()
                .into_any_element(),
        )
    }

    /// Timeline scroller. The empty screen is rendered by `render` itself so
    /// the composer can sit inside it; this only handles onboarding and rows.
    fn render_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(onboarding) = self.render_onboarding(cx) {
            return onboarding;
        }
        let rows = self.rows.clone();
        let weak = cx.weak_entity();
        div()
            .id("timeline")
            .role(Role::Log)
            .aria_label(t("a11y.timeline"))
            .flex_1()
            .min_h_0()
            .child(
                MessageScroller::new(
                    "timeline-scroller",
                    self.scroller.clone(),
                    move |index, _, cx| {
                        let row = rows.borrow().get(index).cloned();
                        match row {
                            // v2.1 §2: 12pt inside a turn, 24pt between turns
                            // (a Divider ends one).
                            Some(row) => {
                                let gap = match rows.borrow().get(index.wrapping_sub(1)) {
                                    Some(TimelineRowVm::Divider { .. }) if index > 0 => {
                                        tokens::component::timeline::TURN_GAP
                                    }
                                    _ => tokens::component::timeline::ROW_GAP,
                                };
                                div()
                                    .pt(px(gap))
                                    .child(render_row(&row, &weak, cx))
                                    .into_any_element()
                            }
                            None => div().into_any_element(),
                        }
                    },
                )
                .with_jump_button_label(t("timeline.jump_latest"))
                .size_full(),
            )
            .test_support()
            .into_any_element()
    }

    /// Floating composer (v2.1 §0/§2/§4): `surface` card, radius-xl,
    /// `shadow-sm`, 1pt `borderStrong` (1.5pt `accent` + 3pt 16% glow on
    /// focus). The 32pt circular send and the "■ 停止" pill sit 8pt from the
    /// card border. §0: no idle hint row — the "↩ 发送 · ⇧↩ 换行" /
    /// "停止 ⌘." shortcuts live on the buttons' tooltips and accessibility
    /// descriptions; the only remaining line is the no-key notice or the
    /// queue-capable running hint.
    fn render_composer(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let palette = *Palette::for_dark(theme.is_dark());
        use tokens::component::composer as cc;
        let mode = self.state.composer_mode();
        let disabled = matches!(mode, ComposerMode::Disabled(_));
        let no_key = mode == ComposerMode::Disabled(DisabledReason::NoKey);
        let sending = mode == ComposerMode::Sending;
        let busy = matches!(mode, ComposerMode::Running | ComposerMode::Cancelling);
        let error = self
            .state
            .selected()
            .and_then(|vm| vm.composer.error.clone());
        let button: AnyElement = if busy {
            // v2.1: "■ 停止" outline pill (borderStrong / fg), Semibold; ⌘.
            // moves to the tooltip and the AccessKit description. While
            // cancelling it is a readable disabled control, never faded.
            let stopping = mode == ComposerMode::Cancelling;
            let button = Button::new("composer-stop")
                .icon(IconName::Square)
                .label(if stopping {
                    t("composer.stopping")
                } else {
                    t("composer.stop")
                })
                .accessibility_label(if stopping {
                    t("composer.stopping")
                } else {
                    t("composer.stop")
                })
                .tooltip(t("composer.stop_key"))
                .custom(
                    gpui_kit::component::button::ButtonCustomVariant::new(cx)
                        .foreground(rgb(cc::stop_fg(&palette)).into())
                        .hover(rgb(cc::stop_hover(&palette)).into())
                        .active(rgb(cc::stop_hover(&palette)).into()),
                )
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .h(px(cc::SEND_SIZE))
                .px_3()
                .rounded(px(tokens::RADIUS_LG))
                .border_1()
                .border_color(rgb(cc::stop_border(&palette)))
                .disabled(stopping)
                .on_click(cx.listener(|this, _, window, cx| this.primary_button(window, cx)));
            crate::style::disabled_readable(button, stopping, cx).into_any_element()
        } else {
            // 32pt circle, accent/onAccent; disabled keeps fg3 on subtle
            // (5.00:1 light / 4.71:1 dark), the v2.1 "send.off" state.
            let button_disabled = !self.state.can_send();
            let button = Button::new("composer-send")
                .icon(IconName::ArrowUp)
                .accessibility_label(t("composer.send"))
                .tooltip(t("composer.send_key"))
                .primary()
                .with_size(gpui_kit::component::Size::Size(px(cc::SEND_SIZE)))
                .rounded_full()
                .disabled(button_disabled)
                .on_click(cx.listener(|this, _, window, cx| this.primary_button(window, cx)));
            crate::style::disabled_readable(button, button_disabled, cx).into_any_element()
        };

        let focused = !disabled && self.composer.read(cx).focus_handle(cx).is_focused(window);
        let workspace = self
            .state
            .active_workspace
            .as_ref()
            .and_then(|id| self.state.workspaces.iter().find(|w| &w.id == id))
            .map(|w| w.name.clone());
        let model = self
            .state
            .selected()
            .map(|vm| vm.session.model.model.clone())
            .or_else(|| {
                self.state
                    .settings
                    .settings
                    .as_ref()
                    .map(|s| s.model.model.clone())
            })
            .unwrap_or_default();

        // Card: padding 16 top/sides, 8 bottom so the control row ends 8pt
        // from the border. Focus swaps the resting borderStrong for the 1.5pt
        // accent stroke plus the 3pt 16% glow (stacked on shadow-sm).
        let shadows = theme.shadow;
        let mut elevation = if shadows {
            crate::style::shadow_sm(theme.is_dark())
        } else {
            Vec::new()
        };
        if focused && shadows {
            elevation.push(crate::style::focus_glow(cc::focus_glow(&palette)));
        }
        let field = v_flex()
            .id("composer-box")
            .w_full()
            .rounded(px(cc::RADIUS))
            .bg(rgb(cc::fill(&palette)))
            .border(px(if focused {
                cc::BORDER_FOCUS
            } else {
                cc::BORDER
            }))
            .border_color(rgb(if focused {
                cc::focus_border(&palette)
            } else {
                cc::resting_border(&palette)
            }))
            .shadow(elevation)
            .px(px(cc::PADDING))
            .pt(px(cc::PADDING))
            .pb(px(cc::CONTROL_INSET))
            .child(
                Textarea::new(&self.composer)
                    .aria_label(t("a11y.composer"))
                    .appearance(false)
                    // Not `.disabled()`: gpui-kit fades disabled text to 50%.
                    // The input stays read-only and the send button is disabled.
                    .readonly(sending || disabled),
            )
            .child(
                h_flex()
                    .id("composer-toolbar")
                    .h(px(cc::SEND_SIZE))
                    .mt(px(cc::CONTROL_INSET))
                    .mr(px(-cc::CONTROL_INSET))
                    .gap_2()
                    .items_center()
                    .text_size(px(tokens::FOOTNOTE.size))
                    .line_height(px(tokens::FOOTNOTE.line_height))
                    .text_color(rgb(palette.fg2))
                    .child(
                        Button::new("composer-attach")
                            .icon(gpui_kit::assets::IconName::Paperclip)
                            .ghost()
                            .xsmall()
                            .accessibility_label(t("composer.attach"))
                            .tooltip(t("composer.attach")),
                    )
                    .children(workspace.map(|name| {
                        // Workspace chip: `chip` fill, radius-sm, fg2 (mock .ws).
                        h_flex()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded(px(tokens::RADIUS_SM))
                            .bg(rgb(cc::chip_bg(&palette)))
                            .text_color(rgb(cc::chip_fg(&palette)))
                            .child(Icon::new(IconName::Folder).xsmall())
                            .child(name)
                    }))
                    .child(div().flex_1())
                    .child(div().child(model))
                    .child(button)
                    .test_support(),
            )
            .test_support();

        // §0: the idle hint line is gone — the send button's tooltip and
        // accessibility description carry "↩ 发送 · ⇧↩ 换行". Only the
        // actionable notices remain: the no-key link and, when running with
        // queue_when_busy, "↩ 加入队列 · ⌥↩ 插话".
        let hint: Option<AnyElement> = if no_key {
            Some(
                h_flex()
                    .id("composer-hint")
                    .role(Role::Group)
                    .aria_label(t("composer.no_key"))
                    .gap_1()
                    .child(t("composer.no_key"))
                    .child(
                        Button::new("composer-open-settings")
                            .label(t("composer.open_settings"))
                            .link()
                            .xsmall()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    )
                    .test_support()
                    .into_any_element(),
            )
        } else {
            self.state.composer_hint_key().map(|key| {
                div()
                    .id("composer-hint")
                    .role(Role::Group)
                    .aria_label(t(key))
                    .child(t(key))
                    .test_support()
                    .into_any_element()
            })
        };

        h_flex()
            .w_full()
            .justify_center()
            .flex_shrink_0()
            .px_8()
            .pb(px(cc::BOTTOM))
            .pt_2()
            .child(
                v_flex()
                    .id("composer")
                    .key_context(COMPOSER_CONTEXT)
                    .on_action(cx.listener(Self::on_steer))
                    .w_full()
                    .max_w(px(forge_state::CONTENT_MAX))
                    .gap_1()
                    .child(field)
                    .when_some(error, |col, error| {
                        col.child(
                            div()
                                .id("composer-error")
                                .role(Role::Alert)
                                .text_size(px(tokens::FOOTNOTE.size))
                                .text_color(rgb(palette.danger))
                                .child(error),
                        )
                    })
                    .when_some(hint, |col, hint| {
                        col.child(
                            div()
                                .w_full()
                                .text_center()
                                .text_size(px(tokens::FOOTNOTE.size))
                                .line_height(px(tokens::FOOTNOTE.line_height))
                                .text_color(rgb(palette.fg3))
                                .child(hint),
                        )
                    })
                    .test_support(),
            )
            .into_any_element()
    }

    fn render_inspector(&self, width: f32, overlay: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let palette = *Palette::for_dark(theme.is_dark());
        let tab = self.inspector_tab;
        // TODO(core-p2): checkpoints / tool output; tab counts ("改动 3", V2)
        // appear once that data exists.
        let empty = match tab {
            0 => t("inspector.empty.changes").to_owned(),
            1 => {
                let n = self.state.selected().map_or(0, |vm| vm.pending_queue_len());
                if n == 0 {
                    t("inspector.empty.queue").to_owned()
                } else {
                    tf("inspector.queue.count", &[("n", &n.to_string())])
                }
            }
            _ => t("inspector.empty.output").to_owned(),
        };
        let tabs = [
            ("inspector.tab.changes", "⌥⌘1"),
            ("inspector.tab.queue", "⌥⌘2"),
            ("inspector.tab.output", "⌥⌘3"),
        ];
        v_flex()
            .id("inspector")
            .role(Role::Complementary)
            .aria_label(t("inspector.title"))
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .bg(theme.background)
            .border_l_1()
            .border_color(theme.border)
            .when(overlay, |el| el.absolute().top_0().right_0().shadow_lg())
            .child(
                h_flex()
                    .h(px(52.))
                    .px_3()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        TabBar::new("inspector-tabs")
                            .segmented()
                            .small()
                            .w_full()
                            .selected_index(tab)
                            .children(tabs.iter().map(|(key, keys)| {
                                Tab::new()
                                    .label(t(key))
                                    .aria_label(tf(
                                        "a11y.inspector.tab",
                                        &[("label", t(key)), ("keys", *keys)],
                                    ))
                                    // 2026-11-14: shortcut as a Kbd-style chip in fg3.
                                    .suffix(crate::style::kbd(keys, &palette))
                            }))
                            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                this.show_inspector_tab(*ix, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .id("inspector-empty")
                    .aria_label(empty.clone())
                    .p_3()
                    .text_sm()
                    .text_color(rgb(palette.fg2))
                    .child(empty)
                    .test_support(),
            )
            .test_support()
            .into_any_element()
    }

    /// §6.12: a store that cannot be opened (Runtime::new / Store::open
    /// failed) replaces the whole window: title, path + error code, and
    /// 打开日志目录 / 在 Finder 中显示 / 退出.
    fn render_storage_error(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let BootState::StorageError { path, code } = &self.state.boot else {
            return None;
        };
        let theme = cx.theme().clone();
        let palette = *Palette::for_dark(theme.is_dark());
        let reveal = path.clone();
        let logs = path.join("logs");
        Some(
            v_flex()
                .id("storage-error")
                .role(Role::Alert)
                .aria_label(t("storage.title"))
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .bg(theme.background)
                .text_color(rgb(palette.fg))
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .size(px(32.))
                        .text_color(rgb(palette.danger)),
                )
                .child(
                    div()
                        .text_size(px(tokens::TITLE1.size))
                        .line_height(px(tokens::TITLE1.line_height))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(t("storage.title")),
                )
                .child(
                    div()
                        .id("storage-error-detail")
                        .max_w(px(520.))
                        .text_sm()
                        .text_color(rgb(palette.fg2))
                        .child(tf(
                            "storage.detail",
                            &[("path", &path.display().to_string()), ("code", code)],
                        ))
                        .test_support(),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .pt_2()
                        .child(
                            Button::new("storage-logs")
                                .label(t("storage.action.logs"))
                                .outline()
                                .on_click(move |_, _, cx| cx.reveal_path(&logs)),
                        )
                        .child(
                            Button::new("storage-reveal")
                                .label(t("storage.action.reveal"))
                                .outline()
                                .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
                        )
                        .child(
                            Button::new("storage-quit")
                                .label(t("storage.action.quit"))
                                .primary()
                                .on_click(|_, _, cx| cx.quit()),
                        ),
                )
                .test_support()
                .into_any_element(),
        )
    }

    /// "模型与 Key" (§6.13, P1 subset) as an in-window dialog, grouped per the
    /// v2.1 mock: 模型服务 (provider key rows + 默认模型), 外观 (主题
    /// segmented control), 关于 (version badge). Rows are title + value /
    /// control; the keychain note moved into the ⓘ tooltips.
    fn render_settings(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.state.settings.open {
            return None;
        }
        let theme = cx.theme().clone();
        let dark = theme.is_dark();
        let palette = *Palette::for_dark(dark);
        let shadows = theme.shadow;
        use tokens::component::card::settings_bg;
        use tokens::component::dialog as dg;

        // Settings item row inside a group card (`.it`): padded, children
        // supplied by the caller; `first` rows carry no top hairline.
        let item = |children: Vec<AnyElement>, first: bool| {
            let el = h_flex()
                .w_full()
                .min_w_0()
                .min_h(px(52.))
                .items_center()
                .px(px(tokens::component::card::PADDING))
                .py_2()
                .gap_3();
            let el = if first {
                el
            } else {
                el.border_t_1().border_color(rgb(dg::separator(&palette)))
            };
            el.children(children)
        };
        let info = |id: SharedString, note: SharedString| {
            Button::new(id)
                .icon(IconName::Info)
                .ghost()
                .xsmall()
                // ⓘ notes tab after the index-0 controls and before 关闭
                // (index 2), keeping the first row control first.
                .tab_index(1)
                .accessibility_label(note.clone())
                .tooltip(note)
        };
        let badge = |icon: Option<IconName>, text: String, fg: u32| {
            h_flex()
                .gap_1()
                .items_center()
                .flex_shrink_0()
                .text_size(px(tokens::CAPTION.size))
                .line_height(px(tokens::CAPTION.line_height))
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(rgb(fg))
                .children(icon.map(|icon| Icon::new(icon).xsmall().text_color(rgb(fg))))
                .child(text)
                .into_any_element()
        };
        // Row title (Semibold, flexes) + optional ⓘ whose tooltip and
        // AccessKit description carry the explanatory note (v2.1).
        let item_title = |name: &'static str, note: Option<(SharedString, SharedString)>| {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .truncate()
                        .child(name),
                )
                .children(note.map(|(id, note)| info(id, note)))
                .into_any_element()
        };
        let note_of = |unavailable: bool| {
            if unavailable {
                SharedString::from(t("settings.keychain_denied"))
            } else {
                SharedString::from(t("settings.key.note"))
            }
        };

        // ---- 模型服务: one item per provider (name + ⓘ + badge + controls),
        // then the read-only 默认模型 row.
        let mut service_items: Vec<AnyElement> = Vec::new();
        for (index, ((id, name), (provider, input))) in
            PROVIDERS.iter().zip(self.key_inputs.iter()).enumerate()
        {
            let configured = self.state.settings.has_key_for(provider);
            let unavailable = self.state.settings.key_unavailable(provider);
            let local = self.state.settings.key_status.get(provider);
            let failed = matches!(local, Some(LocalKeyState::Failed { .. }));
            let status = match local {
                _ if unavailable => t("settings.key.status.unavailable").to_owned(),
                Some(LocalKeyState::SavedUnverified) => t("settings.key.status.saved").to_owned(),
                Some(LocalKeyState::Testing) => t("settings.key.status.testing").to_owned(),
                Some(LocalKeyState::Verified) => t("settings.key.status.verified").to_owned(),
                Some(LocalKeyState::Failed { message, .. }) => tf(
                    "settings.key.status.failed",
                    &[("reason", message.as_deref().unwrap_or(""))],
                ),
                _ if configured => t("settings.key.status.saved").to_owned(),
                _ => t("settings.key.status.unconfigured").to_owned(),
            };
            // Badge: ✓ success when verified, ✕ danger on failure, fg2 else.
            let (badge_icon, badge_fg) = if unavailable || failed {
                (Some(IconName::CircleX), palette.danger)
            } else if matches!(local, Some(LocalKeyState::Verified)) {
                (Some(IconName::CircleCheck), palette.success)
            } else {
                (None, palette.fg2)
            };
            service_items.push(
                v_flex()
                    .id(SharedString::from(format!("settings-key-{id}")))
                    .role(Role::Group)
                    .aria_label(tf(
                        "a11y.settings.row",
                        &[("name", name), ("status", &status)],
                    ))
                    .w_full()
                    .min_w_0()
                    .min_h(px(52.))
                    .px(px(tokens::component::card::PADDING))
                    .py_2()
                    .gap_2()
                    .when(!service_items.is_empty(), |el| {
                        el.border_t_1().border_color(rgb(dg::separator(&palette)))
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .flex_wrap()
                            .gap_x_2()
                            .gap_y_1()
                            .items_center()
                            .child(item_title(
                                name,
                                Some((
                                    SharedString::from(format!("settings-info-{id}")),
                                    note_of(unavailable),
                                )),
                            ))
                            .child(
                                div()
                                    .id(SharedString::from(format!("settings-status-{id}")))
                                    .child(badge(badge_icon, status, badge_fg))
                                    .test_support(),
                            )
                            .when(unavailable, |row| {
                                let focus = self.key_retry_focus[index].clone();
                                row.child(
                                    Button::new(SharedString::from(format!("settings-retry-{id}")))
                                        .track_focus(&focus)
                                        .label(t("settings.key.unavailable_retry"))
                                        .accessibility_label(tf(
                                            "settings.key.unavailable_retry_a11y",
                                            &[("provider", *name)],
                                        ))
                                        .outline()
                                        .small()
                                        // §0: the old 28pt controls are 32 now.
                                        .h(px(tokens::component::button::HEIGHT))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.retry_key_status(index, window, cx)
                                        })),
                                )
                            }),
                    )
                    .child(
                        // The key field shrinks (flex-1 + min-w-0) so 保存 /
                        // 移除 always stay inside the card.
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .id(SharedString::from(format!("settings-input-{id}")))
                                    .flex_1()
                                    .min_w_0()
                                    // §0: 32pt control height (Input's default
                                    // medium already renders h-8).
                                    .h(px(tokens::component::button::HEIGHT))
                                    .child(Input::new(input))
                                    .test_support(),
                            )
                            .child(
                                Button::new(SharedString::from(format!("settings-save-{id}")))
                                    .flex_none()
                                    .label(t("settings.key.save"))
                                    .primary()
                                    .small()
                                    .h(px(tokens::component::button::HEIGHT))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.save_key(index, false, window, cx)
                                    })),
                            )
                            .when(configured, |row| {
                                row.child(
                                    Button::new(SharedString::from(format!(
                                        "settings-remove-{id}"
                                    )))
                                    .flex_none()
                                    .label(t("settings.key.remove"))
                                    .outline()
                                    .small()
                                    .h(px(tokens::component::button::HEIGHT))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.save_key(index, true, window, cx)
                                    })),
                                )
                            }),
                    )
                    .test_support()
                    .into_any_element(),
            );
        }
        let model = self
            .state
            .settings
            .settings
            .as_ref()
            .map(|s| s.model.model.clone())
            .unwrap_or_default();
        service_items.push(
            item(
                vec![
                    item_title(
                        t("settings.model.label"),
                        Some((
                            "settings-info-model".into(),
                            t("settings.model.note").into(),
                        )),
                    ),
                    badge(None, model, palette.fg2),
                ],
                service_items.is_empty(),
            )
            .into_any_element(),
        );

        // ---- 外观: 主题 segmented control (ui_prefs.appearance.theme).
        let appearance = self.state.ui.appearance;
        use tokens::component::segmented as segc;
        let seg = |id: &'static str, label: &'static str, value: forge_state::Appearance| {
            let on = appearance == value;
            Button::new(SharedString::from(format!("settings-theme-{id}")))
                .label(t(label))
                .ghost()
                .xsmall()
                .flex_1()
                // 32pt track, 2pt inset → 28pt segments nesting at
                // radius-sm − 2 (§0 concentric outline).
                .h(px(segc::SEGMENT_HEIGHT))
                .rounded(px(segc::segment_radius()))
                .when(on, |b| {
                    b.bg(rgb(palette.surface))
                        .text_color(rgb(palette.fg))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .when(shadows, |b| b.shadow(crate::style::shadow_sm(dark)))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.state.ui.appearance = value;
                    this.after_change(window, cx);
                }))
                .into_any_element()
        };
        let appearance_items = vec![
            item(
                vec![
                    item_title(t("settings.theme"), None),
                    h_flex()
                        .id("settings-theme-track")
                        .items_center()
                        .h(px(segc::TRACK_HEIGHT))
                        .p(px(segc::TRACK_INSET))
                        .rounded(px(segc::TRACK_RADIUS))
                        .bg(rgb(palette.subtle))
                        .test_support()
                        .child(seg(
                            "light",
                            "settings.theme.light",
                            forge_state::Appearance::Light,
                        ))
                        .child(seg(
                            "dark",
                            "settings.theme.dark",
                            forge_state::Appearance::Dark,
                        ))
                        .child(seg(
                            "system",
                            "settings.theme.system",
                            forge_state::Appearance::System,
                        ))
                        .into_any_element(),
                ],
                true,
            )
            .into_any_element(),
        ];

        // ---- 关于: Forge + version badge (read-only; no update checker).
        let about_items = vec![
            item(
                vec![item_title("Forge", None), {
                    let v = env!("CARGO_PKG_VERSION").to_owned();
                    badge(
                        None,
                        tf("settings.about.version", &[("version", v.as_str())]),
                        palette.fg2,
                    )
                }],
                true,
            )
            .into_any_element(),
        ];

        let group = |title: &'static str, items: Vec<AnyElement>| {
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_size(px(tokens::CAPTION.size))
                        .line_height(px(tokens::CAPTION.line_height))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(rgb(palette.fg3))
                        .child(t(title)),
                )
                .child(
                    v_flex()
                        .rounded(px(tokens::component::card::RADIUS))
                        .bg(rgb(settings_bg(&palette, dark)))
                        .children(items),
                )
        };
        let error = self.state.settings.error.clone();
        Some(
            div()
                .id("settings-overlay")
                .key_context(SETTINGS_CONTEXT)
                .on_action(cx.listener(Self::on_close_settings))
                .on_action(cx.listener(Self::on_settings_escape))
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                // Scrim token (gpui-kit `overlay`): black 30 % light / 50 % dark.
                .bg(theme.overlay)
                // Modal scrim: blocks hover, clicks and the scroll wheel for
                // everything behind it. Clicking it does not close the panel
                // (only Esc or ✕ do).
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .child(
                    // Tab / Shift-Tab cycle within the panel only.
                    div()
                        .id("settings-trap")
                        .track_focus(&self.settings_focus)
                        .tab_stop(false)
                        .w(px(dg::WIDTH))
                        .max_w_full()
                        .min_w_0()
                        .child(
                            v_flex()
                                .id("settings")
                                .role(Role::Dialog)
                                .aria_label(t("settings.page"))
                                .w_full()
                                .min_w_0()
                                .p(px(dg::PADDING))
                                .gap(px(dg::GROUP_GAP))
                                .rounded(px(dg::RADIUS))
                                .bg(rgb(palette.surface))
                                .when_some(dg::border_color(&palette, dark, shadows), |el, c| {
                                    el.border_1().border_color(rgb(c))
                                })
                                .shadow(crate::style::shadow_lg(dark))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_size(px(tokens::TITLE2.size))
                                                .line_height(px(tokens::TITLE2.line_height))
                                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                                .child(t("settings.page")),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            Button::new("settings-close")
                                                .track_focus(&self.settings_close_focus)
                                                // Last tab stop: the cycle starts in the rows
                                                // (ⓘ notes sit at index 1).
                                                .tab_index(2)
                                                .icon(IconName::X)
                                                .ghost()
                                                .small()
                                                .h(px(tokens::component::button::HEIGHT))
                                                .accessibility_label(t("a11y.settings.close"))
                                                .tooltip(t("settings.close"))
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.close_settings(window, cx)
                                                })),
                                        ),
                                )
                                .child(group("settings.group.services", service_items))
                                .child(group("settings.group.appearance", appearance_items))
                                .child(group("settings.group.about", about_items))
                                .when_some(error, |col, error| {
                                    col.child(
                                        div()
                                            .id("settings-error")
                                            .role(Role::Alert)
                                            .text_sm()
                                            .text_color(rgb(palette.danger))
                                            .child(error),
                                    )
                                })
                                .test_support(),
                        )
                        .focus_trap("settings-trap", &self.settings_focus),
                )
                .test_support()
                .into_any_element(),
        )
    }
}

impl Focusable for ForgeApp {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ForgeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        let layout = self.state.ui.layout(width);
        let sidebar = match layout.sidebar {
            PanelLayout::Docked { width } | PanelLayout::Overlay { width } => {
                let vm = self.state.sidebar();
                Some((self.render_sidebar(vm, width, cx), self.render_divider(cx)))
            }
            PanelLayout::Hidden => None,
        };
        let inspector = match layout.inspector {
            PanelLayout::Docked { width } => Some(self.render_inspector(width, false, cx)),
            PanelLayout::Overlay { width } => Some(self.render_inspector(width, true, cx)),
            PanelLayout::Hidden => None,
        };
        if let Some(page) = self.render_storage_error(cx) {
            return div()
                .id("forge-root")
                .key_context(ROOT_CONTEXT)
                .track_focus(&self.focus)
                .on_action(cx.listener(Self::on_quit))
                .size_full()
                .child(page)
                .into_any_element();
        }
        let settings = self.render_settings(cx);
        let header = self.render_header(cx);
        // v2.1: on the empty screen the composer sits inside it between the
        // title and the suggestion rows; otherwise it docks at the bottom.
        let empty =
            self.state.onboarding.is_none() && self.state.selected().is_none_or(|vm| vm.is_empty());
        let composer = self.render_composer(window, cx);
        let content: AnyElement = if empty {
            self.render_empty(cx, composer)
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .child(self.render_timeline(cx))
                .child(composer)
                .into_any_element()
        };
        let theme = cx.theme().clone();
        let (sidebar, divider) = match sidebar {
            Some((s, d)) => (Some(s), Some(d)),
            None => (None, None),
        };
        h_flex()
            .id("forge-root")
            .font_features(crate::style::tabular_figures())
            .key_context(ROOT_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_inspector))
            .on_action(cx.listener(|this, _: &InspectorChanges, window, cx| {
                this.show_inspector_tab(0, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InspectorQueue, window, cx| {
                this.show_inspector_tab(1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InspectorOutput, window, cx| {
                this.show_inspector_tab(2, window, cx)
            }))
            .on_action(cx.listener(Self::on_new_session))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(Self::on_close_settings))
            .on_action(cx.listener(Self::on_settings_escape))
            .on_action(cx.listener(Self::on_focus_composer))
            .on_action(cx.listener(Self::on_stop))
            .on_action(cx.listener(Self::on_retry_last))
            .on_action(cx.listener(Self::on_prev))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_close))
            .on_action(cx.listener(Self::on_minimize))
            .on_mouse_move(cx.listener(Self::on_root_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    // Drag ended: persist the new width (ui_prefs).
                    if this.drag.take().is_some() {
                        this.after_change(window, cx);
                    }
                }),
            )
            .relative()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .children(sidebar)
            .children(divider)
            .child(
                v_flex()
                    .id("main")
                    .role(Role::Main)
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .children(header)
                    .child(content),
            )
            .children(inspector)
            .children(settings)
            .into_any_element()
    }
}
