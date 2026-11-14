//! P1 headless UI tests: timeline rows, error card + countdown, reasoning
//! disclosure, sidebar resize, Inspector toggle, announcement routing.
use forge_protocol::*;
use forge_state::AnnouncementPriority;
use forge_ui::ForgeApp;
use forge_ui::announce::RecordingAnnouncer;
use forge_ui::backend::{Backend, CommandFuture};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds, WindowHandle,
    WindowOptions, base::Root, point, px, size,
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
                configured: true,
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

    fn events(&self, cx: &mut TestAppContext, events: Vec<Event>) {
        self.update(cx, |app, window, cx| {
            for event in events {
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
            configured: false,
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
