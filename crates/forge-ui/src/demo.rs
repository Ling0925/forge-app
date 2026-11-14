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
    /// Rate-limited failure with a retry countdown plus an earlier auth error.
    Error,
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
            _ => None,
        }
    }
}

struct Feeder<'a> {
    app: &'a mut AppState,
    /// Wall-clock origin (UTC ms); scene times are offsets from it.
    base: u64,
    now: u64,
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
        self.accept_all();
        self
    }
}

fn settings(with_key: bool) -> Settings {
    Settings {
        providers: vec![ProviderSettings {
            id: "openai".into(),
            endpoint: None,
            keychain_reference: with_key.then(|| "forge-app.openai".to_owned()),
        }],
        model: ModelSelection {
            provider: "openai".into(),
            model: "gpt-5".into(),
        },
        permission_mode: PermissionMode::Ask,
    }
}

fn session(id: &str, title: &str) -> Session {
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
    }
}

fn run(id: &str, session: &str) -> Run {
    Run {
        id: id.into(),
        session_id: session.into(),
        queue_item_id: None,
        state: RunState::Running,
        terminal_reason: None,
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

fn finished(run: &str, outcome: RunOutcome, reason: Option<&str>) -> Event {
    Event::RunFinished {
        run: run.into(),
        outcome,
        reason: reason.map(Into::into),
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
    };
    f.at(1_000);
    f.app
        .load_settings(settings(!matches!(scene, Scene::Onboarding | Scene::NoKey)));
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
    f.event(Event::SessionCreated {
        session: session("s-old", "整理 README 目录"),
    });
    f.event(Event::SessionCreated {
        session: session("s-ci", "排查构建失败"),
    });
    f.event(Event::SessionCreated {
        session: session("s1", "解释这个项目的结构"),
    });
    f.app.select_session(&"s1".into());
    f.accept_all();
    if scene == Scene::Empty {
        return f.now;
    }

    // Run 1: reasoning (collapsed) + Markdown answer, success.
    f.at(10_000).send("解释一下这个项目的结构，入口在哪？");
    f.event(Event::RunStarted {
        run: run("r1", "s1"),
    });
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
    f.at(57_000)
        .event(finished("r1", RunOutcome::Succeeded, None));

    // Run 2: reasoning expanded, success.
    f.at(70_000).send("侧栏宽度能拖动吗？给个实现顺序。");
    f.event(Event::RunStarted {
        run: run("r2", "s1"),
    });
    f.at(71_000).event(reasoning(
        "r2",
        "规格 §4：默认 240pt，范围 200–360，低于 160 折叠，双击恢复，命中区 8pt。",
    ));
    f.at(75_000).event(delta("r2", ANSWER_2));
    f.event(usage("r2", 13_020, 410, 0.0108));
    f.at(83_000)
        .event(finished("r2", RunOutcome::Succeeded, None));
    f.app.toggle_reasoning(&"r2".into());

    if scene == Scene::Conversation {
        return f.now;
    }

    // Run 3: rate limited with Retry-After 100 s (> 60 s, R7a) → error card + countdown.
    f.at(90_000).send("再把错误卡片也做了吧。");
    f.event(Event::RunStarted {
        run: run("r3", "s1"),
    });
    f.at(91_000).event(Event::ProviderRetrying {
        run: "r3".into(),
        n: 1,
        delay_ms: 2_000,
        reason: "rate_limited".into(),
    });
    f.at(98_000).event(finished(
        "r3",
        RunOutcome::Failed,
        Some(
            "rate_limited http_status=429 retry_after_ms=100000 retryable=true: \
             Rate limit reached for gpt-5 (org-demo). Authorization: Bearer sk-demo1234567890abcdef",
        ),
    ));
    f.app
        .error_action(&"r3".into(), forge_state::ErrorAction::Details);
    f.now
}
