//! Headless UI tests: real gpui-kit window, real key dispatch, recorded backend.
use forge_protocol::*;
use forge_state::{ComposerMode, QuitState};
use forge_ui::ForgeApp;
use forge_ui::backend::{Backend, CommandFuture};
use gpui_kit::test::{TestInput, TestWindowExt};
use gpui_kit::{
    AppContext, Bounds, ElementId, ElementInputHandler, Entity, Point, TestAppContext,
    WindowBounds, WindowHandle, WindowOptions, base::Root, px, size,
};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Recording {
    commands: Arc<Mutex<Vec<Command>>>,
}

impl Recording {
    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut self.commands.lock().unwrap())
    }
}

impl Backend for Recording {
    fn execute(&self, command: Command) -> CommandFuture {
        self.commands.lock().unwrap().push(command);
        Box::pin(async { Some(CommandResult::Accepted { id: "cmd".into() }) })
    }
}

fn model() -> ModelSelection {
    ModelSelection {
        provider: "openai".into(),
        model: "gpt-5".into(),
    }
}

fn session(id: &str) -> Session {
    Session {
        id: id.into(),
        workspace_id: "ws".into(),
        title: format!("会话 {id}"),
        model: model(),
        permission_mode: PermissionMode::Ask,
        state: SessionState::Idle,
        archived: false,
        queue_state: QueueState::Active,
        created_at: 0,
        updated_at: 0,
        last_finished_run: None,
    }
}

fn run(id: &str, session: &str) -> Run {
    Run {
        id: id.into(),
        session_id: session.into(),
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
}

impl Fixture {
    fn new(cx: &mut TestAppContext) -> Self {
        let backend = Recording::default();
        let shared = backend.clone();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            forge_ui::init(cx);
            let (window, app) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1000.), px(700.)),
                    })),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    cx.new(|cx| ForgeApp::new(Rc::new(shared), None, None, window, cx))
                },
            )
            .expect("open window");
            (window.downcast::<Root>().unwrap(), app)
        });
        let fixture = Self {
            handle,
            app,
            backend,
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
            app.apply_event(event(app, key), window, cx);
            app.apply_event(event(app, Event::WorkspaceAdded { workspace }), window, cx);
            for id in ["s1", "s2"] {
                let created = Event::SessionCreated {
                    session: session(id),
                };
                app.apply_event(event(app, created), window, cx);
            }
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

    fn composer_id(&self, cx: &mut TestAppContext) -> ElementId {
        let composer = cx.update(|cx| self.app.read(cx).composer().clone());
        ("input", composer.entity_id()).into()
    }

    /// Selects `s1`, answers its LoadHistory and focuses the composer.
    fn select_s1(&self, cx: &mut TestAppContext) {
        self.with_window(cx, |window, cx| {
            window.click("session-s1", cx);
        });
        cx.run_until_parked();
        let id = self.composer_id(cx);
        self.with_window(cx, |window, cx| window.click(id, cx));
        cx.run_until_parked();
    }

    fn sends(&self) -> Vec<(String, String)> {
        self.backend
            .take()
            .into_iter()
            .filter_map(|command| match command {
                Command::Send {
                    input_id, content, ..
                } => Some((input_id.0, content.text)),
                _ => None,
            })
            .collect()
    }
}

fn event(app: &ForgeApp, event: Event) -> EventEnvelope {
    EventEnvelope {
        seq: app.state.last_seq + 1,
        event,
    }
}

#[gpui_kit::test]
fn sidebar_lists_sessions_and_ctrl_cmd_s_toggles(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    fixture.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sidebar").is_some());
        assert!(window.try_find("session-s1").is_some());
        assert!(window.try_find("session-s2").is_some());
        window.press("ctrl-cmd-s", cx);
        assert!(
            window.try_find("sidebar").is_none(),
            "⌃⌘S hides the sidebar"
        );
        window.press("ctrl-cmd-s", cx);
        assert!(window.try_find("sidebar").is_some(), "⌃⌘S shows it again");
    });
}

#[gpui_kit::test]
fn clicking_a_row_selects_it_and_loads_history_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    // A session restored from an earlier launch has history not loaded yet
    // (sessions created live start with an empty, complete history).
    fixture.update(cx, |app, _, _| {
        app.state
            .sessions
            .get_mut(&SessionId::from("s2"))
            .unwrap()
            .history = forge_state::HistoryState::NotLoaded;
    });
    for _ in 0..2 {
        fixture.with_window(cx, |window, cx| window.click("session-s2", cx));
        cx.run_until_parked();
    }
    let loads: Vec<_> = fixture
        .backend
        .take()
        .into_iter()
        .filter_map(|c| match c {
            Command::LoadHistory { session, .. } => Some(session),
            _ => None,
        })
        .collect();
    assert_eq!(
        loads,
        vec![SessionId::from("s2")],
        "one LoadHistory, not per click"
    );
    fixture.update(cx, |app, _, _| {
        assert_eq!(app.state.selected_session, Some("s2".into()));
        let row = app
            .state
            .sidebar()
            .recent
            .into_iter()
            .find(|row| row.id.0 == "s2")
            .unwrap();
        assert!(row.selected);
    });
}

#[gpui_kit::test]
fn enter_sends_and_shift_enter_inserts_newline(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    fixture.select_s1(cx);
    fixture.backend.take();
    fixture.with_window(cx, |window, cx| {
        window.input("你好", cx);
        window.press("shift-enter", cx);
        window.input("world", cx);
    });
    cx.run_until_parked();
    assert!(fixture.sends().is_empty(), "⇧↩ must not send");
    fixture.update(cx, |app, _, cx| {
        assert_eq!(app.composer().read(cx).value(), "你好\nworld");
        assert_eq!(app.state.selected().unwrap().composer.draft, "你好\nworld");
    });
    fixture.with_window(cx, |window, cx| window.press("enter", cx));
    cx.run_until_parked();
    let sends = fixture.sends();
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].1, "你好\nworld");
    assert!(!sends[0].0.is_empty(), "UI generates the input id (R10)");
    fixture.update(cx, |app, _, cx| {
        assert_eq!(app.composer().read(cx).value(), "", "accepted send clears");
    });
}

#[gpui_kit::test]
fn enter_with_empty_draft_sends_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    fixture.select_s1(cx);
    fixture.backend.take();
    fixture.with_window(cx, |window, cx| {
        window.input("   ", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
    assert!(fixture.sends().is_empty());
}

#[gpui_kit::test]
fn enter_during_ime_composition_goes_to_the_ime(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    fixture.select_s1(cx);
    fixture.backend.take();
    let id = fixture.composer_id(cx);
    let composer = cx.update(|cx| fixture.app.read(cx).composer().clone());
    // Committed text first, so a wrong send would not be masked by an empty draft.
    fixture.with_window(cx, |window, cx| window.input("你好", cx));
    fixture.with_window(cx, |window, cx| {
        let bounds = window.find(id).bounds();
        let mut input = TestInput::new(ElementInputHandler::new(bounds, composer.clone()), window);
        input.compose_text(None, "ni", Some(2..2), cx);
        assert!(input.marked_text_range(cx).is_some());
    });
    fixture.with_window(cx, |window, cx| window.press("enter", cx));
    cx.run_until_parked();
    assert!(
        fixture.sends().is_empty(),
        "Enter while composing must not send"
    );
    fixture.update(cx, |app, _, _| {
        assert!(
            app.state
                .selected()
                .unwrap()
                .composer
                .draft
                .starts_with("你好")
        );
    });
}

#[gpui_kit::test]
fn enter_while_a_run_is_busy_is_ignored_in_p1(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    fixture.select_s1(cx);
    fixture.update(cx, |app, window, cx| {
        let started = Event::RunStarted {
            run: run("r1", "s1"),
            input_id: "in-x".into(),
        };
        app.apply_event(event(app, started), window, cx);
        assert_eq!(app.state.composer_mode(), ComposerMode::Running);
    });
    fixture.backend.take();
    fixture.with_window(cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("composer-stop").is_some());
        window.input("next", cx);
        window.press("enter", cx);
        window.press("alt-enter", cx);
    });
    cx.run_until_parked();
    assert!(fixture.sends().is_empty());
    assert!(
        !fixture
            .backend
            .take()
            .iter()
            .any(|c| matches!(c, Command::Steer { .. }))
    );
}

#[gpui_kit::test]
fn cmd_period_sends_cancel_run_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    fixture.select_s1(cx);
    fixture.update(cx, |app, window, cx| {
        let started = Event::RunStarted {
            run: run("r1", "s1"),
            input_id: "in-x".into(),
        };
        app.apply_event(event(app, started), window, cx);
    });
    fixture.backend.take();
    fixture.with_window(cx, |window, cx| {
        window.press("cmd-.", cx);
        window.press("cmd-.", cx);
    });
    cx.run_until_parked();
    let cancels = fixture
        .backend
        .take()
        .into_iter()
        .filter(|c| matches!(c, Command::CancelRun { .. }))
        .count();
    assert_eq!(cancels, 1);
}

#[gpui_kit::test]
fn cmd_n_creates_a_session(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    fixture.with_window(cx, |window, cx| window.press("cmd-n", cx));
    cx.run_until_parked();
    assert!(
        fixture
            .backend
            .take()
            .iter()
            .any(|c| matches!(c, Command::CreateSession { .. }))
    );
}

#[gpui_kit::test]
fn quit_with_an_active_run_confirms_then_sends_shutdown(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    fixture.select_s1(cx);
    fixture.update(cx, |app, window, cx| {
        let started = Event::RunStarted {
            run: run("r1", "s1"),
            input_id: "in-x".into(),
        };
        app.apply_event(event(app, started), window, cx);
    });
    fixture.backend.take();

    // Cancel keeps the app open and sends nothing.
    fixture.with_window(cx, |window, cx| window.press("cmd-q", cx));
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消");
    cx.run_until_parked();
    assert!(fixture.backend.take().is_empty());
    fixture.update(cx, |app, _, _| assert_eq!(app.state.quit, QuitState::Idle));

    // Confirm sends Shutdown exactly once and waits for ShutdownComplete.
    fixture.with_window(cx, |window, cx| window.press("cmd-q", cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("停止并退出");
    cx.run_until_parked();
    let shutdowns = fixture
        .backend
        .take()
        .into_iter()
        .filter(|c| matches!(c, Command::Shutdown))
        .count();
    assert_eq!(shutdowns, 1);
    fixture.update(cx, |app, window, cx| {
        assert!(matches!(app.state.quit, QuitState::ShuttingDown { .. }));
        let done = Event::ShutdownComplete {
            stopped: vec!["r1".into()],
            unfinished: vec![],
        };
        app.apply_event(event(app, done), window, cx);
        assert!(app.state.should_exit());
    });
}
