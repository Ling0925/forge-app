#![allow(dead_code)]

use forge_protocol::*;
use forge_state::ids::SequentialIds;
use forge_state::*;

pub fn app() -> AppState {
    let mut app = AppState::new(Box::new(SequentialIds::default()));
    app.load_settings(settings(true));
    app
}

pub fn settings(with_key: bool) -> Settings {
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

pub fn session(id: &str) -> Session {
    Session {
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
    }
}

pub fn run(id: &str, session: &str, item: Option<&str>) -> Run {
    Run {
        id: id.into(),
        session_id: session.into(),
        queue_item_id: item.map(Into::into),
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
pub fn feed(app: &mut AppState, events: impl IntoIterator<Item = Event>) {
    for event in events {
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
