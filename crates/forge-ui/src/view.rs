//! Root view. Rendering only: every decision comes from `forge_state::AppState`.

use crate::actions::{
    COMPOSER_CONTEXT, CloseWindow, FocusComposer, Minimize, NewSession, NextSession, PrevSession,
    Quit, ROOT_CONTEXT, Steer, StopRun, ToggleInspector, ToggleSidebar,
};
use crate::announce::Announcer;
use crate::backend::{Backend, EventReceiver, StartError};
use crate::timeline_view::render_row;
use crate::tokens::Palette;
use forge_protocol::{EventEnvelope, RunId, SessionId};
use forge_state::i18n::{t, tf};
use forge_state::{
    AppState, BootState, ComposerKey, ComposerMode, ErrorAction, ErrorEffect, KeyOutcome,
    OnboardingStep, PanelLayout, QuitDecision, SessionRowVm, SidebarGroup, SidebarResize,
    SidebarVm, StatusIcon, TimelineRowVm,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext, Context, CursorStyle, Entity, EntityInputHandler, FocusHandle,
    Focusable, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement, PromptButton, PromptLevel, Render, Role, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
    px, rgb,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

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
        let subscriptions = vec![
            cx.subscribe_in(&composer, window, Self::on_composer_event),
            cx.observe_window_appearance(window, |_, window, cx| {
                crate::theme::sync_appearance(Some(window), cx);
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

        let focus = cx.focus_handle();
        let app = Self {
            state,
            backend,
            announcer,
            composer,
            scroller,
            rows: Rc::new(RefCell::new(Vec::new())),
            rows_session: None,
            focus,
            started: Instant::now(),
            clock_base_ms: 0,
            exiting: false,
            drag: None,
            _tasks: tasks,
            _subscriptions: subscriptions,
        };
        app.composer.read(cx).focus_handle(cx).focus(window, cx);
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
        if self.state.should_exit() && !self.exiting {
            self.exiting = true;
            cx.quit();
        }
        cx.notify();
    }

    fn sync_timeline(&mut self, cx: &mut Context<Self>) {
        let rows = self.state.timeline_rows();
        let session = self.state.selected_session.clone();
        let old_len = self.rows.borrow().len();
        let changed = *self.rows.borrow() != rows;
        let new_len = rows.len();
        let same_session = session == self.rows_session;
        *self.rows.borrow_mut() = rows;
        self.rows_session = session;
        self.scroller.update(cx, |scroller, cx| {
            if !same_session || new_len < old_len {
                scroller.reset(new_len, cx);
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
            // The settings window lands with §6.13 UI; nothing to open yet.
            ErrorEffect::OpenSettings => {}
        }
        self.after_change(window, cx);
    }

    pub fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.stop_run();
        self.after_change(window, cx);
    }

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
        let mut list = v_flex()
            .id("sidebar")
            .role(Role::Navigation)
            .aria_label(t("a11y.sidebar"))
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .p_2()
            .gap_3()
            .child(
                Button::new("new-session")
                    .label(t("sidebar.new_session"))
                    .icon(IconName::Plus)
                    .ghost()
                    .small()
                    .tooltip(t("menu.file.new_session"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_new_session(&NewSession, window, cx)
                    })),
            );
        if sidebar.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t("sidebar.empty")),
            );
        }
        for (group, rows) in [
            (SidebarGroup::NeedsAttention, sidebar.needs_attention),
            (SidebarGroup::InProgress, sidebar.in_progress),
            (SidebarGroup::Recent, sidebar.recent),
            (SidebarGroup::Archived, sidebar.archived),
        ] {
            if rows.is_empty() {
                continue;
            }
            let label = SidebarVm::group_label(group, rows.len());
            let mut section = v_flex()
                .id(SharedString::from(format!("group-{group:?}")))
                .role(Role::List)
                .aria_label(label)
                .gap_0p5()
                .child(
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t(group.label_key())),
                );
            for row in rows {
                section = section.child(self.render_row(row, cx));
            }
            list = list.child(section);
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
        let icon: Option<AnyElement> = match row.icon {
            StatusIcon::None => None,
            StatusIcon::Spinner => Some(Spinner::new().xsmall().into_any_element()),
            StatusIcon::Approval => Some(
                Icon::new(IconName::Shield)
                    .text_color(theme.warning)
                    .into_any_element(),
            ),
            StatusIcon::Failed => Some(
                Icon::new(IconName::CircleX)
                    .text_color(theme.danger)
                    .into_any_element(),
            ),
            StatusIcon::Stopped => Some(
                Icon::new(IconName::Square)
                    .text_color(theme.muted_foreground)
                    .into_any_element(),
            ),
            StatusIcon::Interrupted => Some(
                Icon::new(IconName::TriangleAlert)
                    .text_color(theme.warning)
                    .into_any_element(),
            ),
            StatusIcon::Uncertain => Some(
                Icon::new(IconName::CircleAlert)
                    .text_color(theme.warning)
                    .into_any_element(),
            ),
        };
        let id = row.id.clone();
        h_flex()
            .id(SharedString::from(format!("session-{}", row.id.0)))
            .role(Role::ListItem)
            .aria_label(row.a11y_label.clone())
            .aria_selected(row.selected)
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(8.))
            .cursor_pointer()
            .when(row.selected, |row| {
                row.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .when(!row.selected, |row| row.hover(|s| s.bg(theme.list_hover)))
            .children(icon)
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .child(div().text_sm().truncate().child(row.title.clone()))
                    .when(!row.status_text.is_empty(), |col| {
                        col.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .truncate()
                                .child(row.status_text.clone()),
                        )
                    }),
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
        Some(
            h_flex()
                .id("session-header")
                .h(px(52.))
                .flex_shrink_0()
                .px_6()
                .gap_3()
                .items_center()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .truncate()
                        .child(vm.session.title.clone()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(vm.session.model.model.clone()),
                )
                .child(div().flex_1())
                .children(
                    context.map(|c| div().text_sm().text_color(theme.muted_foreground).child(c)),
                )
                .into_any_element(),
        )
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let no_key = !self.state.settings.has_any_key();
        let has_session = self.state.selected().is_some();
        let mut col = v_flex()
            .id("timeline-empty")
            .flex_1()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                div()
                    .text_size(px(22.))
                    .line_height(px(28.))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(t("empty.title")),
            );
        if has_session && !no_key {
            col =
                col.child(h_flex().gap_2().children((1..=3).map(|i| {
                    Button::new(SharedString::from(format!("example-{i}")))
                        .label(t(&format!("empty.example.{i}")))
                        .outline()
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.fill_example(i - 1, window, cx)
                        }))
                })));
        }
        if no_key {
            let palette = *Palette::for_dark(theme.is_dark());
            col = col.child(
                h_flex()
                    .id("no-key")
                    .role(Role::Group)
                    .gap_3()
                    .px_4()
                    .py_3()
                    .rounded(px(8.))
                    .bg(rgb(palette.info_bg))
                    .text_color(rgb(palette.fg))
                    .child(Icon::new(IconName::Info).text_color(rgb(palette.info)))
                    .child(t("composer.no_key"))
                    .child(
                        Button::new("no-key-settings")
                            .label(t("composer.open_settings"))
                            .small()
                            .outline(),
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
                                .text_size(px(22.))
                                .line_height(px(28.))
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
                                .child(
                                    Button::new("onboarding-next")
                                        .label(next_label)
                                        .primary()
                                        .disabled(!self.state.onboarding_can_advance())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.state.onboarding_next();
                                            this.after_change(window, cx);
                                        })),
                                ),
                        ),
                )
                .test_support()
                .into_any_element(),
        )
    }

    fn render_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(onboarding) = self.render_onboarding(cx) {
            return onboarding;
        }
        let empty = self.state.selected().is_none_or(|vm| vm.is_empty());
        if empty {
            return self.render_empty(cx);
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
                            Some(row) => div()
                                .pt_4()
                                .child(render_row(&row, &weak, cx))
                                .into_any_element(),
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

    fn render_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let mode = self.state.composer_mode();
        let disabled = matches!(mode, ComposerMode::Disabled(_));
        let sending = mode == ComposerMode::Sending;
        let busy = matches!(mode, ComposerMode::Running | ComposerMode::Cancelling);
        let error = self
            .state
            .selected()
            .and_then(|vm| vm.composer.error.clone());
        let button = if busy {
            Button::new("composer-stop")
                .label(if mode == ComposerMode::Cancelling {
                    t("composer.stopping")
                } else {
                    t("composer.stop")
                })
                .danger()
                .disabled(mode == ComposerMode::Cancelling)
                .tooltip("⌘.")
        } else {
            Button::new("composer-send")
                .label(t("composer.send"))
                .primary()
                .disabled(!self.state.can_send())
                .tooltip("↩")
        }
        .on_click(cx.listener(|this, _, window, cx| this.primary_button(window, cx)));

        h_flex()
            .w_full()
            .justify_center()
            .flex_shrink_0()
            .px_6()
            .pb_4()
            .pt_2()
            .child(
                v_flex()
                    .id("composer")
                    .key_context(COMPOSER_CONTEXT)
                    .on_action(cx.listener(Self::on_steer))
                    .w_full()
                    .max_w(px(forge_state::CONTENT_MAX))
                    .gap_1()
                    .child(
                        h_flex()
                            .items_end()
                            .gap_2()
                            .child(
                                div().flex_1().child(
                                    Textarea::new(&self.composer)
                                        .aria_label(t("a11y.composer"))
                                        .disabled(disabled)
                                        .readonly(sending),
                                ),
                            )
                            .child(button),
                    )
                    .when_some(error, |col, error| {
                        col.child(
                            div()
                                .id("composer-error")
                                .role(Role::Alert)
                                .text_xs()
                                .text_color(theme.danger)
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t(self.state.composer_hint_key())),
                    ),
            )
            .into_any_element()
    }

    fn render_inspector(&self, width: f32, overlay: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
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
                    .px_4()
                    .gap_4()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_sm()
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(t("inspector.tab.changes")),
                    )
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(t("inspector.tab.queue")),
                    )
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(t("inspector.tab.output")),
                    ),
            )
            .child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t("inspector.empty")),
            )
            .test_support()
            .into_any_element()
    }

    fn render_storage_error(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let BootState::StorageError { path, .. } = &self.state.boot else {
            return None;
        };
        let theme = cx.theme().clone();
        let palette = *Palette::for_dark(theme.is_dark());
        Some(
            v_flex()
                .id("storage-error")
                .role(Role::Alert)
                .m_4()
                .p_4()
                .gap_1()
                .rounded(px(8.))
                .bg(rgb(palette.danger_bg))
                .text_color(rgb(palette.fg))
                .child(t("storage.title"))
                .child(div().text_xs().child(path.display().to_string()))
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
        let storage = self.render_storage_error(cx);
        let header = self.render_header(cx);
        let timeline = self.render_timeline(cx);
        let composer = self.render_composer(cx);
        let theme = cx.theme().clone();
        let (sidebar, divider) = match sidebar {
            Some((s, d)) => (Some(s), Some(d)),
            None => (None, None),
        };
        h_flex()
            .id("forge-root")
            .key_context(ROOT_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_inspector))
            .on_action(cx.listener(Self::on_new_session))
            .on_action(cx.listener(Self::on_focus_composer))
            .on_action(cx.listener(Self::on_stop))
            .on_action(cx.listener(Self::on_prev))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_close))
            .on_action(cx.listener(Self::on_minimize))
            .on_mouse_move(cx.listener(Self::on_root_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag = None),
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
                    .children(storage)
                    .children(header)
                    .child(timeline)
                    .child(composer),
            )
            .children(inspector)
    }
}
