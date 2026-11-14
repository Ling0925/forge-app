#![allow(dead_code)]

use forge_protocol::*;
use forge_state::ids::SequentialIds;
use forge_state::*;

/// App with settings and a configured OpenAI key (`ProviderKeyConfigured`).
pub fn app() -> AppState {
    let mut app = AppState::new(Box::new(SequentialIds::default()));
    app.load_settings(settings(true));
    set_key(&mut app, true);
    app
}

/// `ProviderKeyConfigured{openai, configured}` as the next event.
pub fn set_key(app: &mut AppState, configured: bool) {
    feed(
        app,
        [Event::ProviderKeyConfigured {
            provider: "openai".into(),
            configured,
        }],
    );
}

/// Settings fixture. Whether a key exists is not part of `Settings` any more
/// (P1: `key_configured` comes from the snapshot / `ProviderKeyConfigured`);
/// the flag is kept so call sites read the same.
pub fn settings(_with_key: bool) -> Settings {
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

pub fn session(id: &str) -> Session {
    session_at(id, 0)
}

/// Session whose `updated_at` (= `created_at`) is `at_ms` (UTC ms).
pub fn session_at(id: &str, at_ms: u64) -> Session {
    Session {
        created_at: at_ms,
        updated_at: at_ms,
        id: id.into(),
        workspace_id: "ws".into(),
        title: format!("会话 {id}"),
        model: ModelSelection {
            provider: "openai".into(),
            model: "gpt-5".into(),
        },
        permission_mode: PermissionMode::Ask,
        state: SessionState::Idle,
        archived: false,
        queue_state: QueueState::Active,
        last_finished_run: None,
    }
}

pub fn run(id: &str, session: &str, item: Option<&str>) -> Run {
    Run {
        id: id.into(),
        session_id: session.into(),
        queue_item_id: item.map(Into::into),
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

/// `RunStarted{run, input_id}` for run `id` in `session`.
pub fn started(id: &str, session: &str, input: &str) -> Event {
    Event::RunStarted {
        run: run(id, session, None),
        input_id: input.into(),
    }
}

/// `RunFinished` helpers for the structured outcome.
pub fn finished(id: &str, outcome: RunOutcome) -> Event {
    Event::RunFinished {
        run: id.into(),
        outcome,
    }
}

pub fn failed(kind: RunErrorKind) -> RunOutcome {
    RunOutcome::Failed {
        kind,
        message: String::new(),
        http_status: None,
        retry_after_ms: None,
        retryable: false,
    }
}

pub fn cancelled() -> RunOutcome {
    RunOutcome::Cancelled {
        source: CancelSource::User,
    }
}

pub fn interrupted() -> RunOutcome {
    failed(RunErrorKind::Interrupted)
}

pub fn message(
    ordinal: u64,
    role: MessageRole,
    run: &str,
    input: Option<&str>,
    text: &str,
) -> Message {
    Message {
        id: MessageId(format!("m{ordinal}")),
        ordinal,
        input_id: input.map(Into::into),
        run_id: run.into(),
        turn_id: TurnId(format!("t-{run}")),
        role,
        content: Content {
            text: text.into(),
            attachments: vec![],
        },
        tool_call_id: None,
    }
}

pub fn item(
    id: &str,
    session: &str,
    input: &str,
    text: &str,
    state: QueueItemState,
    ordinal: u64,
) -> QueueItem {
    QueueItem {
        id: id.into(),
        input_id: input.into(),
        session_id: session.into(),
        content: Content {
            text: text.into(),
            attachments: vec![],
        },
        state,
        ordinal,
    }
}

/// Feeds events with consecutive seqs, continuing from `app.last_seq`.
/// Applies events with consecutive seqs. A `RunStarted` fixture without a
/// start time (`started_at == 0`) is stamped with the app clock, the way Core
/// stamps `Run.started_at` with its own clock.
pub fn feed(app: &mut AppState, events: impl IntoIterator<Item = Event>) {
    for mut event in events {
        if let Event::RunStarted { run, .. } = &mut event
            && run.started_at == 0
        {
            run.started_at = app.now_ms();
        }
        let seq = app.last_seq + 1;
        app.apply(EventEnvelope { seq, event });
    }
}

pub fn accepted() -> CommandResult {
    CommandResult::Accepted { id: "cmd".into() }
}

pub fn rejected(code: RejectCode) -> CommandResult {
    CommandResult::Rejected {
        code,
        message: "nope".into(),
    }
}

/// App with workspace `ws`, session `s1` selected and history loaded.
pub fn app_with_session() -> AppState {
    let mut app = app();
    feed(
        &mut app,
        [
            Event::WorkspaceAdded {
                workspace: Workspace {
                    id: "ws".into(),
                    root: "/tmp/ws".into(),
                    name: "ws".into(),
                },
            },
            Event::SessionCreated {
                session: session("s1"),
            },
        ],
    );
    assert!(app.select_session(&"s1".into()));
    app.take_outbox();
    app
}

pub fn assistant_text(app: &AppState, session: &str, run: &str) -> Option<String> {
    app.sessions[&SessionId::from(session)]
        .timeline
        .iter()
        .find_map(|item| match item {
            TimelineItem::Assistant(m) if m.run_id.0 == run => Some(m.text.clone()),
            _ => None,
        })
}

pub fn single_command(app: &mut AppState) -> Outgoing {
    let mut out = app.take_outbox();
    assert_eq!(out.len(), 1, "expected one command, got {out:?}");
    out.remove(0)
}
