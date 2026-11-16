//! P1 headless UI tests: timeline rows, error card + countdown, reasoning
//! disclosure, sidebar resize, Inspector toggle, announcement routing.
use forge_protocol::*;
use forge_state::AnnouncementPriority;
use forge_ui::ForgeApp;
use forge_ui::announce::RecordingAnnouncer;
use forge_ui::backend::{Backend, CommandFuture};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Entity, Focusable, Point, ScrollDelta, TestAppContext, WindowBounds,
    WindowHandle, WindowOptions, base::Root, point, px, size,
};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Recording {
    commands: Arc<Mutex<Vec<Command>>>,
}

impl Backend for Recording {
    fn execute(&self, command: Command) -> CommandFuture {
        self.commands.lock().unwrap().push(command);
        Box::pin(async { Some(CommandResult::Accepted { id: "cmd".into() }) })
    }
}

impl Recording {
    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut self.commands.lock().unwrap())
    }
}

fn model() -> ModelSelection {
    ModelSelection {
        provider: "openai".into(),
        model: "gpt-5".into(),
    }
}

fn run(id: &str) -> Run {
    Run {
        id: id.into(),
        session_id: "s1".into(),
        queue_item_id: None,
        state: RunState::Running,
        outcome: None,
        started_at: 0,
        finished_at: None,
        usage: Usage {
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            requests: 0,
            estimated_cost_usd: None,
        },
    }
}

struct Fixture {
    handle: WindowHandle<Root>,
    app: Entity<ForgeApp>,
    backend: Recording,
    spoken: Rc<RecordingAnnouncer>,
}

impl Fixture {
    fn new(cx: &mut TestAppContext, width: f32) -> Self {
        let backend = Recording::default();
        let spoken = Rc::new(RecordingAnnouncer::default());
        let (shared, announcer) = (backend.clone(), spoken.clone());
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            forge_ui::init(cx);
            let (window, app) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(width), px(800.)),
                    })),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    cx.new(|cx| {
                        ForgeApp::with_announcer(Rc::new(shared), announcer, None, None, window, cx)
                    })
                },
            )
            .expect("open window");
            (window.downcast::<Root>().unwrap(), app)
        });
        let fixture = Self {
            handle,
            app,
            backend,
            spoken,
        };
        fixture.update(cx, |app, window, cx| {
            app.state.load_settings(Settings {
                providers: vec![ProviderSettings {
                    id: "openai".into(),
                    endpoint: None,
                    keychain_reference: None,
                    max_output_tokens: None,
                }],
                model: model(),
                permission_mode: PermissionMode::Ask,
                ui_prefs: Default::default(),
            });
            let workspace = Workspace {
                id: "ws".into(),
                root: "/tmp/ws".into(),
                name: "ws".into(),
            };
            let key = Event::ProviderKeyConfigured {
                provider: "openai".into(),
                key_status: KeyStatus::Configured,
            };
            fixture_event(app, key, window, cx);
            fixture_event(app, Event::WorkspaceAdded { workspace }, window, cx);
            let session = Session {
                id: "s1".into(),
                workspace_id: "ws".into(),
                title: "会话".into(),
                model: model(),
                permission_mode: PermissionMode::Ask,
                state: SessionState::Idle,
                archived: false,
                queue_state: QueueState::Active,
                created_at: 0,
                updated_at: 0,
                last_finished_run: None,
            };
            fixture_event(app, Event::SessionCreated { session }, window, cx);
            app.state.select_session(&"s1".into());
            app.after_change(window, cx);
        });
        cx.run_until_parked();
        fixture.backend.take();
        fixture
    }

    fn update<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut ForgeApp, &mut gpui_kit::Window, &mut gpui_kit::Context<ForgeApp>) -> R,
    ) -> R {
        let app = self.app.clone();
        cx.update_window(self.handle.into(), |_, window, cx| {
            app.update(cx, |app, cx| f(app, window, cx))
        })
        .unwrap()
    }

    fn with_window<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
    ) -> R {
        cx.update_window(self.handle.into(), |_, window, cx| f(window, cx))
            .unwrap()
    }

    fn composer_id(&self, cx: &mut TestAppContext) -> gpui_kit::ElementId {
        let composer = cx.update(|cx| self.app.read(cx).composer().clone());
        ("input", composer.entity_id()).into()
    }

    fn events(&self, cx: &mut TestAppContext, events: Vec<Event>) {
        self.update(cx, |app, window, cx| {
            for mut event in events {
                // Simulate Core: live RunStarted / RunFinished always carry
                // real clocks. `0` means "fixture left it unset".
                if let Event::RunStarted { run, .. } = &mut event
                    && run.started_at == 0
                {
                    run.started_at = app.now_ms();
                }
                if let Event::RunFinished { finished_at, .. } = &mut event
                    && *finished_at == 0
                {
                    *finished_at = app.now_ms();
                }
                fixture_event(app, event, window, cx);
            }
        });
        cx.run_until_parked();
    }

    /// Sends "hi" through the composer state and starts run `id` (Core
    /// echoes the send's `input_id` in `RunStarted`).
    fn start_run(&self, cx: &mut TestAppContext, id: &str) {
        self.update(cx, |app, window, cx| {
            app.state.set_draft("hi");
            app.state.composer_key(forge_state::ComposerKey::Enter);
            app.after_change(window, cx);
        });
        cx.run_until_parked();
        let input_id = self
            .backend
            .take()
            .into_iter()
            .find_map(|c| match c {
                Command::Send { input_id, .. } => Some(input_id),
                _ => None,
            })
            .unwrap_or_else(|| "in-x".into());
        self.events(
            cx,
            vec![Event::RunStarted {
                run: run(id),
                input_id,
            }],
        );
    }

    /// `"kind[ http_status=..][ retry_after_ms=..][: message]"` → P1 outcome.
    fn fail(&self, cx: &mut TestAppContext, id: &str, spec: &str) {
        let (head, message) = spec.split_once(": ").unwrap_or((spec, ""));
        let mut parts = head.split_whitespace();
        let kind = match parts.next().unwrap_or("") {
            "network" => RunErrorKind::Network,
            "rate_limited" => RunErrorKind::RateLimited,
            "auth" => RunErrorKind::Auth,
            "quota" => RunErrorKind::Quota,
            "server" => RunErrorKind::Server,
            "context_too_long" => RunErrorKind::ContextTooLong,
            "interrupted" => RunErrorKind::Interrupted,
            "storage" => RunErrorKind::Unknown {
                source: UnknownSource::Storage,
            },
            _ => RunErrorKind::Unknown {
                source: UnknownSource::Provider,
            },
        };
        let (mut http_status, mut retry_after_ms) = (None, None);
        for part in parts {
            match part.split_once('=') {
                Some(("http_status", v)) => http_status = v.parse().ok(),
                Some(("retry_after_ms", v)) => retry_after_ms = v.parse().ok(),
                _ => {}
            }
        }
        let retryable = !matches!(kind, RunErrorKind::Auth | RunErrorKind::Quota);
        self.events(
            cx,
            vec![Event::RunFinished {
                run: id.into(),
                outcome: RunOutcome::Failed {
                    kind,
                    message: message.into(),
                    http_status,
                    retry_after_ms,
                    retryable,
                },
                finished_at: 0,
            }],
        );
    }

    fn retries(&self) -> usize {
        self.backend
            .take()
            .into_iter()
            .filter(|c| matches!(c, Command::RetryRun { .. }))
            .count()
    }
}

fn fixture_event(
    app: &mut ForgeApp,
    event: Event,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<ForgeApp>,
) {
    let seq = app.state.last_seq + 1;
    app.apply_event(EventEnvelope { seq, event }, window, cx);
}

#[gpui_kit::test]
fn default_layout_is_240_sidebar_and_hidden_inspector(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        let sidebar = window.find("sidebar");
        assert_eq!(sidebar.bounds().size.width, px(240.));
        assert!(window.try_find("inspector").is_none());
        window.press("alt-cmd-i", cx);
        let inspector = window.find("inspector");
        assert_eq!(inspector.bounds().size.width, px(360.));
        assert_eq!(inspector.bounds().origin.x, px(1280. - 360.));
        window.press("alt-cmd-i", cx);
        assert!(window.try_find("inspector").is_none());
    });
}

#[gpui_kit::test]
fn sidebar_drag_clamps_collapses_and_double_click_resets(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    let divider_x = |f: &Fixture, cx: &mut TestAppContext| {
        f.with_window(cx, |window, cx| {
            window.render_frame(cx);
            let b = window.find("sidebar-divider").bounds();
            point(b.origin.x + b.size.width / 2., b.origin.y + px(300.))
        })
    };
    let from = divider_x(&f, cx);
    f.with_window(cx, |window, cx| {
        window.drag(from, point(from.x + px(60.), from.y), cx)
    });
    f.update(cx, |app, _, _| assert_eq!(app.state.ui.sidebar_width, 300.));
    let from = divider_x(&f, cx);
    f.with_window(cx, |window, cx| {
        window.drag(from, point(from.x + px(400.), from.y), cx)
    });
    f.update(cx, |app, _, _| assert_eq!(app.state.ui.sidebar_width, 360.));
    f.with_window(cx, |window, cx| window.double_click("sidebar-divider", cx));
    f.update(cx, |app, _, _| assert_eq!(app.state.ui.sidebar_width, 240.));
    let from = divider_x(&f, cx);
    f.with_window(cx, |window, cx| {
        window.drag(from, point(from.x - px(120.), from.y), cx)
    });
    f.update(cx, |app, _, _| {
        assert!(!app.state.ui.sidebar_visible, "below 160 collapses")
    });
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sidebar").is_none());
    });
}

#[gpui_kit::test]
fn narrow_window_collapses_panels(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 700.);
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sidebar").is_none(), "700 < 240 + 480");
    });
}

#[gpui_kit::test]
fn timeline_renders_rows_and_reasoning_toggles(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.start_run(cx, "r1");
    f.events(
        cx,
        vec![
            Event::ReasoningDelta {
                run: "r1".into(),
                text: "思考中".into(),
            },
            Event::TextDelta {
                run: "r1".into(),
                text: "**答案**".into(),
            },
            Event::RunFinished {
                run: "r1".into(),
                outcome: RunOutcome::Succeeded,
                finished_at: 1,
            },
        ],
    );
    let keys: Vec<String> = f.update(cx, |app, _, _| {
        app.rows().iter().map(|r| r.key().to_owned()).collect()
    });
    assert_eq!(keys.len(), 3, "{keys:?}");
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("timeline").is_some());
        assert!(window.try_find(keys[0].clone()).is_some(), "user row");
        assert!(window.try_find("assistant-r1").is_some());
        assert!(window.try_find(keys[2].clone()).is_some(), "divider");
        let toggle = window.find("reasoning-r1");
        assert_eq!(toggle.expanded(), Some(false));
        window.click("reasoning-r1", cx);
        window.render_frame(cx);
        assert_eq!(window.find("reasoning-r1").expanded(), Some(true));
    });
}

#[gpui_kit::test]
fn error_card_countdown_disables_retry_until_ready(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.update(cx, |app, _, _| app.set_clock_base(10_000));
    f.start_run(cx, "r1");
    f.fail(
        cx,
        "r1",
        "rate_limited http_status=429 retry_after_ms=90000: slow",
    );
    f.backend.take();
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        let card = window.find("error-r1");
        assert_eq!(card.label(), Some("运行失败：服务要求等待"));
        let retry = window.find("error-r1-primary");
        assert_eq!(retry.disabled(), Some(true));
        assert!(
            retry.label().unwrap_or_default().starts_with("重试（"),
            "{:?}",
            retry.label()
        );
        window.click("error-r1-primary", cx);
        window.press("cmd-r", cx);
    });
    cx.run_until_parked();
    assert_eq!(f.retries(), 0, "disabled while waiting (click and ⌘R)");

    // Details toggle.
    f.with_window(cx, |window, cx| {
        assert!(window.try_find("error-details-r1").is_none());
        window.click("error-r1-secondary", cx);
        window.render_frame(cx);
        assert!(window.try_find("error-details-r1").is_some());
    });

    // Jump the clock past Retry-After: one click → one RetryRun, second click
    // while in flight does nothing new.
    f.update(cx, |app, window, cx| {
        let now = app.now_ms();
        app.set_clock_base(now + 91_000);
        app.tick(window, cx);
    });
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        let retry = window.find("error-r1-primary");
        assert_ne!(retry.disabled(), Some(true));
        assert_eq!(retry.label(), Some("重试"));
        window.click("error-r1-primary", cx);
    });
    cx.run_until_parked();
    assert_eq!(f.retries(), 1);
}

#[gpui_kit::test]
fn auth_card_opens_settings_and_has_no_countdown(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.start_run(cx, "r1");
    f.fail(
        cx,
        "r1",
        "auth http_status=401: Bearer sk-secret-abcdefghijklmnop",
    );
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("error-r1-primary").label(), Some("打开设置"));
        assert_eq!(window.find("error-r1-secondary").label(), Some("重试"));
        // UI-REF-01: shared card skeleton — status pill, ⋯ menu, labelled group.
        assert!(window.try_find("error-r1-status").is_some());
        assert_eq!(window.find("error-r1-more").label(), Some("更多操作"));
        let card = window.find("error-r1");
        assert_eq!(card.label(), Some("运行失败：API Key 无效或没有权限"));
    });
}

#[gpui_kit::test]
fn announcements_reach_the_platform_bridge(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.spoken.spoken.borrow_mut().clear();
    f.start_run(cx, "r1");
    f.fail(cx, "r1", "server");
    let spoken = f.spoken.spoken.borrow().clone();
    assert!(
        spoken
            .iter()
            .any(|(text, p)| text == "运行失败：模型服务出错" && *p == AnnouncementPriority::High),
        "{spoken:?}"
    );
    // Drained: nothing re-spoken on the next tick.
    let before = spoken.len();
    f.update(cx, |app, window, cx| app.tick(window, cx));
    assert_eq!(f.spoken.spoken.borrow().len(), before);
}

#[gpui_kit::test]
fn short_rate_limit_retries_at_once_with_cmd_r(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.start_run(cx, "r1");
    f.fail(
        cx,
        "r1",
        "rate_limited http_status=429 retry_after_ms=20000: slow",
    );
    f.backend.take();
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("error-r1").label(),
            Some("运行失败：请求太频繁")
        );
        let retry = window.find("error-r1-primary");
        assert_ne!(retry.disabled(), Some(true), "Retry-After <= 60 s: enabled");
        assert_eq!(retry.label(), Some("重试"));
        window.press("cmd-r", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        f.retries(),
        1,
        "⌘R sends RetryRun for the latest failed run"
    );
}

#[gpui_kit::test]
fn composer_toolbar_and_stop_button(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("composer-box").is_some());
        assert!(window.try_find("composer-toolbar").is_some());
        assert!(window.try_find("composer-attach").is_some());
        let send = window.find("composer-send");
        assert_eq!(send.label(), Some("发送"));
        assert_eq!(send.disabled(), Some(true), "empty draft");
    });
    f.start_run(cx, "r1");
    f.backend.take();
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("composer-send").is_none());
        assert_eq!(window.find("composer-stop").label(), Some("停止"));
        window.click("composer-stop", cx);
    });
    cx.run_until_parked();
    let cancels = f
        .backend
        .take()
        .into_iter()
        .filter(|c| matches!(c, Command::CancelRun { .. }))
        .count();
    assert_eq!(cancels, 1);
}

#[gpui_kit::test]
fn no_key_shows_banner_and_settings_link(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.update(cx, |app, window, cx| {
        let key = Event::ProviderKeyConfigured {
            provider: "openai".into(),
            key_status: KeyStatus::Missing,
        };
        fixture_event(app, key, window, cx);
    });
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("no-key").is_some(),
            "还没有添加 API Key banner"
        );
        assert!(window.try_find("composer-open-settings").is_some());
        assert_eq!(window.find("composer-send").disabled(), Some(true));
    });
}

#[gpui_kit::test]
fn sidebar_rows_are_48pt_and_groups_show_counts(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        let row = window.find("session-s1");
        assert_eq!(row.bounds().size.height, px(48.));
        let group = ["Today", "Yesterday", "Week", "Older", "InProgress"]
            .into_iter()
            .find_map(|k| window.try_find(format!("group-{k}")))
            .expect("s1 sits in a group");
        let label = group.label().unwrap().to_string();
        assert!(label.ends_with("个会话"), "{label}");
        let n: usize = label
            .rsplit('，')
            .next()
            .unwrap()
            .trim_end_matches(" 个会话")
            .parse()
            .unwrap();
        assert!(n >= 1);
        // ⌘N hint sits inside the left-aligned 新会话 button.
        let button = window.find("new-session");
        assert_eq!(button.label(), Some("新会话"));
    });
}

#[gpui_kit::test]
fn inspector_tabs_switch_with_alt_cmd_digits(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("inspector").is_none());
        window.press("alt-cmd-1", cx);
        assert!(window.try_find("inspector").is_some(), "⌥⌘1 opens it");
        assert_eq!(
            window.find("inspector-empty").label(),
            Some("本会话还没有改动文件")
        );
        window.press("alt-cmd-3", cx);
        assert_eq!(
            window.find("inspector-empty").label(),
            Some("选中一个工具步骤查看完整输出")
        );
        window.press("alt-cmd-2", cx);
        assert_eq!(
            window.find("inspector-empty").label(),
            Some("没有排队的消息")
        );
    });
}

#[gpui_kit::test]
fn open_settings_from_error_card_and_save_key(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.start_run(cx, "r1");
    f.fail(
        cx,
        "r1",
        "auth http_status=401: invalid api key: please check settings",
    );
    f.backend.take();
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings").is_none());
        window.click("error-r1-primary", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("settings").is_some(),
            "打开设置 opens the panel"
        );
        assert!(window.try_find("settings-key-openai").is_some());
        assert!(window.try_find("settings-key-anthropic").is_some());
    });
    // Typing a key and pressing 保存 sends SetProviderKey; the field clears.
    f.update(cx, |app, window, cx| {
        app.state
            .set_provider_key("openai".into(), Some("sk-new-key".into()));
        app.after_change(window, cx);
    });
    cx.run_until_parked();
    assert!(f.backend.take().iter().any(|c| matches!(
        c,
        Command::SetProviderKey { provider, key: Some(k) } if provider.0 == "openai" && k == "sk-new-key"
    )));
    f.with_window(cx, |window, cx| {
        window.click("settings-close", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings").is_none());
        window.press("cmd-,", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings").is_some(), "⌘, opens settings");
    });
}

#[gpui_kit::test]
fn storage_error_replaces_the_window(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.update(cx, |app, window, cx| {
        app.state.boot = forge_state::BootState::StorageError {
            code: "storage_unavailable".into(),
            path: "/data/forge-app".into(),
        };
        app.after_change(window, cx);
    });
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("storage-error").is_some());
        assert!(window.try_find("sidebar").is_none(), "§6.12: whole window");
        assert!(window.try_find("composer").is_none());
        assert!(window.try_find("storage-reveal").is_some());
        assert!(window.try_find("storage-logs").is_some());
        assert!(window.try_find("storage-quit").is_some());
        let detail = window.find("storage-error-detail");
        let text = format!("{:?}", detail.label());
        let _ = text;
    });
}

#[gpui_kit::test]
fn wait_card_body_switches_when_countdown_ends(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.update(cx, |app, _, _| app.set_clock_base(10_000));
    f.start_run(cx, "r1");
    f.fail(
        cx,
        "r1",
        "rate_limited http_status=429 retry_after_ms=90000: slow",
    );
    f.update(cx, |app, window, cx| {
        let now = app.now_ms();
        app.set_clock_base(now + 91_000);
        app.tick(window, cx);
        let card = app
            .rows()
            .into_iter()
            .find_map(|r| match r {
                forge_state::TimelineRowVm::Error(c) => Some(c),
                _ => None,
            })
            .unwrap();
        assert_eq!(card.body, "现在可以重试了。");
    });
    let spoken = f.spoken.spoken.borrow().clone();
    assert_eq!(
        spoken
            .iter()
            .filter(|(text, _)| text == "现在可以重试")
            .count(),
        1,
        "{spoken:?}"
    );
}

#[gpui_kit::test]
fn settings_cmd_comma_opens_and_escape_restores_focus(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.update(cx, |app, window, cx| {
        assert!(
            app.composer().read(cx).focus_handle(cx).is_focused(window),
            "composer is focused before opening settings"
        );
    });
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        window.press("cmd-,", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings").is_some(), "⌘, opens settings");
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings").is_none(), "Esc closes settings");
    });
    f.update(cx, |app, window, cx| {
        assert!(
            app.composer().read(cx).focus_handle(cx).is_focused(window),
            "focus returns to the composer"
        );
    });

    // Close button also restores the focus captured at open.
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        window.press("cmd-,", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings").is_some());
        window.click("settings-close", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings").is_none());
    });
    f.update(cx, |app, window, cx| {
        assert!(
            app.composer().read(cx).focus_handle(cx).is_focused(window),
            "关闭 also restores the prior focus"
        );
    });
}

fn key_snapshot(app: &ForgeApp, openai: KeyStatus) -> Event {
    let session = app.state.selected().unwrap().session.clone();
    Event::Snapshot {
        sessions: vec![session],
        workspaces: vec![Workspace {
            id: "ws".into(),
            root: "/tmp/ws".into(),
            name: "ws".into(),
        }],
        settings: app.state.settings.settings.clone().unwrap(),
        providers: vec![
            ProviderKeyStatus {
                provider: "openai".into(),
                key_status: openai,
            },
            ProviderKeyStatus {
                provider: "anthropic".into(),
                key_status: KeyStatus::Missing,
            },
        ],
        active_runs: vec![],
        last_seq: app.state.last_seq + 1,
    }
}

#[gpui_kit::test]
fn keychain_unavailable_shows_retry_not_missing_and_keeps_focus(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.update(cx, |app, window, cx| {
        let event = key_snapshot(app, KeyStatus::Unavailable);
        fixture_event(app, event, window, cx);
    });
    cx.run_until_parked();
    f.backend.take();
    f.update(cx, |app, _, _| {
        assert_ne!(
            app.state.composer_mode(),
            forge_state::ComposerMode::Disabled(forge_state::DisabledReason::NoKey),
            "unavailable is not the 未配置 Key state"
        );
    });
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("no-key").is_none(), "no missing-key banner");
        assert!(window.try_find("composer-open-settings").is_none());
        window.press("cmd-,", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-key-openai").label(),
            Some("OpenAI，无法读取钥匙串")
        );
        assert_eq!(
            window.find("settings-key-anthropic").label(),
            Some("Anthropic，未配置"),
            "missing keeps the existing 未配置 text"
        );
        assert!(window.try_find("settings-retry-anthropic").is_none());
        let retry = window.find("settings-retry-openai");
        assert_eq!(retry.label(), Some("重试读取 OpenAI 的钥匙串"));
        window.click("settings-retry-openai", cx);
    });
    cx.run_until_parked();
    let sent = f.backend.take();
    assert_eq!(
        sent.iter().filter(|c| **c == Command::GetSnapshot).count(),
        1,
        "重试 re-sends GetSnapshot once: {sent:?}"
    );
    f.update(cx, |app, window, _| {
        assert!(app.key_retry_focused(0, window), "focus stays on 重试");
    });

    // Still unreadable: one polite announcement, button and focus stay.
    f.update(cx, |app, window, cx| {
        let event = key_snapshot(app, KeyStatus::Unavailable);
        fixture_event(app, event, window, cx);
    });
    cx.run_until_parked();
    let spoken = f.spoken.spoken.borrow().clone();
    let still: Vec<_> = spoken
        .iter()
        .filter(|(text, _)| text == "仍无法读取钥匙串")
        .collect();
    assert_eq!(still.len(), 1, "{spoken:?}");
    assert_ne!(
        still[0].1,
        AnnouncementPriority::High,
        "polite, not assertive"
    );
    f.update(cx, |app, window, _| {
        assert!(app.key_retry_focused(0, window));
    });

    // Retry again; keychain readable now → button gone, focus moves to the
    // OpenAI key field (not lost), announced once.
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        window.click("settings-retry-openai", cx);
    });
    cx.run_until_parked();
    let sent = f.backend.take();
    assert_eq!(
        sent.iter().filter(|c| **c == Command::GetSnapshot).count(),
        1,
        "{sent:?}"
    );
    f.update(cx, |app, window, cx| {
        let event = key_snapshot(app, KeyStatus::Configured);
        fixture_event(app, event, window, cx);
    });
    cx.run_until_parked();
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-retry-openai").is_none());
        assert_eq!(
            window.find("settings-key-openai").label(),
            Some("OpenAI，已保存，未验证")
        );
    });
    f.update(cx, |app, window, cx| {
        assert!(!app.key_retry_focused(0, window));
        assert!(window.focused(cx).is_some(), "focus is not dropped");
    });
    let spoken = f.spoken.spoken.borrow().clone();
    assert_eq!(
        spoken
            .iter()
            .filter(|(text, _)| text == "已重新读取钥匙串")
            .count(),
        1,
        "{spoken:?}"
    );
}

/// Every control of the settings panel lies inside the panel's bounds.
fn assert_settings_controls_inside(
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
    case: &str,
) {
    window.render_frame(cx);
    let panel = window.find("settings").bounds();
    let ids = [
        "settings-close",
        "settings-status-openai",
        "settings-retry-openai",
        "settings-input-openai",
        "settings-save-openai",
        "settings-remove-openai",
        "settings-status-anthropic",
        "settings-input-anthropic",
        "settings-save-anthropic",
        "settings-remove-anthropic",
    ];
    let mut seen = 0;
    for id in ids {
        let Some(el) = window.try_find(id) else {
            continue;
        };
        seen += 1;
        let b = el.bounds();
        assert!(
            b.origin.x >= panel.origin.x
                && b.origin.y >= panel.origin.y
                && b.origin.x + b.size.width <= panel.origin.x + panel.size.width
                && b.origin.y + b.size.height <= panel.origin.y + panel.size.height,
            "{case}: {id} {b:?} outside panel {panel:?}"
        );
    }
    assert!(seen >= 5, "{case}: only {seen} controls found");
}

#[gpui_kit::test]
fn settings_rows_fit_panel_in_every_key_state(cx: &mut TestAppContext) {
    for width in [960., 1280.] {
        for (case, openai) in [
            ("unavailable", KeyStatus::Unavailable),
            ("missing", KeyStatus::Missing),
            ("configured", KeyStatus::Configured),
        ] {
            let f = Fixture::new(cx, width);
            f.update(cx, |app, window, cx| {
                let event = key_snapshot(app, openai);
                fixture_event(app, event, window, cx);
                app.open_settings(window, cx);
            });
            cx.run_until_parked();
            f.with_window(cx, |window, cx| {
                assert_settings_controls_inside(window, cx, &format!("{case} @{width}"))
            });
            if case == "configured" {
                // Saving: a key typed and 保存 pressed (Pending::SetKey).
                f.update(cx, |app, window, cx| {
                    app.state
                        .set_provider_key("openai".into(), Some("sk-x".into()));
                    app.state.test_provider_key("openai".into());
                    app.after_change(window, cx);
                });
                f.with_window(cx, |window, cx| {
                    assert_settings_controls_inside(window, cx, &format!("saving @{width}"))
                });
            }
        }
    }
}

fn long_conversation(f: &Fixture, cx: &mut TestAppContext) {
    let text = "一行回答。\n\n".repeat(12);
    for i in 1..=12 {
        let id = format!("r{i}");
        f.start_run(cx, &id);
        f.events(
            cx,
            vec![
                Event::TextDelta {
                    run: id.as_str().into(),
                    text: text.clone(),
                },
                Event::RunFinished {
                    run: id.as_str().into(),
                    outcome: RunOutcome::Succeeded,
                    finished_at: 0,
                },
            ],
        );
    }
    f.with_window(cx, |window, cx| window.render_frame(cx));
}

#[gpui_kit::test]
fn settings_scrim_blocks_scroll_and_clicks_behind_and_does_not_close(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    long_conversation(&f, cx);
    let scroll_up = ScrollDelta::Pixels(point(px(0.), px(600.)));

    // Control: without the panel, the wheel scrolls the timeline.
    let before = f.update(cx, |app, _, cx| app.timeline_scroll_top(cx));
    f.with_window(cx, |window, cx| {
        window.scroll("timeline", scroll_up, cx);
        window.render_frame(cx);
    });
    let after = f.update(cx, |app, _, cx| app.timeline_scroll_top(cx));
    assert_ne!(
        before, after,
        "wheel scrolls the timeline when no panel is open"
    );

    f.with_window(cx, |window, cx| {
        window.press("cmd-,", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings").is_some());
    });
    let before = f.update(cx, |app, _, cx| app.timeline_scroll_top(cx));
    f.with_window(cx, |window, cx| {
        window.scroll("timeline", scroll_up, cx);
        window.render_frame(cx);
    });
    let after = f.update(cx, |app, _, cx| app.timeline_scroll_top(cx));
    assert_eq!(
        before, after,
        "scroll wheel over the scrim must not scroll the timeline"
    );

    // Clicking the scrim (outside the panel) keeps the panel open.
    f.with_window(cx, |window, cx| {
        window.click_at("settings-overlay", point(px(8.), px(8.)), cx);
        window.render_frame(cx);
        assert!(
            window.try_find("settings").is_some(),
            "scrim click does not close"
        );
        // A click on a control behind the scrim does nothing either.
        window.click("new-session", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings").is_some());
    });
    cx.run_until_parked();
    let sent = f.backend.take();
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, Command::CreateSession { .. })),
        "新会话 behind the scrim must not react: {sent:?}"
    );
}

#[gpui_kit::test]
fn settings_initial_focus_and_tab_cycle_stay_in_panel(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        window.press("cmd-,", cx);
        window.render_frame(cx);
    });
    f.update(cx, |app, window, cx| {
        assert!(
            app.key_input_focused(0, window, cx),
            "initial focus: first control of the panel (OpenAI key field)"
        );
    });
    // Shift-Tab from the first control wraps to the last (关闭).
    f.with_window(cx, |window, cx| {
        window.press("shift-tab", cx);
        window.render_frame(cx);
    });
    f.update(cx, |app, window, _| {
        assert!(
            app.settings_close_focused(window),
            "Shift-Tab wraps to 关闭"
        );
    });
    // Tab from the last wraps back to the first.
    f.with_window(cx, |window, cx| {
        window.press("tab", cx);
        window.render_frame(cx);
    });
    f.update(cx, |app, window, cx| {
        assert!(
            app.key_input_focused(0, window, cx),
            "Tab wraps to the first control"
        );
    });
    // A full forward cycle never leaves the panel.
    for step in 0..12 {
        f.with_window(cx, |window, cx| {
            window.press("tab", cx);
            window.render_frame(cx);
        });
        f.update(cx, |app, window, cx| {
            assert!(
                app.settings_contains_focus(window, cx),
                "tab {step} left the panel"
            );
        });
    }
    for step in 0..12 {
        f.with_window(cx, |window, cx| {
            window.press("shift-tab", cx);
            window.render_frame(cx);
        });
        f.update(cx, |app, window, cx| {
            assert!(
                app.settings_contains_focus(window, cx),
                "shift-tab {step} left the panel"
            );
        });
    }
    // Esc closes and focus returns to the composer (focused before opening).
    f.with_window(cx, |window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
    });
    f.update(cx, |app, window, cx| {
        assert!(app.composer().read(cx).focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn settings_initial_focus_is_retry_when_keychain_unavailable(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.update(cx, |app, window, cx| {
        let event = key_snapshot(app, KeyStatus::Unavailable);
        fixture_event(app, event, window, cx);
        app.open_settings(window, cx);
    });
    f.update(cx, |app, window, _| {
        assert!(
            app.key_retry_focused(0, window),
            "重试 is the first control"
        );
    });
}

#[gpui_kit::test]
fn example_n_fills_text_n(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    for (n, text) in [
        (1, "解释这个项目的结构"),
        (2, "找出并修复一个失败的测试"),
        (3, "给最近的改动写说明"),
    ] {
        f.with_window(cx, |window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find(format!("example-{n}")).label(), Some(text));
            window.click(format!("example-{n}"), cx);
            window.render_frame(cx);
        });
        f.update(cx, |app, window, cx| {
            assert_eq!(
                app.composer().read(cx).value().as_ref(),
                text,
                "example {n}"
            );
            assert!(app.composer().read(cx).focus_handle(cx).is_focused(window));
        });
        assert!(
            !f.backend
                .take()
                .iter()
                .any(|c| matches!(c, Command::Send { .. })),
            "example {n} fills only, never sends"
        );
    }
}

#[gpui_kit::test]
fn settings_scrim_covers_window_and_uses_scrim_token(cx: &mut TestAppContext) {
    use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
    for (width, mode, dark) in [
        (960., ThemeMode::Light, false),
        (1280., ThemeMode::Dark, true),
    ] {
        let f = Fixture::new(cx, width);
        f.update(cx, |app, window, cx| {
            Theme::change(mode, Some(window), cx);
            app.open_settings(window, cx);
        });
        f.with_window(cx, |window, cx| {
            window.render_frame(cx);
            let viewport = window.viewport_size();
            let scrim = window.find("settings-overlay").bounds();
            assert_eq!(scrim.origin, Point::default(), "{mode:?}");
            assert_eq!(scrim.size, viewport, "{mode:?}: scrim spans the window");
            // Sidebar, main (incl. the empty-state examples) and composer
            // all lie under the scrim.
            for id in [
                "sidebar",
                "new-session",
                "timeline-empty",
                "example-3",
                "composer-send",
            ] {
                let b = window.find(id).bounds();
                assert!(
                    b.origin.x >= scrim.origin.x
                        && b.origin.y >= scrim.origin.y
                        && b.origin.x + b.size.width <= scrim.origin.x + scrim.size.width
                        && b.origin.y + b.size.height <= scrim.origin.y + scrim.size.height,
                    "{mode:?}: {id} {b:?} not under scrim {scrim:?}"
                );
            }
            let expected = gpui_kit::rgba(forge_ui::tokens::scrim(dark));
            let actual = gpui_kit::Rgba::from(cx.theme().overlay);
            for (a, e, ch) in [
                (actual.r, expected.r, "r"),
                (actual.g, expected.g, "g"),
                (actual.b, expected.b, "b"),
                (actual.a, expected.a, "a"),
            ] {
                assert!(
                    (a - e).abs() < 0.01,
                    "{mode:?}: overlay {ch} {a} vs token {e}"
                );
            }
        });
    }
}

// ------------------------------------------------------------------ v2.1 §0

/// §0: the header is 52pt with 24pt side padding, ＋新会话 is a 32pt row,
/// and the empty column keeps its last row 48pt off the bottom edge.
#[gpui_kit::test]
fn v2_1_header_sidebar_and_empty_metrics(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        let header = window.find("session-header");
        assert_eq!(header.bounds().size.height, px(52.));
        // The title starts exactly one PADDING_X inside the header.
        let title = window.find("session-title");
        assert_eq!(
            title.bounds().origin.x - header.bounds().origin.x,
            px(24.),
            "horizontal padding is 24 now"
        );
        assert_eq!(window.find("new-session").bounds().size.height, px(32.));
        // Bottom padding 60 → 48: the column centers its block inside the
        // padded box, so the bottom slack exceeds the top slack by exactly
        // the padding (48) regardless of the block's height.
        let col = window.find("timeline-empty").bounds();
        let first = window.find("empty-title").bounds();
        let last = window.find("example-3").bounds();
        let bottom_slack = col.origin.y + col.size.height - (last.origin.y + last.size.height);
        let top_slack = first.origin.y - col.origin.y;
        assert_eq!(bottom_slack - top_slack, px(48.));
    });
}

/// §0: idle renders no hint row — no painted line and no leftover gap — and
/// the send button's accessibility description carries "↩ 发送 · ⇧↩ 换行",
/// reachable when focus lands on it (the managed tooltip itself is
/// hover-only, so the description is the keyboard surface).
#[gpui_kit::test]
fn idle_composer_has_no_hint_row_and_send_carries_the_shortcuts(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("composer-hint").is_none(),
            "the idle hint row is gone"
        );
        // The composer column is exactly the card: no residual row or gap.
        let composer = window.find("composer");
        let card = window.find("composer-box");
        assert_eq!(composer.bounds(), card.bounds());
        let send = window.find("composer-send");
        assert_eq!(send.label(), Some("发送"));
        assert_eq!(send.description(), Some("↩ 发送 · ⇧↩ 换行"));
        assert_eq!(send.bounds().size.height, px(32.));
        assert_eq!(send.disabled(), Some(true), "empty draft");
    });
    // A real draft — typed like a user — enables the button so it joins tab
    // traversal, where its description announces both shortcuts. Typed input
    // reaches `set_draft` via deferred `InputEvent`s: give the app a
    // `run_until_parked` between typing and asserting.
    let input_id = f.composer_id(cx);
    f.with_window(cx, |window, cx| window.click(input_id, cx));
    cx.run_until_parked();
    f.with_window(cx, |window, cx| window.input("draft", cx));
    cx.run_until_parked();
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("composer-send").disabled(),
            None,
            "draft enables send"
        );
        let mut focused = None;
        for _ in 0..12 {
            window.focus_next(cx);
            window.render_frame(cx);
            focused = window.find("composer-send").focused();
            if focused == Some(true) {
                break;
            }
        }
        assert_eq!(focused, Some(true), "Tab reaches the send button");
        assert_eq!(
            window.find("composer-send").description(),
            Some("↩ 发送 · ⇧↩ 换行")
        );
    });
}

/// §0: only the actionable notices remain under the composer — the queue
/// hint while running with `queue_when_busy`, and the no-key notice with
/// its 打开设置 link — and the old 28pt controls are 32pt everywhere.
#[gpui_kit::test]
fn v2_1_remaining_hints_and_32pt_controls(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    // The no-key notice keeps its link on the empty screen — and its
    // 打开设置 button is a 32pt control like the rest.
    f.update(cx, |app, window, cx| {
        let key = Event::ProviderKeyConfigured {
            provider: "openai".into(),
            key_status: KeyStatus::Missing,
        };
        fixture_event(app, key, window, cx);
    });
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("no-key").is_some());
        assert!(window.try_find("composer-open-settings").is_some());
        assert_eq!(
            window.find("composer-hint").label(),
            Some("先在设置里添加 API Key"),
            "no-key notice stays"
        );
        assert_eq!(window.find("no-key-settings").bounds().size.height, px(32.));
    });

    // Key restored → a run with queue support shows "↩ 加入队列 · ⌥↩ 插话".
    f.update(cx, |app, window, cx| {
        let key = Event::ProviderKeyConfigured {
            provider: "openai".into(),
            key_status: KeyStatus::Configured,
        };
        fixture_event(app, key, window, cx);
        app.state.capabilities = forge_state::Capabilities::P3;
        app.state.set_draft("hi");
        app.state.composer_key(forge_state::ComposerKey::Enter);
        app.after_change(window, cx);
    });
    cx.run_until_parked();
    f.backend.take();
    f.events(
        cx,
        vec![Event::RunStarted {
            run: run("r1"),
            input_id: "in-x".into(),
        }],
    );
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        let hint = window.find("composer-hint");
        assert!(hint.visible(), "queue hint stays");
        assert_eq!(hint.label(), Some("↩ 加入队列 · ⌥↩ 插话"));
        assert_eq!(window.find("composer-stop").label(), Some("停止"));
    });

    // Error-card action buttons are 32pt (was ~28).
    f.update(cx, |app, window, cx| {
        let finished = Event::RunFinished {
            run: "r1".into(),
            outcome: RunOutcome::Failed {
                kind: RunErrorKind::Network,
                message: "lost".into(),
                http_status: None,
                retry_after_ms: None,
                retryable: true,
            },
            finished_at: 1,
        };
        fixture_event(app, finished, window, cx);
    });
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        for id in ["error-r1-primary", "error-r1-secondary"] {
            assert_eq!(
                window.find(id).bounds().size.height,
                px(32.),
                "{id} is a 32pt control"
            );
        }
    });

    // Settings: key field, 保存/移除, ✕ and the theme track are all 32pt;
    // the theme segments are the 28pt outline nested inside it.
    f.with_window(cx, |window, cx| {
        window.press("cmd-,", cx);
        window.render_frame(cx);
        for id in [
            "settings-input-openai",
            "settings-save-openai",
            "settings-remove-openai",
            "settings-input-anthropic",
            "settings-save-anthropic",
            "settings-close",
            "settings-theme-track",
        ] {
            assert_eq!(
                window.find(id).bounds().size.height,
                px(32.),
                "{id} is a 32pt control"
            );
        }
        for id in [
            "settings-theme-light",
            "settings-theme-dark",
            "settings-theme-system",
        ] {
            assert_eq!(
                window.find(id).bounds().size.height,
                px(28.),
                "{id}: 32pt track − 2 × 2pt inset"
            );
        }
    });
}

fn painted(window: &gpui_kit::Window, text: &str) -> bool {
    gpui_kit::base::test_support::texts(window)
        .into_iter()
        .any(|t| t.visible() && t.text() == text)
}

/// §0/§4 rendered-tree keyboard-help tests: the shortcut surface is a
/// painted chip — not AccessKit metadata or the hover-only managed tooltip.
/// Input focus reveals "↩ 发送 · ⇧↩ 换行" while the empty-draft send stays
/// disabled and unreachable; a focused enabled send reveals the same chip.
/// Escape and blur hide it (Escape keeps focus and sends nothing), and the
/// hover tooltip + a11y description stay on the button throughout.
#[gpui_kit::test]
fn composer_keyboard_focus_paints_shortcut_help(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    // Empty draft: send is disabled and not a tab stop, but the focused
    // input still paints the shortcuts above the control.
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        let chip = window
            .try_find("composer-help")
            .expect("input focus must show the help chip");
        assert!(chip.visible());
        assert!(
            painted(window, "↩ 发送 · ⇧↩ 换行"),
            "the shortcuts are rendered text, not a11y metadata"
        );
        assert_eq!(window.find("composer-send").disabled(), Some(true));
        assert_eq!(
            window.find("composer-send").description(),
            Some("↩ 发送 · ⇧↩ 换行"),
            "the accessibility description stays on the control"
        );
        assert!(
            chip.bounds().bottom() <= window.find("composer-send").bounds().top(),
            "the chip floats above the send control"
        );
        assert_eq!(
            window.find("composer").bounds(),
            window.find("composer-box").bounds(),
            "the chip reserves no layout space"
        );
    });
    // Escape hides the chip; focus stays in the input; nothing sends.
    let input = f.composer_id(cx);
    f.with_window(cx, |window, cx| window.press("escape", cx));
    cx.run_until_parked();
    f.with_window(cx, |window, _| {
        assert!(window.try_find("composer-help").is_none());
        assert_eq!(
            window.find(input.clone()).focused(),
            Some(true),
            "Escape keeps focus, it never steals it"
        );
    });
    assert!(f.backend.take().is_empty(), "Escape sent nothing");
    // Blur re-arms: Tab to the attach control hides the send chip (its own
    // note paints instead); Shift-Tab back to the input brings it back.
    f.with_window(cx, |window, cx| {
        window.focus_next(cx);
        window.render_frame(cx);
        assert_eq!(window.find("composer-attach").focused(), Some(true));
        assert!(window.try_find("composer-help").is_none());
        let tip = window
            .try_find("composer-attach-help")
            .expect("attach focus shows its own chip");
        assert!(tip.visible());
        assert!(painted(window, "添加附件（即将支持）"));
        window.focus_prev(cx);
        window.render_frame(cx);
        let chip = window
            .try_find("composer-help")
            .expect("input refocus re-arms the chip");
        assert!(chip.visible());
    });
    // A real draft enables send; focusing it paints the same shortcuts.
    f.with_window(cx, |window, cx| window.input("draft", cx));
    cx.run_until_parked();
    f.with_window(cx, |window, cx| {
        assert_eq!(window.find("composer-send").disabled(), None);
        let mut focused = None;
        for _ in 0..12 {
            window.focus_next(cx);
            window.render_frame(cx);
            focused = window.find("composer-send").focused();
            if focused == Some(true) {
                break;
            }
        }
        assert_eq!(focused, Some(true), "Tab reaches the enabled send");
        let chip = window
            .try_find("composer-help")
            .expect("send focus shows the help chip");
        assert!(chip.visible());
        assert!(painted(window, "↩ 发送 · ⇧↩ 换行"));
        // Escape hides it; focus stays on send; the draft survives.
        window.press("escape", cx);
        assert!(window.try_find("composer-help").is_none());
        assert_eq!(window.find("composer-send").focused(), Some(true));
        assert_eq!(
            window.find("composer-send").disabled(),
            None,
            "Escape kept the draft"
        );
    });
    cx.run_until_parked();
    assert!(
        !f.backend
            .take()
            .iter()
            .any(|c| matches!(c, Command::Send { .. })),
        "Escape on the send button must not send"
    );
    // Blur hides it too.
    f.with_window(cx, |window, cx| {
        window.focus_next(cx);
        window.render_frame(cx);
        assert!(window.try_find("composer-help").is_none(), "blur hides");
    });
}

/// §0/§4: the same focus-chip rule covers the error card's ⋯ and shortcut
/// buttons plus the settings ⓘ notes — every migrated tooltip paints on
/// keyboard focus too.
#[gpui_kit::test]
fn error_card_and_settings_notes_paint_keyboard_help(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, 1280.);
    // Failed run → error card with 重试 (⌘R) + ⋯.
    f.start_run(cx, "r1");
    f.update(cx, |app, window, cx| {
        let finished = Event::RunFinished {
            run: "r1".into(),
            outcome: RunOutcome::Failed {
                kind: RunErrorKind::Network,
                message: "lost".into(),
                http_status: None,
                retry_after_ms: None,
                retryable: true,
            },
            finished_at: 1,
        };
        fixture_event(app, finished, window, cx);
    });
    cx.run_until_parked();
    f.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("error-r1-primary").label(), Some("重试"));
        // The ⋯ overflow sits first in the card's tab order (header row):
        // focus it and its "更多操作" note paints as a chip.
        let mut focused = None;
        for _ in 0..40 {
            window.focus_next(cx);
            window.render_frame(cx);
            focused = window.find("error-r1-more").focused();
            if focused == Some(true) {
                break;
            }
        }
        assert_eq!(focused, Some(true), "the ⋯ button is a tab stop");
        let chip = window
            .try_find("error-r1-more-tip")
            .expect("⋯ focus paints its note");
        assert!(chip.visible());
        assert!(painted(window, "更多操作"));
        // Escape hides it; the ⋯ keeps focus; nothing fires.
        window.press("escape", cx);
        assert!(window.try_find("error-r1-more-tip").is_none());
        assert_eq!(window.find("error-r1-more").focused(), Some(true));
        // Tab order continues to 重试: "重试 ⌘R" paints on focus too.
        let mut focused = None;
        for _ in 0..10 {
            window.focus_next(cx);
            window.render_frame(cx);
            focused = window.find("error-r1-primary").focused();
            if focused == Some(true) {
                break;
            }
        }
        assert_eq!(focused, Some(true), "the primary action is a tab stop");
        let chip = window
            .try_find("error-r1-primary-tip")
            .expect("primary focus paints its shortcut");
        assert!(chip.visible());
        assert!(painted(window, "重试 ⌘R"));
    });
    assert!(f.backend.take().is_empty(), "Escape fired nothing");

    // Settings ⓘ notes: focus paints the keychain note as a chip.
    f.with_window(cx, |window, cx| {
        window.press("cmd-,", cx);
        window.render_frame(cx);
        let mut focused = None;
        for _ in 0..30 {
            window.focus_next(cx);
            window.render_frame(cx);
            focused = window.find("settings-info-openai").focused();
            if focused == Some(true) {
                break;
            }
        }
        assert_eq!(focused, Some(true), "the ⓘ button is a tab stop");
        let chip = window
            .try_find("settings-info-openai-tip")
            .expect("ⓘ focus paints its note");
        assert!(chip.visible());
        assert!(painted(
            window,
            "Key 只保存在本机钥匙串里，不写入数据库或日志。"
        ));

        // The ✕ header button keeps its label tooltip-only; focus paints
        // "关闭" as the same chip, and Escape still closes the panel.
        let mut focused = None;
        for _ in 0..30 {
            window.focus_next(cx);
            window.render_frame(cx);
            focused = window.find("settings-close").focused();
            if focused == Some(true) {
                break;
            }
        }
        assert_eq!(focused, Some(true), "the ✕ button is a tab stop");
        let chip = window
            .try_find("settings-close-tip")
            .expect("✕ focus paints its label");
        assert!(chip.visible());
        assert!(painted(window, "关闭"));
    });
    f.with_window(cx, |window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("settings-close-tip").is_none()
                && window.try_find("settings-close").is_none(),
            "Escape still dismisses the panel (chip and button gone)"
        );
    });
}
