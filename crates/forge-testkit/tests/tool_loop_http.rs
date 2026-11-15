mod common;
use forge_core::{
    Runtime, RuntimeHandle, RuntimeServices,
    secrets::{MemorySecretStore, SecretStore},
};
use forge_protocol::*;
use forge_provider::{ANTHROPIC_VERSION, HttpProviderFactory};
use forge_store::Store;
use forge_testkit::http::{MockResponse, MockServer, RecordedRequest};
use forge_tools::clock::FakeClock;
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

const KEY: &str = "sentinel-runtime-key-never-persist";
const ANTHROPIC_CALLS: &str = include_str!("fixtures/anthropic_tool_calls.sse");
const ANTHROPIC_MISSING: &str = include_str!("fixtures/anthropic_tool_missing.sse");
const ANTHROPIC_FINAL: &str = include_str!("fixtures/anthropic_final.sse");
const OPENAI_CALLS: &str = include_str!("fixtures/openai_tool_calls.sse");
const OPENAI_FINAL: &str = include_str!("fixtures/openai_final.sse");
const TOOL_NAMES: [&str; 4] = ["read_file", "list_dir", "glob", "grep"];

struct Harness {
    runtime: Runtime,
    handle: RuntimeHandle,
    events: mpsc::UnboundedReceiver<EventEnvelope>,
    secrets: Arc<MemorySecretStore>,
    directory: tempfile::TempDir,
    name: String,
}
impl Harness {
    fn new(name: &str) -> Self {
        let clock = Arc::new(FakeClock::default());
        clock.advance(Duration::from_secs(100));
        let secrets = Arc::new(MemorySecretStore::default());
        secrets.set(&name.into(), KEY).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let (runtime, handle) = Runtime::new(
            RuntimeServices {
                clock,
                store: Store::open(directory.path()).unwrap(),
                providers: Arc::new(HttpProviderFactory),
                secrets: secrets.clone(),
                initial_settings: common::settings(name),
            },
            0,
        )
        .unwrap();
        let events = handle.subscribe();
        Self {
            runtime,
            handle,
            events,
            secrets,
            directory,
            name: name.into(),
        }
    }
    /// Drop the runtime and reopen the same store, as a process restart would.
    fn reopen(self) -> Self {
        let Self {
            runtime,
            handle,
            events,
            secrets,
            directory,
            name,
        } = self;
        drop(runtime);
        drop(handle);
        drop(events);
        let clock = Arc::new(FakeClock::default());
        clock.advance(Duration::from_secs(100));
        let (runtime, handle) = Runtime::new(
            RuntimeServices {
                clock,
                store: Store::open(directory.path()).unwrap(),
                providers: Arc::new(HttpProviderFactory),
                secrets: secrets.clone(),
                initial_settings: common::settings(&name),
            },
            0,
        )
        .unwrap();
        let events = handle.subscribe();
        Self {
            runtime,
            handle,
            events,
            secrets,
            directory,
            name,
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
    async fn session(&mut self, workspace: &Path, endpoint: &str) -> SessionId {
        let mut settings = common::settings(&self.name);
        settings.providers[0].endpoint = Some(endpoint.into());
        accepted(
            self.command(Command::UpdateSettings {
                settings: settings.clone(),
            })
            .await,
        );
        let workspace = accepted(
            self.command(Command::AddWorkspace {
                path: workspace.to_owned(),
            })
            .await,
        );
        SessionId(accepted(
            self.command(Command::CreateSession {
                workspace: WorkspaceId(workspace),
                title: "HTTP".into(),
                model: settings.model,
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
fn send(session: &SessionId, input: &str, text: &str) -> Command {
    Command::Send {
        session: session.clone(),
        input_id: input.into(),
        content: Content {
            text: text.into(),
            attachments: vec![],
        },
    }
}
async fn requests(server: &mut MockServer, count: usize) -> Vec<RecordedRequest> {
    let mut out = Vec::new();
    for _ in 0..count {
        out.push(server.requests.recv().await.unwrap());
    }
    out
}
fn body(request: &RecordedRequest) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}
fn succeeded(events: &[EventEnvelope]) -> bool {
    events.iter().any(|envelope| {
        matches!(
            envelope.event,
            Event::RunFinished {
                outcome: RunOutcome::Succeeded,
                ..
            }
        )
    })
}
/// The first request carries only the user prompt and the advertised
/// read-only tools; assertions are identical for both providers.
fn first_request_assertions(name: &str, request: &RecordedRequest) {
    let body = body(request);
    assert_eq!(body["model"], "fixture-model");
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["messages"],
        json!([{"role":"user","content":"run the tools"}])
    );
    let names: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            if name == "anthropic" {
                tool["name"].as_str().unwrap()
            } else {
                tool["function"]["name"].as_str().unwrap()
            }
        })
        .collect();
    assert_eq!(names, TOOL_NAMES);
    if name == "anthropic" {
        assert_eq!(request.path, "/v1/messages");
        assert_eq!(request.headers["x-api-key"], KEY);
        assert_eq!(request.headers["anthropic-version"], ANTHROPIC_VERSION);
        assert_eq!(body["max_tokens"], 128);
    } else {
        assert_eq!(request.path, "/v1/chat/completions");
        assert_eq!(request.headers["authorization"], format!("Bearer {KEY}"));
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["max_completion_tokens"], 128);
    }
}
/// Run the two-request tool loop and return the recorded requests.
async fn run_pair(name: &str, first: &str, second: &str) -> Vec<RecordedRequest> {
    let mut server =
        MockServer::start_all(vec![MockResponse::sse(first), MockResponse::sse(second)]).await;
    let mut h = Harness::new(name);
    let workspace = h.directory.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("note.txt"), "file-body\n").unwrap();
    let session = h.session(&workspace, &server.endpoint).await;
    accepted(h.command(send(&session, "in-1", "run the tools")).await);
    let events = h.until_finished().await;
    assert!(succeeded(&events));
    let requests = requests(&mut server, 2).await;
    server.task.await.unwrap().unwrap();
    requests
}

#[tokio::test]
async fn anthropic_continuation_encodes_tool_use_and_tool_result_blocks() {
    let requests = run_pair("anthropic", ANTHROPIC_CALLS, ANTHROPIC_FINAL).await;
    first_request_assertions("anthropic", &requests[0]);
    // The continuation must pair each tool_use block with a tool_result block
    // inside a user message — the shape Anthropic rejects otherwise.
    assert_eq!(
        body(&requests[1])["messages"],
        json!([
            {"role":"user","content":"run the tools"},
            {"role":"assistant","content":[
                {"type":"text","text":"Let me look."},
                {"type":"tool_use","id":"call_read","name":"read_file","input":{"path":"note.txt"}},
                {"type":"tool_use","id":"call_list","name":"list_dir","input":{"path":"."}}
            ]},
            {"role":"user","content":[
                {"type":"tool_result","tool_use_id":"call_read","content":"file-body\n"},
                {"type":"tool_result","tool_use_id":"call_list","content":"note.txt"}
            ]}
        ])
    );
    assert_eq!(body(&requests[1])["tools"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn anthropic_failed_tool_result_sets_is_error() {
    let requests = run_pair("anthropic", ANTHROPIC_MISSING, ANTHROPIC_FINAL).await;
    // No text preceded the tool_use, so the assistant content is the call alone.
    let messages = body(&requests[1])["messages"].clone();
    assert_eq!(
        messages[1],
        json!({"role":"assistant","content":[
            {"type":"tool_use","id":"call_gone","name":"read_file","input":{"path":"missing.txt"}}
        ]})
    );
    let results = messages[2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["type"], "tool_result");
    assert_eq!(results[0]["tool_use_id"], "call_gone");
    assert_eq!(results[0]["is_error"], true);
    assert!(!results[0]["content"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn openai_continuation_encodes_assistant_tool_calls_and_tool_messages() {
    let requests = run_pair("openai", OPENAI_CALLS, OPENAI_FINAL).await;
    first_request_assertions("openai", &requests[0]);
    // The continuation must carry the calls on the assistant message and one
    // role:tool message per result, each keyed by its exact tool_call_id.
    assert_eq!(
        body(&requests[1])["messages"],
        json!([
            {"role":"user","content":"run the tools"},
            {"role":"assistant","content":"Let me look.","tool_calls":[
                {"id":"call_read","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"note.txt\"}"}},
                {"id":"call_list","type":"function","function":{"name":"list_dir","arguments":"{\"path\":\".\"}"}}
            ]},
            {"role":"tool","tool_call_id":"call_read","content":"file-body\n"},
            {"role":"tool","tool_call_id":"call_list","content":"note.txt"}
        ])
    );
}

#[tokio::test]
async fn reopen_replays_completed_tool_context_for_both_providers() {
    for (name, calls, expected) in [
        (
            "anthropic",
            ANTHROPIC_CALLS,
            json!([
                {"role":"user","content":"run the tools"},
                {"role":"assistant","content":[
                    {"type":"text","text":"Let me look."},
                    {"type":"tool_use","id":"call_read","name":"read_file","input":{"path":"note.txt"}},
                    {"type":"tool_use","id":"call_list","name":"list_dir","input":{"path":"."}}
                ]},
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"call_read","content":"file-body\n"},
                    {"type":"tool_result","tool_use_id":"call_list","content":"note.txt"}
                ]},
                {"role":"assistant","content":"The file says hi."},
                {"role":"user","content":"and again"}
            ]),
        ),
        (
            "openai",
            OPENAI_CALLS,
            json!([
                {"role":"user","content":"run the tools"},
                {"role":"assistant","content":"Let me look.","tool_calls":[
                    {"id":"call_read","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"note.txt\"}"}},
                    {"id":"call_list","type":"function","function":{"name":"list_dir","arguments":"{\"path\":\".\"}"}}
                ]},
                {"role":"tool","tool_call_id":"call_read","content":"file-body\n"},
                {"role":"tool","tool_call_id":"call_list","content":"note.txt"},
                {"role":"assistant","content":"The file says hi."},
                {"role":"user","content":"and again"}
            ]),
        ),
    ] {
        let second = if name == "anthropic" {
            ANTHROPIC_FINAL
        } else {
            OPENAI_FINAL
        };
        let mut server = MockServer::start_all(vec![
            MockResponse::sse(calls),
            MockResponse::sse(second),
            MockResponse::sse(second),
        ])
        .await;
        let mut h = Harness::new(name);
        let workspace = h.directory.path().join("ws");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("note.txt"), "file-body\n").unwrap();
        let session = h.session(&workspace, &server.endpoint).await;
        accepted(h.command(send(&session, "in-1", "run the tools")).await);
        assert!(succeeded(&h.until_finished().await));
        // Restart against the same store; the next send must rebuild the whole
        // tool context from persisted rows, not from process memory.
        let mut h = h.reopen();
        accepted(h.command(send(&session, "in-2", "and again")).await);
        assert!(succeeded(&h.until_finished().await));
        let requests = requests(&mut server, 3).await;
        server.task.await.unwrap().unwrap();
        assert_eq!(body(&requests[2])["messages"], expected);
    }
}
