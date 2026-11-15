mod common;
use forge_core::{
    Runtime, RuntimeHandle, RuntimeServices,
    secrets::{MemorySecretStore, SecretStore},
};
use forge_protocol::*;
use forge_provider::{ErrorKind, ProviderError, StreamEvent};
use forge_store::Store;
use forge_testkit::provider::{FixedProviderFactory, Script, ScriptedProvider, Step};
use forge_tools::clock::FakeClock;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

const KEY: &str = "sentinel-runtime-key-never-persist";
const READ_ONLY_TOOLS: [&str; 4] = ["glob", "grep", "list_dir", "read_file"];

struct Harness {
    runtime: Runtime,
    handle: RuntimeHandle,
    events: mpsc::UnboundedReceiver<EventEnvelope>,
    provider: Arc<ScriptedProvider>,
    directory: tempfile::TempDir,
}
impl Harness {
    fn new(scripts: impl IntoIterator<Item = Script>) -> Self {
        let clock = Arc::new(FakeClock::default());
        clock.advance(Duration::from_secs(100));
        let provider = Arc::new(ScriptedProvider::new(clock.clone(), scripts));
        let secrets = Arc::new(MemorySecretStore::default());
        secrets.set(&"script".into(), KEY).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let (runtime, handle) = Runtime::new(
            RuntimeServices {
                clock,
                store: Store::open(directory.path()).unwrap(),
                providers: Arc::new(FixedProviderFactory(provider.clone())),
                secrets,
                initial_settings: common::settings("script"),
            },
            0,
        )
        .unwrap();
        let events = handle.subscribe();
        Self {
            runtime,
            handle,
            events,
            provider,
            directory,
        }
    }
    async fn command(&mut self, command: Command) -> CommandResult {
        let future = self.handle.execute(command);
        tokio::pin!(future);
        loop {
            tokio::select! {
                result=&mut future => return result.unwrap(),
                result=self.runtime.dispatch_next() => assert!(result.unwrap()),
            }
        }
    }
    async fn next(&mut self) -> EventEnvelope {
        loop {
            if let Ok(event) = self.events.try_recv() {
                return event;
            }
            assert!(self.runtime.dispatch_next().await.unwrap());
        }
    }
    /// Collect every event through the run's terminal RunFinished.
    async fn until_finished(&mut self) -> Vec<EventEnvelope> {
        let mut events = Vec::new();
        loop {
            let envelope = self.next().await;
            let done = matches!(envelope.event, Event::RunFinished { .. });
            events.push(envelope);
            if done {
                return events;
            }
        }
    }
    async fn session(&mut self, workspace: &Path) -> SessionId {
        let workspace = accepted(
            self.command(Command::AddWorkspace {
                path: workspace.to_owned(),
            })
            .await,
        );
        SessionId(accepted(
            self.command(Command::CreateSession {
                workspace: WorkspaceId(workspace),
                title: "Chat".into(),
                model: common::settings("script").model,
                permission_mode: PermissionMode::Ask,
            })
            .await,
        ))
    }
}

fn accepted(result: CommandResult) -> String {
    match result {
        CommandResult::Accepted { id } => id.0,
        other => panic!("{other:?}"),
    }
}
fn send(session: &SessionId, input: &str) -> Command {
    Command::Send {
        session: session.clone(),
        input_id: input.into(),
        content: Content {
            text: "run the tools".into(),
            attachments: vec![],
        },
    }
}
fn tool_call(id: &str, tool: &str, args: &str) -> Vec<Step> {
    vec![
        Step::Event(StreamEvent::ToolCallStarted {
            id: ToolCallId(id.into()),
            name: tool.into(),
        }),
        Step::Event(StreamEvent::ToolArgumentsDelta {
            id: ToolCallId(id.into()),
            json: args.into(),
        }),
        Step::Event(StreamEvent::Finished),
    ]
}
fn usage(requests: u64) -> StreamEvent {
    StreamEvent::Usage(Usage {
        input_tokens: requests,
        output_tokens: requests,
        cached_tokens: 0,
        requests,
        estimated_cost_usd: None,
    })
}
fn done(text: &str) -> Script {
    Script::Stream(vec![
        Step::Event(StreamEvent::TextDelta(text.into())),
        Step::Event(usage(1)),
        Step::Event(StreamEvent::Finished),
    ])
}

fn proposed(events: &[EventEnvelope]) -> Vec<ToolCall> {
    events
        .iter()
        .filter_map(|envelope| match &envelope.event {
            Event::ToolCallProposed { call } => Some(call.clone()),
            _ => None,
        })
        .collect()
}
fn state_changes(events: &[EventEnvelope]) -> Vec<(ToolCallId, ToolCallState, Option<Content>)> {
    events
        .iter()
        .filter_map(|envelope| match &envelope.event {
            Event::ToolCallStateChanged {
                call,
                state,
                result,
            } => Some((call.clone(), state.clone(), result.clone())),
            _ => None,
        })
        .collect()
}
fn outcome(events: &[EventEnvelope]) -> &RunOutcome {
    match &events
        .iter()
        .find(|envelope| matches!(envelope.event, Event::RunFinished { .. }))
        .expect("run finished")
        .event
    {
        Event::RunFinished { outcome, .. } => outcome,
        _ => unreachable!(),
    }
}
fn persisted_tool_events(h: &Harness) -> Vec<Event> {
    h.runtime
        .services
        .store
        .connection()
        .prepare("SELECT event_json FROM events WHERE json_extract(event_json, '$.type') LIKE 'tool%' ORDER BY seq")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect()
}
/// Assert a request advertises exactly the read-only tool set.
fn assert_advertised(h: &Harness) {
    for request in h.provider.requests() {
        let mut names: Vec<&str> = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, READ_ONLY_TOOLS);
        assert!(request.tools.iter().all(|tool| !tool.name.is_empty()));
    }
}

#[tokio::test]
async fn tool_result_feeds_back_and_run_succeeds() {
    let mut h = Harness::new([
        Script::Stream(vec![
            Step::Event(StreamEvent::ToolCallStarted {
                id: ToolCallId("call-1".into()),
                name: "read_file".into(),
            }),
            Step::Event(StreamEvent::ToolArgumentsDelta {
                id: ToolCallId("call-1".into()),
                json: "{\"path\":\"note".into(),
            }),
            Step::Event(StreamEvent::ToolArgumentsDelta {
                id: ToolCallId("call-1".into()),
                json: ".txt\"}".into(),
            }),
            Step::Event(StreamEvent::Finished),
        ]),
        done("done"),
    ]);
    let workspace = h.directory.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("note.txt"), "file-body\n").unwrap();
    let session = h.session(&workspace).await;
    accepted(h.command(send(&session, "in-1")).await);
    let events = h.until_finished().await;

    let proposed = proposed(&events);
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0].id, ToolCallId("call-1".into()));
    assert_eq!(proposed[0].tool, "read_file");
    assert_eq!(proposed[0].arguments_json, "{\"path\":\"note.txt\"}");
    assert_eq!(proposed[0].state, ToolCallState::Proposed);
    assert!(!proposed[0].has_side_effects);
    assert!(proposed[0].result.is_none());

    let states = state_changes(&events);
    assert_eq!(states.len(), 2);
    assert_eq!(
        states[0],
        (ToolCallId("call-1".into()), ToolCallState::Running, None)
    );
    assert_eq!(states[1].0, ToolCallId("call-1".into()));
    assert_eq!(states[1].1, ToolCallState::Succeeded);
    assert_eq!(states[1].2.as_ref().unwrap().text, "file-body\n");
    assert_eq!(*outcome(&events), RunOutcome::Succeeded);

    assert_advertised(&h);
    let requests = h.provider.requests();
    assert_eq!(requests.len(), 2);
    let tail = &requests[1].messages[requests[1].messages.len() - 2..];
    assert_eq!(tail[0].role, MessageRole::Assistant);
    assert_eq!(tail[0].content.text, "");
    // The assistant carries the call it issued so providers can pair results.
    assert_eq!(tail[0].tool_calls.len(), 1);
    assert_eq!(tail[0].tool_calls[0].id, ToolCallId("call-1".into()));
    assert_eq!(tail[0].tool_calls[0].name, "read_file");
    assert_eq!(
        tail[0].tool_calls[0].arguments_json,
        "{\"path\":\"note.txt\"}"
    );
    assert_eq!(tail[1].role, MessageRole::Tool);
    assert_eq!(tail[1].tool_call_id, Some(ToolCallId("call-1".into())));
    assert_eq!(tail[1].content.text, "file-body\n");
    assert!(!tail[1].is_error);

    assert_eq!(persisted_tool_events(&h).len(), 3);
}

#[tokio::test]
async fn malformed_arguments_fail_the_call_and_run_continues() {
    let mut h = Harness::new([
        Script::Stream(tool_call("call-1", "read_file", "{\"path\":")),
        done("recovered"),
    ]);
    let workspace = h.directory.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    let session = h.session(&workspace).await;
    accepted(h.command(send(&session, "in-1")).await);
    let events = h.until_finished().await;

    let states = state_changes(&events);
    assert_eq!(states.len(), 2);
    assert_eq!(states[0].1, ToolCallState::Running);
    assert_eq!(states[1].1, ToolCallState::Failed);
    let error = states[1].2.as_ref().unwrap().text.clone();
    assert!(!error.is_empty());
    assert_eq!(*outcome(&events), RunOutcome::Succeeded);

    // The failure text is fed back to the provider as the tool result.
    let requests = h.provider.requests();
    assert_eq!(requests.len(), 2);
    let result = requests[1].messages.last().unwrap();
    assert_eq!(result.role, MessageRole::Tool);
    assert_eq!(result.tool_call_id, Some(ToolCallId("call-1".into())));
    assert_eq!(result.content.text, error);
    assert!(result.is_error);
}

#[tokio::test]
async fn tool_failure_becomes_a_failed_result_and_run_continues() {
    let mut h = Harness::new([
        Script::Stream(tool_call(
            "call-1",
            "read_file",
            "{\"path\":\"missing.txt\"}",
        )),
        done("recovered"),
    ]);
    let workspace = h.directory.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    let session = h.session(&workspace).await;
    accepted(h.command(send(&session, "in-1")).await);
    let events = h.until_finished().await;

    let states = state_changes(&events);
    assert_eq!(states.len(), 2);
    assert_eq!(states[1].1, ToolCallState::Failed);
    assert!(!states[1].2.as_ref().unwrap().text.is_empty());
    assert_eq!(*outcome(&events), RunOutcome::Succeeded);
    assert_eq!(h.provider.requests().len(), 2);
}

#[tokio::test]
async fn multiple_calls_run_in_stream_order() {
    let mut h = Harness::new([
        Script::Stream(vec![
            Step::Event(StreamEvent::ToolCallStarted {
                id: ToolCallId("call-1".into()),
                name: "list_dir".into(),
            }),
            Step::Event(StreamEvent::ToolArgumentsDelta {
                id: ToolCallId("call-1".into()),
                json: "{\"path\":\".\"}".into(),
            }),
            Step::Event(StreamEvent::ToolCallStarted {
                id: ToolCallId("call-2".into()),
                name: "glob".into(),
            }),
            Step::Event(StreamEvent::ToolArgumentsDelta {
                id: ToolCallId("call-2".into()),
                json: "{\"pattern\":\"**/*.txt\"}".into(),
            }),
            Step::Event(StreamEvent::Finished),
        ]),
        done("done"),
    ]);
    let workspace = h.directory.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("a.txt"), "a").unwrap();
    std::fs::create_dir(workspace.join("sub")).unwrap();
    std::fs::write(workspace.join("sub").join("b.txt"), "b").unwrap();
    let session = h.session(&workspace).await;
    accepted(h.command(send(&session, "in-1")).await);
    let events = h.until_finished().await;

    // Both proposals are committed before the first call runs.
    let proposed = proposed(&events);
    assert_eq!(proposed.len(), 2);
    assert_eq!(proposed[0].tool, "list_dir");
    assert_eq!(proposed[1].tool, "glob");

    let states = state_changes(&events);
    assert_eq!(states.len(), 4);
    assert_eq!(
        states
            .iter()
            .map(|(call, ..)| call.clone())
            .collect::<Vec<_>>(),
        [
            ToolCallId("call-1".into()),
            ToolCallId("call-1".into()),
            ToolCallId("call-2".into()),
            ToolCallId("call-2".into())
        ]
    );
    assert_eq!(
        states
            .iter()
            .map(|(_, state, _)| state.clone())
            .collect::<Vec<_>>(),
        [
            ToolCallState::Running,
            ToolCallState::Succeeded,
            ToolCallState::Running,
            ToolCallState::Succeeded
        ]
    );
    let listing = states[1].2.as_ref().unwrap().text.clone();
    assert!(
        listing.contains("a.txt") && listing.contains("sub/"),
        "{listing}"
    );
    assert_eq!(states[3].2.as_ref().unwrap().text, "a.txt\nsub/b.txt");

    let requests = h.provider.requests();
    let tail = &requests[1].messages[requests[1].messages.len() - 3..];
    assert_eq!(tail[0].role, MessageRole::Assistant);
    assert_eq!(
        tail[0]
            .tool_calls
            .iter()
            .map(|call| call.id.clone())
            .collect::<Vec<_>>(),
        [ToolCallId("call-1".into()), ToolCallId("call-2".into())]
    );
    assert_eq!(tail[1].tool_call_id, Some(ToolCallId("call-1".into())));
    assert_eq!(tail[1].content.text, listing);
    assert_eq!(tail[2].tool_call_id, Some(ToolCallId("call-2".into())));
    assert_eq!(tail[2].content.text, "a.txt\nsub/b.txt");
    assert_eq!(*outcome(&events), RunOutcome::Succeeded);
}

#[tokio::test]
async fn unadvertised_side_effect_tools_are_rejected() {
    let mut h = Harness::new([
        Script::Stream(tool_call(
            "call-1",
            "write_file",
            "{\"path\":\"evil.txt\",\"content\":\"x\"}",
        )),
        done("done"),
    ]);
    let workspace = h.directory.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    let session = h.session(&workspace).await;
    accepted(h.command(send(&session, "in-1")).await);
    let events = h.until_finished().await;

    // Rejected side-effect tools are still marked side-effecting: the flag
    // describes the tool's nature, not whether this build ran it.
    let proposed = proposed(&events);
    assert_eq!(proposed.len(), 1);
    assert!(proposed[0].has_side_effects);

    let states = state_changes(&events);
    assert_eq!(states.len(), 2);
    assert_eq!(states[1].1, ToolCallState::Failed);
    assert!(states[1].2.as_ref().unwrap().text.contains("write_file"));
    assert!(!workspace.join("evil.txt").exists());
    assert_eq!(*outcome(&events), RunOutcome::Succeeded);
    assert_advertised(&h);
}

#[tokio::test]
async fn a_second_tool_round_reuses_the_conversation() {
    let mut h = Harness::new([
        Script::Stream({
            let mut steps = tool_call("call-1", "list_dir", "{\"path\":\".\"}");
            steps.insert(steps.len() - 1, Step::Event(usage(1)));
            steps
        }),
        Script::Stream({
            let mut steps = tool_call("call-2", "grep", "{\"pattern\":\"needle\"}");
            steps.insert(steps.len() - 1, Step::Event(usage(1)));
            steps
        }),
        done("final"),
    ]);
    let workspace = h.directory.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("hay.txt"), "a needle here\n").unwrap();
    let session = h.session(&workspace).await;
    accepted(h.command(send(&session, "in-1")).await);
    let events = h.until_finished().await;

    assert_eq!(proposed(&events).len(), 2);
    let states = state_changes(&events);
    assert_eq!(states.len(), 4);
    assert_eq!(states[3].1, ToolCallState::Succeeded);
    assert_eq!(
        states[3].2.as_ref().unwrap().text,
        "hay.txt:1:a needle here"
    );
    assert_eq!(*outcome(&events), RunOutcome::Succeeded);

    let requests = h.provider.requests();
    assert_eq!(requests.len(), 3);
    let messages = &requests[2].messages;
    // user, assistant, tool-1, assistant, tool-2
    let tail = &messages[messages.len() - 5..];
    assert_eq!(tail[0].role, MessageRole::User);
    assert_eq!(tail[1].role, MessageRole::Assistant);
    assert_eq!(tail[1].tool_calls.len(), 1);
    assert_eq!(tail[1].tool_calls[0].id, ToolCallId("call-1".into()));
    assert_eq!(tail[2].tool_call_id, Some(ToolCallId("call-1".into())));
    assert_eq!(tail[3].role, MessageRole::Assistant);
    assert_eq!(tail[3].tool_calls.len(), 1);
    assert_eq!(tail[3].tool_calls[0].id, ToolCallId("call-2".into()));
    assert_eq!(tail[4].tool_call_id, Some(ToolCallId("call-2".into())));
    assert_eq!(tail[4].content.text, "hay.txt:1:a needle here");

    // Usage accumulates across every provider request of the run.
    let run = h.provider.requests()[0].run.clone();
    let stored = h.runtime.services.store.run(&run).unwrap().unwrap();
    assert_eq!(stored.usage.requests, 3);
    assert_eq!(stored.usage.input_tokens, 3);
}

#[tokio::test]
async fn provider_stream_error_mid_tool_call_fails_the_run() {
    let mut h = Harness::new([Script::Stream(vec![
        Step::Event(StreamEvent::ToolCallStarted {
            id: ToolCallId("call-1".into()),
            name: "read_file".into(),
        }),
        Step::Error(ProviderError {
            code: None,
            kind: ErrorKind::Network,
            message: "dropped".into(),
        }),
    ])]);
    let workspace = h.directory.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    let session = h.session(&workspace).await;
    accepted(h.command(send(&session, "in-1")).await);
    let events = h.until_finished().await;

    // A partial tool call is dropped with the response: no proposals, no result.
    assert!(proposed(&events).is_empty());
    assert!(state_changes(&events).is_empty());
    assert!(matches!(outcome(&events), RunOutcome::Failed { .. }));
}

#[tokio::test]
async fn malformed_provider_stream_fails_the_run() {
    let mut h = Harness::new([Script::Stream(vec![
        Step::Event(StreamEvent::ToolCallStarted {
            id: ToolCallId("call-1".into()),
            name: "read_file".into(),
        }),
        Step::Event(StreamEvent::ToolCallStarted {
            id: ToolCallId("call-1".into()),
            name: "read_file".into(),
        }),
        Step::Event(StreamEvent::Finished),
    ])]);
    let workspace = h.directory.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    let session = h.session(&workspace).await;
    accepted(h.command(send(&session, "in-1")).await);
    let events = h.until_finished().await;

    assert!(proposed(&events).is_empty());
    assert!(matches!(outcome(&events), RunOutcome::Failed { .. }));
}
