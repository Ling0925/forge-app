//! Fixture scenes for screenshots and manual checks (`FORGE_UI_DEMO=<scene>`).
//! Every scene is built by feeding real protocol `Event`s and command results
//! through `AppState`, exactly like the core would, so what is rendered is the
//! production reduction, not hand-built view models.

use crate::backend::{Backend, CommandFuture};
use forge_protocol::*;
use forge_state::{AppState, ComposerKey};

/// Accepts every command (no core behind it).
pub struct DemoBackend;

impl Backend for DemoBackend {
    fn execute(&self, _command: Command) -> CommandFuture {
        Box::pin(async { Some(CommandResult::Accepted { id: "demo".into() }) })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// First launch: §6.14 wizard, welcome step.
    Onboarding,
    /// Workspace but no session and no key: empty state with the no-key notice.
    NoKey,
    /// Key configured, empty session with the three examples.
    Empty,
    /// Two finished runs: Markdown, reasoning (one expanded), run dividers.
    Conversation,
    /// Rate-limited failure (Retry-After 100 s) with its countdown.
    Error,
    /// Run whose result could not be written (unknown + storage → 保存失败).
    Storage,
}

impl Scene {
    pub fn from_env() -> Option<Scene> {
        Self::parse(&std::env::var("FORGE_UI_DEMO").ok()?)
    }

    pub fn parse(name: &str) -> Option<Scene> {
        match name {
            "onboarding" => Some(Scene::Onboarding),
            "empty" => Some(Scene::Empty),
            "nokey" => Some(Scene::NoKey),
            "conversation" | "timeline" => Some(Scene::Conversation),
            "error" => Some(Scene::Error),
            "storage" => Some(Scene::Storage),
            _ => None,
        }
    }
}

struct Feeder<'a> {
    app: &'a mut AppState,
    /// Wall-clock origin (UTC ms); scene times are offsets from it.
    base: u64,
    now: u64,
    /// `input_id` of the last composer send (Core echoes it in `RunStarted`).
    last_input: InputId,
}

impl Feeder<'_> {
    fn at(&mut self, offset_ms: i64) -> &mut Self {
        self.now = (self.base as i64 + offset_ms) as u64;
        self.app.set_now_ms(self.now);
        self
    }

    fn event(&mut self, event: Event) -> &mut Self {
        let seq = self.app.last_seq + 1;
        self.app.apply(EventEnvelope { seq, event });
        self.accept_all();
        self
    }

    fn accept_all(&mut self) {
        for out in self.app.take_outbox() {
            self.app
                .apply_result(out.token, CommandResult::Accepted { id: "demo".into() });
        }
    }

    fn send(&mut self, text: &str) -> &mut Self {
        self.app.set_draft(text);
        self.app.composer_key(ComposerKey::Enter);
        for out in self.app.take_outbox() {
            if let Command::Send { input_id, .. } = &out.command {
                self.last_input = input_id.clone();
            }
            self.app
                .apply_result(out.token, CommandResult::Accepted { id: "demo".into() });
        }
        self
    }

    /// `RunStarted` for `run` (stamped with the scene clock) answering the
    /// last send, then `SessionUpdated` like Core.
    fn start(&mut self, run_id: &str, session_id: &str, title: &str) -> &mut Self {
        let input_id = std::mem::replace(&mut self.last_input, InputId::from("demo-in"));
        let mut r = run(run_id, session_id);
        r.started_at = self.now;
        self.event(Event::RunStarted { run: r, input_id });
        let mut s = session(session_id, title, self.now);
        s.state = SessionState::Running;
        self.event(Event::SessionUpdated { session: s })
    }

    fn finish(
        &mut self,
        run_id: &str,
        session_id: &str,
        title: &str,
        outcome: RunOutcome,
    ) -> &mut Self {
        self.event(Event::RunFinished {
            run: run_id.into(),
            outcome,
        });
        let s = session(session_id, title, self.now);
        self.event(Event::SessionUpdated { session: s })
    }

    fn create(&mut self, id: &str, title: &str) -> &mut Self {
        let s = session(id, title, self.now);
        self.event(Event::SessionCreated { session: s })
    }
}

fn settings() -> Settings {
    Settings {
        ui_prefs: Default::default(),
        providers: vec![ProviderSettings {
            id: "openai".into(),
            endpoint: None,
            keychain_reference: None,
            max_output_tokens: None,
        }],
        model: ModelSelection {
            provider: "openai".into(),
            model: "gpt-5".into(),
        },
        permission_mode: PermissionMode::Ask,
    }
}

fn session(id: &str, title: &str, at: u64) -> Session {
    Session {
        id: id.into(),
        workspace_id: "ws".into(),
        title: title.into(),
        model: ModelSelection {
            provider: "openai".into(),
            model: "gpt-5".into(),
        },
        permission_mode: PermissionMode::Ask,
        state: SessionState::Idle,
        archived: false,
        queue_state: QueueState::Active,
        created_at: at,
        updated_at: at,
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

fn usage(run: &str, input: u64, output: u64, cost: f64) -> Event {
    Event::UsageRecorded {
        record: UsageRecord {
            id: UsageRecordId::from(format!("u-{run}").as_str()),
            run_id: run.into(),
            provider: "openai".into(),
            model: "gpt-5".into(),
            usage: Usage {
                input_tokens: input,
                output_tokens: output,
                cached_tokens: 0,
                requests: 1,
                estimated_cost_usd: Some(cost),
            },
        },
    }
}

fn delta(run: &str, text: &str) -> Event {
    Event::TextDelta {
        run: run.into(),
        text: text.into(),
    }
}

fn reasoning(run: &str, text: &str) -> Event {
    Event::ReasoningDelta {
        run: run.into(),
        text: text.into(),
    }
}

fn failure(
    kind: RunErrorKind,
    status: Option<u16>,
    retry_after_ms: Option<u64>,
    retryable: bool,
    message: &str,
) -> RunOutcome {
    RunOutcome::Failed {
        kind,
        message: message.into(),
        http_status: status,
        retry_after_ms,
        retryable,
    }
}

const ANSWER_1: &str = "这是一个 **Rust workspace**，分成几层：\n\n\
| crate | 作用 |\n|---|---|\n\
| `forge-protocol` | 命令 / 事件类型 |\n\
| `forge-core` | 运行循环、队列与重试 |\n\
| `forge-state` | 纯状态与视图模型 |\n\
| `forge-ui` | gpui-kit 视图 |\n\n\
入口在 `crates/forge-ui/src/main.rs`：\n\n\
```rust\nfn main() {\n    gpui_kit::platform::application().run(|cx| forge_ui::init(cx));\n}\n```\n\n\
> UI 只渲染 `AppState`，所有判断都在 `forge-state` 里。";

const ANSWER_2: &str = "可以。建议按下面的顺序：\n\n\
1. 在 `forge-state` 里加 `UiPrefs`，宽度限制在 200–360pt；\n\
2. 低于 160pt 时自动折叠，双击分隔条恢复 240pt；\n\
3. 视图层只读 `layout(width)` 的结果。\n\n\
这样拖动逻辑可以单元测试，不依赖窗口。";

/// Builds `scene` into `app` with scene time 0 at `base` (UTC ms); returns
/// the clock at the end (≈ `base` + 100 s).
pub fn populate(app: &mut AppState, scene: Scene, base: u64) -> u64 {
    let mut f = Feeder {
        app,
        base,
        now: base,
        last_input: InputId::from("demo-in"),
    };
    f.at(1_000);
    f.app.load_settings(settings());
    // P1: whether a key exists comes only from Core (`key_configured`).
    let with_key = !matches!(scene, Scene::Onboarding | Scene::NoKey);
    f.event(Event::ProviderKeyConfigured {
        provider: "openai".into(),
        configured: with_key,
    });
    f.event(Event::WorkspaceAdded {
        workspace: Workspace {
            id: "ws".into(),
            root: "/Users/demo/forge-app".into(),
            name: "forge-app".into(),
        },
    });
    if scene == Scene::Onboarding {
        f.app.start_onboarding();
        return f.now;
    }
    if scene == Scene::NoKey {
        return f.now;
    }
    // Sidebar spread: 3 days ago (过去 7 天), yesterday with a failed run,
    // one still running (进行中), and the selected one today.
    const HOUR: i64 = 3_600_000;
    f.at(-74 * HOUR).create("s-old", "整理 README 目录");
    f.at(-26 * HOUR).create("s-ci", "排查构建失败");
    f.start("r-ci", "s-ci", "排查构建失败");
    f.at(-26 * HOUR + 40_000).finish(
        "r-ci",
        "s-ci",
        "排查构建失败",
        failure(RunErrorKind::Network, None, None, true, "connection reset"),
    );
    f.at(-2 * HOUR).create("s-docs", "更新 CHANGELOG");
    f.at(-42_000).create("s-run", "跑一遍 clippy 并修警告");
    f.start("r-run", "s-run", "跑一遍 clippy 并修警告");
    const T1: &str = "解释这个项目的结构";
    f.at(1_000).create("s1", T1);
    f.app.select_session(&"s1".into());
    f.accept_all();
    // The demo has no Core: the (empty) first history page arrives at once.
    f.event(Event::HistoryLoaded {
        session: "s1".into(),
        messages: vec![],
        next_before: None,
    });
    if scene == Scene::Empty {
        return f.now;
    }

    // Run 1: reasoning (collapsed) + Markdown answer, success.
    f.at(10_000).send("解释一下这个项目的结构，入口在哪？");
    f.start("r1", "s1", T1);
    f.at(11_000).event(reasoning(
        "r1",
        "先看 Cargo workspace 的成员，再找带 main 的 crate。",
    ));
    f.at(19_000)
        .event(reasoning("r1", "forge-ui 有 src/main.rs，是唯一的二进制。"));
    let (head, tail) = ANSWER_1.split_at(ANSWER_1.find("入口").unwrap_or(0));
    f.at(20_000)
        .event(delta("r1", head))
        .event(delta("r1", tail));
    f.event(usage("r1", 12_340, 860, 0.0213));
    f.at(57_000).finish("r1", "s1", T1, RunOutcome::Succeeded);

    // Run 2: reasoning expanded, success.
    f.at(70_000).send("侧栏宽度能拖动吗？给个实现顺序。");
    f.start("r2", "s1", T1);
    f.at(71_000).event(reasoning(
        "r2",
        "规格 §4：默认 240pt，范围 200–360，低于 160 折叠，双击恢复，命中区 8pt。",
    ));
    f.at(75_000).event(delta("r2", ANSWER_2));
    f.event(usage("r2", 13_020, 410, 0.0108));
    f.at(83_000).finish("r2", "s1", T1, RunOutcome::Succeeded);
    f.app.toggle_reasoning(&"r2".into());

    if scene == Scene::Conversation {
        return f.now;
    }

    f.at(90_000).send("再把错误卡片也做了吧。");
    f.start("r3", "s1", T1);
    let outcome = if scene == Scene::Storage {
        failure(
            RunErrorKind::Unknown {
                source: UnknownSource::Storage,
            },
            None,
            None,
            false,
            "database is locked (SQLITE_BUSY) while saving message",
        )
    } else {
        // Rate limited with Retry-After 100 s (> 60 s, R7a) → wait card + countdown.
        f.at(91_000).event(Event::ProviderRetrying {
            run: "r3".into(),
            n: 1,
            delay_ms: 2_000,
            reason: "rate_limited".into(),
        });
        failure(
            RunErrorKind::RateLimited,
            Some(429),
            Some(100_000),
            true,
            "Rate limit reached for gpt-5 (org-demo).\nAuthorization: Bearer sk-demo1234567890abcdef",
        )
    };
    f.at(98_000).finish("r3", "s1", T1, outcome);
    f.app
        .error_action(&"r3".into(), forge_state::ErrorAction::Details);
    f.now
}
