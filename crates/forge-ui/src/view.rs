//! Root view. Rendering only: every decision comes from `forge_state::AppState`.

use crate::actions::{
    COMPOSER_CONTEXT, CloseWindow, FocusComposer, Minimize, NewSession, NextSession, PrevSession,
    Quit, ROOT_CONTEXT, Steer, StopRun, ToggleSidebar,
};
use crate::backend::{Backend, EventReceiver, StartError};
use forge_protocol::{EventEnvelope, RunOutcome, SessionId};
use forge_state::i18n::{t, tf};
use forge_state::{
    AppState, BootState, ComposerKey, ComposerMode, KeyOutcome, QuitDecision, SessionRowVm,
    SidebarGroup, SidebarVm, StatusIcon, TimelineItem,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, EntityInputHandler, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, PromptButton, PromptLevel, Render, Role,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder, px,
};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub struct ForgeApp {
    pub state: AppState,
    backend: Rc<dyn Backend>,
    composer: Entity<TextareaState>,
    focus: FocusHandle,
    started: Instant,
    exiting: bool,
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
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10)
                .placeholder(t("composer.placeholder.idle"))
                .submit_on_enter(true)
        });
        let subscriptions = vec![
            cx.subscribe_in(&composer, window, Self::on_composer_event),
            cx.observe_window_appearance(window, |_, window, cx| {
                gpui_kit::component::Theme::sync_system_appearance(Some(window), cx);
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
            composer,
            focus,
            started: Instant::now(),
            exiting: false,
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

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
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

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.set_now_ms(self.now_ms());
        self.state.poll_quit_timeout();
        self.after_change(window, cx);
    }

    /// Ships queued commands, mirrors the draft into the text area, exits if
    /// the quit flow finished, and re-renders.
    fn after_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        // VoiceOver announcements need a platform bridge gpui does not expose
        // yet; drain so they do not accumulate (see report: macOS manual check).
        self.state.take_announcements();
        self.sync_composer(window, cx);
        if self.state.should_exit() && !self.exiting {
            self.exiting = true;
            cx.quit();
        }
        cx.notify();
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
        self.state.stop_run();
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

    // --------------------------------------------------------------- render

    fn render_sidebar(&self, sidebar: SidebarVm, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let mut list = v_flex()
            .id("sidebar")
            .role(Role::Navigation)
            .aria_label(t("a11y.sidebar"))
            .w(px(260.))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .p_2()
            .gap_3()
            .child(
                Button::new("new-session")
                    .label(t("sidebar.new_session"))
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

    fn render_row(&self, row: SessionRowVm, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let icon: Option<AnyElement> = match row.icon {
            StatusIcon::None => None,
            StatusIcon::Spinner => Some(Spinner::new().xsmall().into_any_element()),
            StatusIcon::Approval => Some(
                Icon::new(IconName::TriangleAlert)
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
                Icon::new(IconName::CircleAlert)
                    .text_color(theme.danger)
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

    fn render_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let mut column = v_flex()
            .id("timeline")
            .role(Role::Log)
            .aria_label(t("a11y.timeline"))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_6()
            .py_4()
            .gap_3();
        let Some(vm) = self.state.selected() else {
            return column
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_lg()
                        .text_color(theme.muted_foreground)
                        .child(t("empty.title")),
                )
                .into_any_element();
        };
        if vm.is_empty() {
            column = column.items_center().justify_center().child(
                div()
                    .text_lg()
                    .text_color(theme.muted_foreground)
                    .child(t("empty.title")),
            );
        }
        for (index, item) in vm.timeline.iter().enumerate() {
            let id = SharedString::from(format!("timeline-{index}"));
            let element = match item {
                TimelineItem::User(message) => h_flex()
                    .id(id)
                    .aria_label(tf("a11y.timeline.user", &[("text", &message.text)]))
                    .justify_end()
                    .child(
                        div()
                            .max_w(px(560.))
                            .px_3()
                            .py_2()
                            .rounded(px(12.))
                            .bg(theme.secondary)
                            .child(message.text.clone()),
                    )
                    .into_any_element(),
                TimelineItem::Assistant(message) => {
                    let mut text = message.text.clone();
                    if message.text.is_empty() && message.streaming {
                        text = t("assistant.thinking").to_owned();
                    }
                    if message.stopped {
                        text.push_str(t("assistant.stopped_suffix"));
                    }
                    div()
                        .id(id)
                        .aria_label(tf("a11y.timeline.assistant", &[("text", &text)]))
                        .child(text)
                        .into_any_element()
                }
                TimelineItem::RunDivider { outcome, .. } => {
                    let key = match outcome {
                        RunOutcome::Succeeded => "run.succeeded",
                        RunOutcome::Failed => "run.failed",
                        RunOutcome::Cancelled => "run.cancelled",
                        RunOutcome::Interrupted => "run.interrupted",
                    };
                    div()
                        .id(id)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t(key))
                        .into_any_element()
                }
                TimelineItem::Error(error) => v_flex()
                    .id(id)
                    .role(Role::Alert)
                    .p_3()
                    .rounded(px(8.))
                    .bg(theme.danger)
                    .text_color(theme.danger_foreground)
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(t(error.kind.title_key())),
                    )
                    .into_any_element(),
                TimelineItem::Notice(notice) => div()
                    .id(id)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(notice.text.clone())
                    .into_any_element(),
            };
            column = column.child(element);
        }
        column.test_support().into_any_element()
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

        v_flex()
            .id("composer")
            .key_context(COMPOSER_CONTEXT)
            .on_action(cx.listener(Self::on_steer))
            .flex_shrink_0()
            .px_6()
            .pb_4()
            .pt_2()
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
            )
            .into_any_element()
    }

    fn render_storage_error(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let BootState::StorageError { path, .. } = &self.state.boot else {
            return None;
        };
        let theme = cx.theme().clone();
        Some(
            v_flex()
                .id("storage-error")
                .role(Role::Alert)
                .m_4()
                .p_4()
                .gap_1()
                .rounded(px(8.))
                .bg(theme.danger)
                .text_color(theme.danger_foreground)
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = self.state.sidebar();
        let visible = sidebar.visible;
        let sidebar = visible.then(|| self.render_sidebar(sidebar, cx));
        let storage = self.render_storage_error(cx);
        let timeline = self.render_timeline(cx);
        let composer = self.render_composer(cx);
        let theme = cx.theme().clone();
        h_flex()
            .id("forge-root")
            .key_context(ROOT_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_new_session))
            .on_action(cx.listener(Self::on_focus_composer))
            .on_action(cx.listener(Self::on_stop))
            .on_action(cx.listener(Self::on_prev))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_close))
            .on_action(cx.listener(Self::on_minimize))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .children(sidebar)
            .child(
                v_flex()
                    .id("main")
                    .role(Role::Main)
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .children(storage)
                    .child(timeline)
                    .child(composer),
            )
    }
}
