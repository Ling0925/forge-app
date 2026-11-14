mod common;
use forge_core::{
    Runtime, RuntimeHandle, RuntimeServices,
    secrets::{MemorySecretStore, SecretStore, SecretStoreError},
};
use forge_protocol::*;
use forge_provider::{ErrorKind, ProviderError, RetryAfter, StreamEvent};
use forge_store::Store;
use forge_testkit::{
    provider::{FixedProviderFactory, Script, ScriptedProvider, Step},
    recorder::EventRecorder,
};
use forge_tools::clock::{Clock, FakeClock};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
use tokio::sync::mpsc;

const KEY: &str = "sentinel-runtime-key-never-persist";
struct Harness {
    runtime: Runtime,
    handle: RuntimeHandle,
    events: mpsc::UnboundedReceiver<EventEnvelope>,
    clock: Arc<FakeClock>,
    provider: Arc<ScriptedProvider>,
    secrets: Arc<MemorySecretStore>,
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
                clock: clock.clone(),
                store: Store::open(directory.path()).unwrap(),
                providers: Arc::new(FixedProviderFactory(provider.clone())),
                secrets: secrets.clone(),
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
            clock,
            provider,
            secrets,
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
    async fn session(&mut self) -> SessionId {
        let root = self.directory.path().to_owned();
        let workspace = accepted(self.command(Command::AddWorkspace { path: root }).await);
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
    async fn http_session(&mut self, name: &str, endpoint: &str, key: Option<&str>) -> SessionId {
        self.runtime.services.providers = Arc::new(forge_provider::HttpProviderFactory);
        let mut settings = common::settings(name);
        settings.providers[0].endpoint = Some(endpoint.into());
        accepted(
            self.command(Command::UpdateSettings {
                settings: settings.clone(),
            })
            .await,
        );
        accepted(
            self.command(Command::SetProviderKey {
                provider: name.into(),
                key: key.map(str::to_owned),
            })
            .await,
        );
        let workspace = accepted(
            self.command(Command::AddWorkspace {
                path: self.directory.path().to_owned(),
            })
            .await,
        );
        SessionId(accepted(
            self.command(Command::CreateSession {
                workspace: WorkspaceId(workspace),
                title: "HTTP errors".into(),
                model: settings.model,
                permission_mode: PermissionMode::Ask,
            })
            .await,
        ))
    }
    async fn finished(&mut self) -> RunId {
        loop {
            if let Event::RunFinished { run, .. } = self.next().await.event {
                return run;
            }
        }
    }
    fn reopen(self) -> Self {
        self.reopen_at(0)
    }
    fn reopen_at(self, last_seq: u64) -> Self {
        let Self {
            runtime,
            handle,
            events,
            clock,
            provider,
            secrets,
            directory,
        } = self;
        drop(runtime);
        drop(handle);
        drop(events);
        let (runtime, handle) = Runtime::new(
            RuntimeServices {
                clock: clock.clone(),
                store: Store::open(directory.path()).unwrap(),
                providers: Arc::new(FixedProviderFactory(provider.clone())),
                secrets: secrets.clone(),
                initial_settings: common::settings("script"),
            },
            last_seq,
        )
        .unwrap();
        let events = handle.subscribe();
        Self {
            runtime,
            handle,
            events,
            clock,
            provider,
            secrets,
            directory,
        }
    }
}
fn accepted(result: CommandResult) -> String {
    match result {
        CommandResult::Accepted { id } => id.0,
        other => panic!("{other:?}"),
    }
}
fn content(text: &str) -> Content {
    Content {
        text: text.into(),
        attachments: vec![],
    }
}
fn send(session: &SessionId, input: &str, text: &str) -> Command {
    Command::Send {
        session: session.clone(),
        input_id: input.into(),
        content: content(text),
    }
}
fn success(text: &str) -> Script {
    Script::Stream(vec![
        Step::Event(StreamEvent::TextDelta(text.into())),
        Step::Event(StreamEvent::Usage(Usage {
            input_tokens: 2,
            output_tokens: 3,
            cached_tokens: 1,
            requests: 1,
            estimated_cost_usd: None,
        })),
        Step::Event(StreamEvent::Finished),
    ])
}
fn rejection(result: CommandResult, expected: RejectCode) {
    assert!(matches!(result,CommandResult::Rejected {code,..} if code==expected));
}

#[tokio::test]
async fn workspace_session_settings_and_key_crud_persist_before_events() {
    let mut h = Harness::new([]);
    let s = h.session().await;
    let reader = Store::open(h.directory.path()).unwrap();
    let workspace = reader.workspaces().unwrap().remove(0);
    assert_eq!(workspace.root, h.directory.path().canonicalize().unwrap());
    assert_eq!(
        workspace.name,
        workspace.root.file_name().unwrap().to_str().unwrap()
    );
    assert_eq!(
        h.next().await.event,
        Event::WorkspaceAdded {
            workspace: workspace.clone()
        }
    );
    let stored = reader.session(&s).unwrap().unwrap();
    assert_eq!(stored.created_at, 100_000);
    assert_eq!(stored.updated_at, 100_000);
    assert_eq!(
        h.next().await.event,
        Event::SessionCreated { session: stored }
    );
    assert_eq!(
        accepted(
            h.command(Command::AddWorkspace {
                path: workspace.root.join(".")
            })
            .await
        ),
        workspace.id.0
    );
    assert!(h.events.try_recv().is_err());
    rejection(
        h.command(Command::RemoveWorkspace {
            workspace: workspace.id.clone(),
        })
        .await,
        RejectCode::InvalidState,
    );
    h.clock.advance(Duration::from_secs(1));
    accepted(
        h.command(Command::RenameSession {
            session: s.clone(),
            title: "renamed".into(),
        })
        .await,
    );
    let stored = reader.session(&s).unwrap().unwrap();
    assert_eq!(stored.title, "renamed");
    assert_eq!(stored.updated_at, 101_000);
    assert_eq!(
        h.next().await.event,
        Event::SessionUpdated { session: stored }
    );
    for archived in [true, false] {
        accepted(
            h.command(Command::ArchiveSession {
                session: s.clone(),
                archived,
            })
            .await,
        );
        assert_eq!(reader.session(&s).unwrap().unwrap().archived, archived);
        assert!(matches!(h.next().await.event, Event::SessionUpdated { .. }));
        assert_eq!(
            h.next().await.event,
            Event::SessionArchived {
                session: s.clone(),
                archived
            }
        );
    }
    let mut settings = common::settings("script");
    settings
        .ui_prefs
        .insert("onboarding_complete".into(), "yes".into());
    accepted(
        h.command(Command::UpdateSettings {
            settings: settings.clone(),
        })
        .await,
    );
    assert_eq!(reader.settings().unwrap(), Some(settings.clone()));
    assert_eq!(
        h.next().await.event,
        Event::SettingsUpdated {
            settings: settings.clone()
        }
    );
    for (key, key_status) in [
        (None, KeyStatus::Missing),
        (Some(KEY.to_owned()), KeyStatus::Configured),
        (Some(String::new()), KeyStatus::Missing),
    ] {
        accepted(
            h.command(Command::SetProviderKey {
                provider: "script".into(),
                key: key.clone(),
            })
            .await,
        );
        assert_eq!(h.secrets.get(&"script".into()).unwrap(), key);
        let event = h.next().await.event;
        assert_eq!(
            event,
            Event::ProviderKeyConfigured {
                provider: "script".into(),
                key_status: key_status.clone()
            }
        );
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({"type":"provider_key_configured", "provider":"script", "key_status":key_status})
        );
        accepted(h.command(Command::GetSnapshot).await);
        let envelope = h.next().await;
        let Event::Snapshot {
            providers,
            last_seq,
            settings: actual,
            ..
        } = envelope.event
        else {
            panic!()
        };
        assert_eq!(last_seq, envelope.seq);
        assert_eq!(actual, settings);
        assert_eq!(
            providers,
            vec![ProviderKeyStatus {
                provider: "script".into(),
                key_status
            }]
        );
    }
    let mut h = h.reopen();
    assert_eq!(h.runtime.services.store.settings().unwrap(), Some(settings));
    accepted(
        h.command(Command::DeleteSession { session: s.clone() })
            .await,
    );
    assert_eq!(reader.session(&s).unwrap(), None);
    assert_eq!(h.next().await.event, Event::SessionDeleted { session: s });
    accepted(
        h.command(Command::RemoveWorkspace {
            workspace: workspace.id.clone(),
        })
        .await,
    );
    assert!(reader.workspaces().unwrap().is_empty());
    assert_eq!(
        h.next().await.event,
        Event::WorkspaceRemoved {
            workspace: workspace.id
        }
    );
}

#[tokio::test]
async fn send_commits_before_acceptance_finishes_atomically_and_replays_after_reopen() {
    let mut h = Harness::new([success("answer"), success("next")]);
    let s = h.session().await;
    while h.events.try_recv().is_ok() {}
    h.clock.advance(Duration::from_secs(1));
    assert_eq!(
        accepted(h.command(send(&s, "input", "hello")).await),
        "input"
    );
    let reader = Store::open(h.directory.path()).unwrap();
    let user = reader.input(&"input".into()).unwrap().unwrap();
    let run = reader.run(&user.run_id).unwrap().unwrap();
    assert_eq!(run.started_at, 101_000);
    assert_eq!(run.finished_at, None);
    assert_eq!(user.content, content("hello"));
    assert_eq!(
        reader.session(&s).unwrap().unwrap().state,
        SessionState::Running
    );
    assert_eq!(
        h.next().await.event,
        Event::RunStarted {
            run: run.clone(),
            input_id: "input".into()
        }
    );
    assert!(matches!(h.next().await.event, Event::SessionUpdated { .. }));
    h.clock.advance(Duration::from_secs(1));
    let mut recorder = EventRecorder::default();
    loop {
        let envelope = h.next().await;
        assert!(!matches!(
            envelope.event,
            Event::QueueItemAdded { .. } | Event::QueueItemUpdated { .. }
        ));
        if let Event::RunFinished {
            outcome,
            finished_at,
            ..
        } = &envelope.event
        {
            assert_eq!(outcome, &RunOutcome::Succeeded);
            let run = reader.run(&run.id).unwrap().unwrap();
            assert_eq!(run.finished_at, Some(102_000));
            assert_eq!(run.finished_at, Some(*finished_at));
            assert_eq!(run.usage.output_tokens, 3);
            let session = reader.session(&s).unwrap().unwrap();
            assert_eq!(session.state, SessionState::Idle);
            let finished = session.last_finished_run.as_ref().unwrap();
            assert_eq!(finished.started_at, run.started_at);
            assert_eq!(finished.finished_at, *finished_at);
            assert_eq!(
                recorder.events().last().unwrap().event,
                Event::SessionUpdated { session }
            );
            let history = reader.messages(&s).unwrap();
            assert_eq!(history.len(), 2);
            assert_eq!(history[1].content.text, "answer");
            recorder.record(envelope);
            break;
        }
        recorder.record(envelope);
    }
    recorder.assert_run_finished_once(&run.id);
    let last = recorder.events().last().unwrap().seq;
    accepted(h.command(Command::GetSnapshot).await);
    assert_eq!(h.next().await.seq, last + 1);
    let mut h = h.reopen();
    accepted(h.command(Command::GetSnapshot).await);
    assert_eq!(h.next().await.seq, last + 1);
    assert_eq!(
        accepted(h.command(send(&s, "input", "hello")).await),
        "input"
    );
    assert!(h.events.try_recv().is_err());
    rejection(
        h.command(send(&s, "input", "different")).await,
        RejectCode::Conflict,
    );
    assert_eq!(reader.messages(&s).unwrap().len(), 2);
    assert_eq!(h.provider.requests().len(), 1);
    accepted(h.command(send(&s, "second", "more")).await);
    assert!(h.next().await.seq > last);
    h.finished().await;
    assert_eq!(
        h.provider.requests()[1]
            .messages
            .iter()
            .map(|m| m.content.text.as_str())
            .collect::<Vec<_>>(),
        ["hello", "answer", "more"]
    );
    accepted(
        h.command(Command::DeleteSession { session: s.clone() })
            .await,
    );
    assert!(reader.messages(&s).unwrap().is_empty());
    assert!(reader.run(&run.id).unwrap().is_none());
}

#[tokio::test]
async fn history_uses_persisted_message_ordinals_in_oldest_to_newest_pages() {
    let mut h = Harness::new([success("a"), success("b"), success("c")]);
    let s = h.session().await;
    for id in ["one", "two", "three"] {
        accepted(h.command(send(&s, id, id)).await);
        h.finished().await;
    }
    let mut h = h.reopen();
    let all = h.runtime.services.store.messages(&s).unwrap();
    assert_eq!(all.len(), 6);
    assert!(all.windows(2).all(|w| w[0].ordinal < w[1].ordinal));
    let mut before = None;
    for expected in [all[4..6].to_vec(), all[2..4].to_vec(), all[0..2].to_vec()] {
        accepted(
            h.command(Command::LoadHistory {
                session: s.clone(),
                before,
                limit: 2,
            })
            .await,
        );
        let Event::HistoryLoaded {
            session,
            messages,
            next_before,
            runs,
        } = h.next().await.event
        else {
            panic!()
        };
        assert_eq!(session, s);
        assert_eq!(messages, expected);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, messages[0].run_id);
        assert_eq!(runs[0].outcome, Some(RunOutcome::Succeeded));
        before = next_before;
        if expected[0].ordinal != all[0].ordinal {
            assert_eq!(before, Some(expected[0].ordinal));
        } else {
            assert_eq!(before, None);
        }
    }
    assert_eq!(
        h.runtime
            .services
            .store
            .history(&s, Some(u64::MAX), 2)
            .unwrap()
            .0,
        all[4..6]
    );
    assert!(
        h.runtime
            .services
            .store
            .history(&s, Some(all[0].ordinal), 2)
            .unwrap()
            .0
            .is_empty()
    );
    assert_eq!(
        h.runtime.services.store.history(&s, None, 0).unwrap(),
        (vec![], None)
    );
    let other = h.session().await;
    assert!(
        h.runtime
            .services
            .store
            .history(&other, None, 2)
            .unwrap()
            .0
            .is_empty()
    );
}

#[derive(Clone, Debug, PartialEq)]
struct ViewRun {
    session: SessionId,
    input_id: InputId,
    text: String,
    reasoning: String,
    outcome: Option<RunOutcome>,
    usage: Usage,
}
#[derive(Default, Debug, PartialEq)]
struct ClientState {
    last_seq: u64,
    sessions: BTreeMap<SessionId, Session>,
    runs: BTreeMap<RunId, ViewRun>,
}
impl ClientState {
    fn apply(&mut self, envelope: &EventEnvelope) -> bool {
        if let Event::Snapshot {
            sessions,
            active_runs,
            last_seq,
            ..
        } = &envelope.event
        {
            assert_eq!(*last_seq, envelope.seq);
            self.sessions = sessions
                .iter()
                .cloned()
                .map(|s| (s.id.clone(), s))
                .collect();
            self.runs.retain(|_, run| run.outcome.is_some());
            for active in active_runs {
                self.runs.insert(
                    active.run.id.clone(),
                    ViewRun {
                        session: active.run.session_id.clone(),
                        input_id: active.input_id.clone(),
                        text: active.text.clone(),
                        reasoning: active.reasoning.clone(),
                        outcome: None,
                        usage: active.run.usage.clone(),
                    },
                );
            }
            self.last_seq = *last_seq;
            return true;
        }
        if envelope.seq <= self.last_seq {
            return true;
        }
        if envelope.seq != self.last_seq + 1 {
            return false;
        }
        self.last_seq = envelope.seq;
        match &envelope.event {
            Event::SessionCreated { session } | Event::SessionUpdated { session } => {
                self.sessions.insert(session.id.clone(), session.clone());
            }
            Event::RunStarted { run, input_id } => {
                self.runs.insert(
                    run.id.clone(),
                    ViewRun {
                        session: run.session_id.clone(),
                        input_id: input_id.clone(),
                        text: String::new(),
                        reasoning: String::new(),
                        outcome: None,
                        usage: run.usage.clone(),
                    },
                );
            }
            Event::TextDelta { run, text } => self.runs.get_mut(run).unwrap().text.push_str(text),
            Event::ReasoningDelta { run, text } => {
                self.runs.get_mut(run).unwrap().reasoning.push_str(text)
            }
            Event::UsageRecorded { record } => {
                self.runs.get_mut(&record.run_id).unwrap().usage = record.usage.clone()
            }
            Event::RunFinished { run, outcome, .. } => {
                self.runs.get_mut(run).unwrap().outcome = Some(outcome.clone());
            }
            _ => {}
        }
        true
    }
}
async fn resync(drop_event: bool) {
    let mut h = Harness::new([Script::Stream(vec![
        Step::Event(StreamEvent::TextDelta("hel".into())),
        Step::Event(StreamEvent::ReasoningDelta("why".into())),
        Step::Delay(Duration::from_secs(10)),
        Step::Event(StreamEvent::TextDelta("lo".into())),
        Step::Event(StreamEvent::ReasoningDelta("?".into())),
        Step::Event(StreamEvent::Finished),
    ])]);
    let s = h.session().await;
    let mut uninterrupted = ClientState::default();
    while let Ok(event) = h.events.try_recv() {
        assert!(uninterrupted.apply(&event));
    }
    let mut client = ClientState::default();
    let mut sink = EventRecorder::dropping_seq(uninterrupted.last_seq + 3);
    if drop_event {
        client.sessions = uninterrupted.sessions.clone();
        client.last_seq = uninterrupted.last_seq;
    }
    accepted(h.command(send(&s, "input", "hello")).await);
    let mut gap = false;
    loop {
        let event = h.next().await;
        assert!(uninterrupted.apply(&event));
        if drop_event && let Some(event) = sink.record(event.clone()) {
            gap |= !client.apply(&event);
        }
        if matches!(event.event, Event::ReasoningDelta { .. }) {
            break;
        }
    }
    if drop_event {
        assert!(gap);
    }
    assert_eq!(
        accepted(h.command(send(&s, "input", "hello")).await),
        "input"
    );
    rejection(
        h.command(send(&s, "input", "changed")).await,
        RejectCode::Conflict,
    );
    rejection(
        h.command(send(&s, "busy", "other")).await,
        RejectCode::InvalidState,
    );
    rejection(
        h.command(Command::DeleteSession { session: s.clone() })
            .await,
        RejectCode::InvalidState,
    );
    accepted(h.command(Command::GetSnapshot).await);
    let snapshot = h.next().await;
    assert!(
        matches!(&snapshot.event,Event::Snapshot {active_runs,..} if active_runs.len()==1 && active_runs[0].text=="hel" && active_runs[0].reasoning=="why")
    );
    assert!(client.apply(&snapshot));
    assert_eq!(snapshot.seq, uninterrupted.last_seq + 1);
    uninterrupted.last_seq = snapshot.seq;
    let boundary = client.last_seq;
    // Replayed envelopes through the snapshot boundary must not append the prefix twice.
    for event in sink.events() {
        assert!(client.apply(event));
    }
    let second = h.session().await;
    assert_ne!(s, second);
    while let Ok(event) = h.events.try_recv() {
        assert!(client.apply(&event));
        assert!(uninterrupted.apply(&event));
    }
    h.clock.advance(Duration::from_secs(10));
    loop {
        let event = h.next().await;
        assert!(event.seq > boundary);
        assert!(client.apply(&event));
        assert!(uninterrupted.apply(&event));
        if matches!(event.event, Event::RunFinished { .. }) {
            break;
        }
    }
    assert_eq!(client, uninterrupted);
    assert_eq!(
        client.sessions,
        h.runtime
            .services
            .store
            .sessions()
            .unwrap()
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect()
    );
    let view = client.runs.values().next().unwrap();
    assert_eq!(view.text, "hello");
    assert_eq!(view.reasoning, "why?");
    let history = h.runtime.services.store.messages(&s).unwrap();
    assert_eq!(history[1].content.text, view.text);
    accepted(h.command(Command::GetSnapshot).await);
    assert!(
        matches!(h.next().await.event,Event::Snapshot {active_runs,..} if active_runs.is_empty())
    );
}
#[tokio::test]
async fn late_client_mid_stream_snapshot_matches_uninterrupted_client() {
    resync(false).await;
}
#[tokio::test]
async fn dropped_delta_gap_requests_snapshot_and_recovers_exact_state() {
    resync(true).await;
}

#[tokio::test]
async fn failed_stream_discards_partial_message_clears_active_buffer_and_never_leaks_key() {
    let mut h = Harness::new([
        Script::Stream(vec![
            Step::Event(StreamEvent::TextDelta("partial".into())),
            Step::Error(ProviderError {
                code: None,
                kind: ErrorKind::Network,
                message: format!("failure with {KEY}"),
            }),
        ]),
        success("fresh"),
    ]);
    let s = h.session().await;
    accepted(h.command(send(&s, "failed", "hello")).await);
    let mut events = vec![];
    loop {
        let e = h.next().await;
        let finished = matches!(e.event, Event::RunFinished { .. });
        events.push(e);
        if finished {
            break;
        }
    }
    let history = h.runtime.services.store.messages(&s).unwrap();
    assert_eq!(history.len(), 1);
    let run = h
        .runtime
        .services
        .store
        .run(&history[0].run_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        run.outcome,
        Some(RunOutcome::Failed {
            kind: RunErrorKind::Network,
            retryable: true,
            ..
        })
    ));
    assert!(!serde_json::to_string(&events).unwrap().contains(KEY));
    assert!(!format!("{run:?}").contains(KEY));
    accepted(h.command(Command::GetSnapshot).await);
    assert!(
        matches!(h.next().await.event,Event::Snapshot {active_runs,..} if active_runs.is_empty())
    );
    accepted(h.command(send(&s, "fresh", "again")).await);
    h.finished().await;
    assert_eq!(
        h.runtime.services.store.messages(&s).unwrap()[2]
            .content
            .text,
        "fresh"
    );
    assert_eq!(h.provider.requests().len(), 2);
    for entry in std::fs::read_dir(h.directory.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            assert!(
                !std::fs::read(path)
                    .unwrap()
                    .windows(KEY.len())
                    .any(|bytes| bytes == KEY.as_bytes())
            );
        }
    }
}

#[tokio::test]
async fn provider_failures_map_structured_outcomes_and_retry_after_using_clock() {
    for (kind, code, expected, retry_after_ms) in [
        (
            ErrorKind::Http {
                status: 429,
                retry_after: Some(RetryAfter::Delay(Duration::from_secs(120))),
            },
            None,
            RunErrorKind::RateLimited,
            Some(120_000),
        ),
        (
            ErrorKind::Http {
                status: 429,
                retry_after: Some(RetryAfter::At(UNIX_EPOCH + Duration::from_secs(103))),
            },
            None,
            RunErrorKind::RateLimited,
            Some(3_000),
        ),
        (
            ErrorKind::Http {
                status: 503,
                retry_after: Some(RetryAfter::At(UNIX_EPOCH + Duration::from_secs(99))),
            },
            None,
            RunErrorKind::Server,
            Some(0),
        ),
        (ErrorKind::Network, None, RunErrorKind::Network, None),
        (ErrorKind::Disconnected, None, RunErrorKind::Network, None),
        (ErrorKind::InvalidResponse, None, RunErrorKind::Server, None),
        (ErrorKind::Configuration, None, RunErrorKind::Auth, None),
        (
            ErrorKind::Http {
                status: 429,
                retry_after: None,
            },
            Some("insufficient_quota"),
            RunErrorKind::Quota,
            None,
        ),
    ] {
        let error = ProviderError {
            kind: kind.clone(),
            code: code.map(str::to_owned),
            message: "scripted".into(),
        };
        let retryable = error.retryable();
        let mut h = Harness::new([Script::Error(error)]);
        let s = h.session().await;
        accepted(h.command(send(&s, "input", "hi")).await);
        let id = h.finished().await;
        let run = h.runtime.services.store.run(&id).unwrap().unwrap();
        let http_status = match kind {
            ErrorKind::Http { status, .. } => Some(status),
            _ => None,
        };
        assert_eq!(
            run.outcome,
            Some(RunOutcome::Failed {
                kind: expected,
                message: "scripted".into(),
                http_status,
                retry_after_ms,
                retryable
            })
        );
        assert_eq!(h.provider.requests().len(), 1);
        assert_eq!(h.clock.now(), Duration::from_secs(100));
    }
}

#[tokio::test]
async fn key_testing_uses_http_factory_and_maps_errors_without_exposing_secrets() {
    use forge_provider::HttpProviderFactory;
    use forge_testkit::http::{MockResponse, MockServer};
    for (status, body, expected) in [
        (200, include_str!("fixtures/openai.sse"), None),
        (401, "unauthorized", Some(ProviderKeyErrorKind::Auth)),
        (403, "forbidden", Some(ProviderKeyErrorKind::Auth)),
        (402, "payment", Some(ProviderKeyErrorKind::Quota)),
        (
            429,
            r#"{"error":{"code":"insufficient_quota","type":"rate_limit_error","message":"credits exhausted"}}"#,
            Some(ProviderKeyErrorKind::Quota),
        ),
        (429, "rate", Some(ProviderKeyErrorKind::RateLimited)),
        (503, "server", Some(ProviderKeyErrorKind::Server)),
        (400, "bad request", Some(ProviderKeyErrorKind::Provider)),
    ] {
        let mut response = MockResponse::sse(body);
        response.status = status;
        let server = MockServer::start(response).await;
        let mut h = Harness::new([]);
        h.runtime.services.providers = Arc::new(HttpProviderFactory);
        let mut settings = common::settings("openai");
        settings.providers[0].endpoint = Some(server.endpoint.clone());
        accepted(h.command(Command::UpdateSettings { settings }).await);
        accepted(
            h.command(Command::SetProviderKey {
                provider: "openai".into(),
                key: Some(KEY.into()),
            })
            .await,
        );
        while h.events.try_recv().is_ok() {}
        accepted(
            h.command(Command::TestProviderKey {
                provider: "openai".into(),
            })
            .await,
        );
        assert_eq!(
            h.next().await.event,
            Event::ProviderKeyTested {
                provider: "openai".into(),
                ok: expected.is_none(),
                error_kind: expected
            }
        );
        let request = server.request.await.unwrap();
        server.task.await.unwrap().unwrap();
        assert_eq!(request.headers["authorization"], format!("Bearer {KEY}"));
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["max_completion_tokens"], 1);
        assert_eq!(body["model"], "fixture-model");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }
    for (endpoint, key, expected) in [
        ("", Some(KEY), ProviderKeyErrorKind::Configuration),
        (
            "http://127.0.0.1:0",
            Some(KEY),
            ProviderKeyErrorKind::Network,
        ),
        (
            "http://127.0.0.1:0",
            None,
            ProviderKeyErrorKind::Configuration,
        ),
    ] {
        let mut h = Harness::new([]);
        h.runtime.services.providers = Arc::new(HttpProviderFactory);
        let mut settings = common::settings("openai");
        settings.providers[0].endpoint = Some(endpoint.into());
        accepted(h.command(Command::UpdateSettings { settings }).await);
        accepted(
            h.command(Command::SetProviderKey {
                provider: "openai".into(),
                key: key.map(str::to_owned),
            })
            .await,
        );
        while h.events.try_recv().is_ok() {}
        accepted(
            h.command(Command::TestProviderKey {
                provider: "openai".into(),
            })
            .await,
        );
        assert_eq!(
            h.next().await.event,
            Event::ProviderKeyTested {
                provider: "openai".into(),
                ok: false,
                error_kind: Some(expected)
            }
        );
    }
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn send_crash_before_and_after_commit_obeys_r1_and_reopen_deduplication() {
    use forge_tools::failpoints::{Failpoints, SEND_AFTER_COMMIT, SEND_BEFORE_COMMIT};
    for point in [SEND_BEFORE_COMMIT, SEND_AFTER_COMMIT] {
        let mut h = Harness::new([]);
        let session = h.session().await;
        while h.events.try_recv().is_ok() {}
        h.runtime.failpoints = Failpoints::new([point]);
        let Harness {
            mut runtime,
            handle,
            mut events,
            clock,
            provider,
            secrets,
            directory,
        } = h;
        let task = tokio::spawn(async move {
            loop {
                runtime.dispatch_next().await.unwrap();
            }
        });
        assert!(
            handle
                .execute(send(&session, "input", "hello"))
                .await
                .is_err()
        );
        let panic = task.await.unwrap_err();
        assert!(panic.is_panic());
        assert!(events.try_recv().is_err());
        let store = Store::open(directory.path()).unwrap();
        let input = store.input(&"input".into()).unwrap();
        assert_eq!(input.is_some(), point == SEND_AFTER_COMMIT);
        let count: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, if point == SEND_AFTER_COMMIT { 1 } else { 0 });
        if point == SEND_AFTER_COMMIT {
            let (mut runtime, handle) = Runtime::new(
                RuntimeServices {
                    clock,
                    store,
                    providers: Arc::new(FixedProviderFactory(provider)),
                    secrets,
                    initial_settings: common::settings("script"),
                },
                0,
            )
            .unwrap();
            let mut events = handle.subscribe();
            let (result, dispatch) = tokio::join!(
                handle.execute(send(&session, "input", "hello")),
                runtime.dispatch_next()
            );
            assert!(dispatch.unwrap());
            assert_eq!(accepted(result.unwrap()), "input");
            assert!(events.try_recv().is_err());
            assert_eq!(runtime.services.store.messages(&session).unwrap().len(), 1);
        }
    }
}
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn crash_before_final_commit_rolls_back_assistant_usage_and_terminal_state() {
    use forge_tools::failpoints::{Failpoints, RUN_BEFORE_FINAL_COMMIT};
    let mut h = Harness::new([success("answer")]);
    let session = h.session().await;
    h.runtime.failpoints = Failpoints::new([RUN_BEFORE_FINAL_COMMIT]);
    let Harness {
        mut runtime,
        handle,
        mut events,
        directory,
        ..
    } = h;
    let task = tokio::spawn(async move {
        loop {
            runtime.dispatch_next().await.unwrap();
        }
    });
    assert_eq!(
        accepted(
            handle
                .execute(send(&session, "input", "hello"))
                .await
                .unwrap()
        ),
        "input"
    );
    assert!(task.await.unwrap_err().is_panic());
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(
            event.event,
            Event::RunFinished { .. } | Event::UsageRecorded { .. }
        ));
    }
    let store = Store::open(directory.path()).unwrap();
    let messages = store.messages(&session).unwrap();
    assert_eq!(messages.len(), 1);
    let run = store.run(&messages[0].run_id).unwrap().unwrap();
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.outcome, None);
    assert_eq!(run.finished_at, None);
    let count: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM usage_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
#[cfg(all(feature = "failpoints", unix))]
#[test]
fn abort_failpoint_terminates_child_with_sigabrt() {
    use forge_tools::failpoints::{Action, Failpoints, SEND_BEFORE_COMMIT};
    use std::os::unix::process::ExitStatusExt;
    const CHILD: &str = "FORGE_P1_ABORT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        Failpoints::with_actions([(SEND_BEFORE_COMMIT, Action::Abort)]).hit(SEND_BEFORE_COMMIT);
        unreachable!();
    }
    let directory = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "abort_failpoint_terminates_child_with_sigabrt"])
        .env(CHILD, "1")
        .current_dir(directory.path())
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGABRT));
}

#[tokio::test]
async fn http_chat_persists_both_providers_and_key_tests_use_anthropic_minimal_request() {
    use forge_provider::HttpProviderFactory;
    use forge_testkit::http::{MockResponse, MockServer};
    for (name, body) in [
        ("anthropic", include_str!("fixtures/anthropic.sse")),
        ("openai", include_str!("fixtures/openai.sse")),
    ] {
        // The fixture ends with tool calls, so the run issues a follow-up
        // request; serve a terminal no-tool response to finish it.
        let follow_up = match name {
            "anthropic" => {
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_follow\",\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
            }
            _ => {
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0}}\n\ndata: [DONE]\n\n"
            }
        };
        let server =
            MockServer::start_all(vec![MockResponse::sse(body), MockResponse::sse(follow_up)])
                .await;
        let mut h = Harness::new([]);
        h.runtime.services.providers = Arc::new(HttpProviderFactory);
        let mut settings = common::settings(name);
        settings.providers[0].endpoint = Some(server.endpoint.clone());
        accepted(
            h.command(Command::UpdateSettings {
                settings: settings.clone(),
            })
            .await,
        );
        accepted(
            h.command(Command::SetProviderKey {
                provider: name.into(),
                key: Some(KEY.into()),
            })
            .await,
        );
        let workspace = accepted(
            h.command(Command::AddWorkspace {
                path: h.directory.path().to_owned(),
            })
            .await,
        );
        let session = SessionId(accepted(
            h.command(Command::CreateSession {
                workspace: WorkspaceId(workspace),
                title: "HTTP chat".into(),
                model: settings.model,
                permission_mode: PermissionMode::Ask,
            })
            .await,
        ));
        accepted(h.command(send(&session, "input", "hello")).await);
        let run = h.finished().await;
        let stored = h.runtime.services.store.run(&run).unwrap().unwrap();
        assert_eq!(stored.outcome, Some(RunOutcome::Succeeded));
        assert_eq!(stored.usage.input_tokens, 12);
        assert_eq!(stored.usage.output_tokens, 9);
        assert_eq!(
            h.runtime.services.store.messages(&session).unwrap()[1]
                .content
                .text,
            "Hello 世界"
        );
        server.request.await.unwrap();
        server.task.await.unwrap().unwrap();
    }
    let server = MockServer::start(MockResponse::sse(include_str!("fixtures/anthropic.sse"))).await;
    let mut h = Harness::new([]);
    h.runtime.services.providers = Arc::new(HttpProviderFactory);
    let mut settings = common::settings("anthropic");
    settings.providers[0].endpoint = Some(server.endpoint.clone());
    settings.providers[0].max_output_tokens = None;
    accepted(h.command(Command::UpdateSettings { settings }).await);
    accepted(
        h.command(Command::SetProviderKey {
            provider: "anthropic".into(),
            key: Some(KEY.into()),
        })
        .await,
    );
    while h.events.try_recv().is_ok() {}
    accepted(
        h.command(Command::TestProviderKey {
            provider: "anthropic".into(),
        })
        .await,
    );
    assert_eq!(
        h.next().await.event,
        Event::ProviderKeyTested {
            provider: "anthropic".into(),
            ok: true,
            error_kind: None
        }
    );
    let request = server.request.await.unwrap();
    server.task.await.unwrap().unwrap();
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["max_tokens"], 1);
}

#[tokio::test]
async fn simultaneous_runs_keep_stream_buffers_and_usage_separate() {
    let mut h = Harness::new([
        Script::Stream(vec![
            Step::Event(StreamEvent::TextDelta("first".into())),
            Step::Delay(Duration::from_secs(10)),
            Step::Event(StreamEvent::TextDelta(" done".into())),
            Step::Event(StreamEvent::Finished),
        ]),
        Script::Stream(vec![
            Step::Event(StreamEvent::TextDelta("second".into())),
            Step::Delay(Duration::from_secs(5)),
            Step::Event(StreamEvent::Finished),
        ]),
    ]);
    let first = h.session().await;
    let second = h.session().await;
    accepted(h.command(send(&first, "first", "one")).await);
    loop {
        if matches!(h.next().await.event, Event::TextDelta { .. }) {
            break;
        }
    }
    accepted(h.command(send(&second, "second", "two")).await);
    loop {
        if matches!(h.next().await.event, Event::TextDelta { .. }) {
            break;
        }
    }
    accepted(h.command(Command::GetSnapshot).await);
    let Event::Snapshot { active_runs, .. } = h.next().await.event else {
        panic!()
    };
    assert_eq!(active_runs.len(), 2);
    assert!(
        active_runs
            .iter()
            .any(|a| a.run.session_id == first && a.text == "first")
    );
    assert!(
        active_runs
            .iter()
            .any(|a| a.run.session_id == second && a.text == "second")
    );
    h.clock.advance(Duration::from_secs(5));
    h.finished().await;
    accepted(h.command(Command::GetSnapshot).await);
    assert!(
        matches!(h.next().await.event,Event::Snapshot {active_runs,..} if active_runs.len()==1 && active_runs[0].run.session_id==first && active_runs[0].text=="first")
    );
    h.clock.advance(Duration::from_secs(5));
    h.finished().await;
    assert_eq!(
        h.runtime.services.store.messages(&first).unwrap()[1]
            .content
            .text,
        "first done"
    );
    assert_eq!(
        h.runtime.services.store.messages(&second).unwrap()[1]
            .content
            .text,
        "second"
    );
}

#[tokio::test]
async fn send_storage_failure_rejects_and_rolls_back_the_whole_transaction() {
    let mut h = Harness::new([success("answer")]);
    let session = h.session().await;
    while h.events.try_recv().is_ok() {}
    h.runtime.services.store.connection().execute_batch("CREATE TRIGGER fail_message BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'write failed'); END;").unwrap();
    rejection(
        h.command(send(&session, "input", "hello")).await,
        RejectCode::Storage,
    );
    assert!(h.events.try_recv().is_err());
    assert!(
        h.runtime
            .services
            .store
            .input(&"input".into())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        h.runtime
            .services
            .store
            .session(&session)
            .unwrap()
            .unwrap()
            .state,
        SessionState::Idle
    );
    let runs: i64 = h
        .runtime
        .services
        .store
        .connection()
        .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(runs, 0);
    assert!(h.provider.requests().is_empty());
    h.runtime
        .services
        .store
        .connection()
        .execute_batch("DROP TRIGGER fail_message")
        .unwrap();
    accepted(h.command(send(&session, "input", "hello")).await);
    h.finished().await;
}

#[tokio::test]
async fn run_storage_failure_persists_failure_and_redacts_the_runs_key() {
    for key in [KEY, "sentinel-\"escaped\\key"] {
        let mut h = Harness::new([Script::Stream(vec![
            Step::Event(StreamEvent::TextDelta("partial".into())),
            Step::Delay(Duration::from_secs(1)),
            Step::Event(StreamEvent::Finished),
        ])]);
        h.secrets.set(&"script".into(), key).unwrap();
        let session = h.session().await;
        accepted(h.command(send(&session, "input", "hello")).await);
        let mut events = Vec::new();
        loop {
            let event = h.next().await;
            let partial = matches!(event.event, Event::TextDelta { .. });
            events.push(event);
            if partial {
                break;
            }
        }
        accepted(
            h.command(Command::SetProviderKey {
                provider: "script".into(),
                key: Some("replacement-key".into()),
            })
            .await,
        );
        accepted(
            h.command(Command::RenameSession {
                session: session.clone(),
                title: "Renamed while streaming".into(),
            })
            .await,
        );
        let encoded = serde_json::to_string(key).unwrap();
        let message = format!(
            "save failed\nAuthorization: Bearer {key}\nx-api-key: {key}\necho: {key}\nencoded: {}\ndetails survive",
            &encoded[1..encoded.len() - 1]
        );
        h.runtime.services.store.connection().execute_batch(&format!(
            "CREATE TRIGGER fail_message BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'{}'); END;", message.replace('\'', "''")
        )).unwrap();
        h.clock.advance(Duration::from_secs(1));
        let (run, outcome, finished_at) = loop {
            let event = h.next().await;
            events.push(event.clone());
            if let Event::RunFinished {
                run,
                outcome,
                finished_at,
            } = event.event
            {
                break (run, outcome, finished_at);
            }
        };
        let expected = RunOutcome::Failed {
            kind: RunErrorKind::Unknown {
                source: UnknownSource::Storage,
            },
            message: "save failed\n[已移除请求头]\n[已移除请求头]\necho: [redacted]\nencoded: [redacted]\ndetails survive".into(),
            http_status: None,
            retry_after_ms: None,
            retryable: false,
        };
        assert_eq!(outcome, expected);
        accepted(h.command(Command::GetSnapshot).await);
        let snapshot = h.next().await;
        assert!(
            matches!(&snapshot.event, Event::Snapshot { active_runs, sessions, last_seq, .. }
            if active_runs.is_empty() && sessions[0].state == SessionState::Idle
                && sessions[0].title == "Renamed while streaming" && *last_seq == snapshot.seq)
        );
        events.push(snapshot);
        assert!(events.windows(2).all(|pair| pair[1].seq == pair[0].seq + 1));
        assert!(
            !serde_json::to_string(&events)
                .unwrap()
                .contains(&encoded[1..encoded.len() - 1])
        );
        let h = h.reopen();
        let stored = h.runtime.services.store.run(&run).unwrap().unwrap();
        assert_eq!(stored.state, RunState::Failed);
        assert_eq!(stored.outcome, Some(expected));
        assert_eq!(stored.finished_at, Some(101_000));
        assert_eq!(stored.finished_at, Some(finished_at));
        let finished = h
            .runtime
            .services
            .store
            .session(&session)
            .unwrap()
            .unwrap()
            .last_finished_run
            .unwrap();
        assert_eq!(finished.started_at, stored.started_at);
        assert_eq!(finished.finished_at, finished_at);
        assert!(events.iter().any(|event| matches!(&event.event, Event::SessionUpdated { session } if session.last_finished_run.as_ref() == Some(&finished))));
        let messages = h.runtime.services.store.messages(&session).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, MessageRole::User);
    }
}

#[tokio::test]
async fn run_storage_failure_keeps_serving_when_recovery_writes_fail() {
    for read_only in [false, true] {
        let mut h = Harness::new([
            Script::Stream(vec![
                Step::Event(StreamEvent::TextDelta("partial".into())),
                Step::Delay(Duration::from_secs(1)),
                Step::Event(StreamEvent::Finished),
            ]),
            success("next run"),
        ]);
        let session = h.session().await;
        accepted(h.command(send(&session, "input", "hello")).await);
        let mut events = Vec::new();
        loop {
            let event = h.next().await;
            let partial = matches!(event.event, Event::TextDelta { .. });
            events.push(event);
            if partial {
                break;
            }
        }
        h.runtime.services.store.connection().execute_batch(if read_only {
            "PRAGMA query_only = ON"
        } else {
            "CREATE TRIGGER fail_run BEFORE UPDATE ON runs BEGIN SELECT RAISE(ABORT,'run write failed'); END;"
        }).unwrap();
        h.clock.advance(Duration::from_secs(1));
        loop {
            let event = h.next().await;
            events.push(event.clone());
            if let Event::RunFinished {
                outcome,
                finished_at,
                ..
            } = event.event
            {
                assert_eq!(finished_at, h.clock.now().as_millis() as u64);
                let RunOutcome::Failed {
                    kind,
                    message,
                    http_status,
                    retry_after_ms,
                    retryable,
                } = outcome
                else {
                    panic!()
                };
                assert_eq!(
                    kind,
                    RunErrorKind::Unknown {
                        source: UnknownSource::Storage
                    }
                );
                assert_eq!(
                    message,
                    if read_only {
                        "attempt to write a readonly database"
                    } else {
                        "run write failed"
                    }
                );
                assert_eq!(http_status, None);
                assert_eq!(retry_after_ms, None);
                assert!(!retryable);
                break;
            }
        }
        accepted(h.command(Command::GetSnapshot).await);
        let snapshot = h.next().await;
        assert!(
            matches!(&snapshot.event, Event::Snapshot { active_runs, sessions, last_seq, .. }
            if active_runs.is_empty() && *last_seq == snapshot.seq
                && sessions[0].state == if read_only { SessionState::Running } else { SessionState::Idle })
        );
        events.push(snapshot);
        assert!(events.windows(2).all(|pair| pair[1].seq == pair[0].seq + 1));
        assert_eq!(
            h.runtime.services.store.messages(&session).unwrap().len(),
            1
        );
        h.runtime
            .services
            .store
            .connection()
            .execute_batch(if read_only {
                "PRAGMA query_only = OFF"
            } else {
                "DROP TRIGGER fail_run"
            })
            .unwrap();
        if !read_only {
            accepted(h.command(send(&session, "next-input", "hello again")).await);
            let run = h.finished().await;
            assert_eq!(
                h.runtime.services.store.run(&run).unwrap().unwrap().outcome,
                Some(RunOutcome::Succeeded)
            );
        }
    }
}

#[tokio::test]
async fn key_test_without_a_configured_model_returns_configuration_and_makes_no_request() {
    let mut h = Harness::new([]);
    accepted(
        h.command(Command::SetProviderKey {
            provider: "other".into(),
            key: Some(KEY.into()),
        })
        .await,
    );
    while h.events.try_recv().is_ok() {}
    accepted(
        h.command(Command::TestProviderKey {
            provider: "other".into(),
        })
        .await,
    );
    assert_eq!(
        h.next().await.event,
        Event::ProviderKeyTested {
            provider: "other".into(),
            ok: false,
            error_kind: Some(ProviderKeyErrorKind::Configuration)
        }
    );
    assert!(h.provider.requests().is_empty());
}

#[tokio::test]
async fn ephemeral_events_skip_transactions_and_event_rows() {
    let mut h = Harness::new([
        Script::Stream(vec![
            Step::Event(StreamEvent::TextDelta("partial".into())),
            Step::Event(StreamEvent::ReasoningDelta("reason".into())),
            Step::Delay(Duration::from_secs(10)),
            Step::Event(StreamEvent::Finished),
        ]),
        Script::Stream(vec![Step::Event(StreamEvent::Finished)]),
    ]);
    let session = h.session().await;
    while h.events.try_recv().is_ok() {}
    accepted(h.command(send(&session, "input", "hello")).await);
    let persisted = h.runtime.services.store.last_seq().unwrap();
    let changes = h.runtime.services.store.connection().total_changes();
    h.runtime
        .services
        .store
        .connection()
        .execute_batch("BEGIN")
        .unwrap();
    let mut seen = Vec::new();
    loop {
        let event = h.next().await;
        let reasoning = matches!(event.event, Event::ReasoningDelta { .. });
        seen.push(event);
        if reasoning {
            break;
        }
    }
    assert!(
        seen.iter()
            .any(|e| matches!(e.event, Event::TextDelta { .. }))
    );
    accepted(
        h.command(Command::LoadHistory {
            session,
            before: None,
            limit: 10,
        })
        .await,
    );
    let history = h.next().await;
    assert!(matches!(&history.event, Event::HistoryLoaded { runs, .. }
        if runs.len() == 1 && runs[0].state == RunState::Running && runs[0].outcome.is_none() && runs[0].finished_at.is_none()));
    seen.push(history);
    accepted(h.command(Command::GetSnapshot).await);
    let snapshot = h.next().await;
    assert!(
        matches!(&snapshot.event, Event::Snapshot { active_runs, last_seq, .. } if *last_seq == snapshot.seq && active_runs[0].text == "partial" && active_runs[0].reasoning == "reason")
    );
    seen.push(snapshot);
    for provider in ["script", "unconfigured"] {
        accepted(
            h.command(Command::TestProviderKey {
                provider: provider.into(),
            })
            .await,
        );
        let tested = h.next().await;
        assert!(matches!(tested.event, Event::ProviderKeyTested { .. }));
        seen.push(tested);
    }
    assert!(seen.windows(2).all(|pair| pair[1].seq == pair[0].seq + 1));
    assert_eq!(h.runtime.services.store.last_seq().unwrap(), persisted);
    assert_eq!(
        h.runtime.services.store.connection().total_changes(),
        changes
    );
    assert!(!h.runtime.services.store.connection().is_autocommit());
    h.runtime
        .services
        .store
        .connection()
        .execute_batch("ROLLBACK")
        .unwrap();
    h.clock.advance(Duration::from_secs(10));
    h.finished().await;
    let ephemeral_rows: i64 = h.runtime.services.store.connection().query_row(
        "SELECT count(*) FROM events WHERE json_extract(event_json, '$.type') IN ('text_delta', 'reasoning_delta', 'history_loaded', 'snapshot', 'provider_key_tested')",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(ephemeral_rows, 0);
}

#[tokio::test]
async fn restart_sequence_uses_caller_or_persisted_seq_and_starts_with_fresh_snapshot() {
    for caller_seq in [0, 100] {
        let mut h = Harness::new([]);
        h.session().await;
        while h.events.try_recv().is_ok() {}
        let persisted = h.runtime.services.store.last_seq().unwrap();
        for offset in [1, 2] {
            accepted(h.command(Command::GetSnapshot).await);
            assert_eq!(h.next().await.seq, persisted + offset);
        }
        assert_eq!(h.runtime.services.store.last_seq().unwrap(), persisted);
        let mut h = h.reopen_at(caller_seq);
        accepted(h.command(Command::GetSnapshot).await);
        let snapshot = h.next().await;
        assert_eq!(snapshot.seq, caller_seq.max(persisted) + 1);
        assert!(
            matches!(snapshot.event, Event::Snapshot { last_seq, .. } if last_seq == snapshot.seq)
        );
    }
}

#[tokio::test]
async fn run_errors_use_recorded_codes_in_http_bodies_and_stream_events() {
    use forge_testkit::http::{MockResponse, MockServer};
    use serde_json::{Value, json};
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/provider_errors.json")).unwrap();
    for fixture in fixtures {
        let name = fixture["provider"].as_str().unwrap();
        for status in [400, 200] {
            let mut payload = json!({"type":"error", "error":fixture["error"]});
            payload["error"]["message"] = json!(format!(
                "{} {KEY}",
                payload["error"]["message"].as_str().unwrap()
            ));
            let body = if status == 200 {
                format!("event: error\ndata: {payload}\n\n")
            } else {
                payload.to_string()
            };
            let mut response = MockResponse::sse(&body);
            response.status = status;
            let server = MockServer::start(response).await;
            let mut h = Harness::new([]);
            let session = h.http_session(name, &server.endpoint, Some(KEY)).await;
            accepted(h.command(send(&session, "input", "hello")).await);
            loop {
                let envelope = h.next().await;
                assert!(!serde_json::to_string(&envelope).unwrap().contains(KEY));
                if let Event::RunFinished {
                    run,
                    outcome,
                    finished_at,
                } = envelope.event
                {
                    let stored = h.runtime.services.store.run(&run).unwrap().unwrap();
                    assert_eq!(stored.outcome, Some(outcome.clone()));
                    assert_eq!(stored.finished_at, Some(finished_at));
                    let RunOutcome::Failed {
                        kind,
                        message,
                        http_status,
                        retry_after_ms,
                        retryable,
                    } = outcome
                    else {
                        panic!()
                    };
                    assert_eq!(
                        kind,
                        serde_json::from_value::<RunErrorKind>(
                            fixture[if status == 200 { "stream_kind" } else { "kind" }].clone()
                        )
                        .unwrap(),
                        "{fixture}"
                    );
                    assert!(message.contains("[redacted]"));
                    assert_eq!(http_status, (status != 200).then_some(status));
                    assert_eq!(retry_after_ms, None);
                    assert_eq!(
                        retryable,
                        status == 200 && fixture["stream_retryable"].as_bool().unwrap()
                    );
                    break;
                }
            }
            server.request.await.unwrap();
            server.task.await.unwrap().unwrap();
        }
    }
}

#[tokio::test]
async fn run_http_status_mapping_does_not_inspect_error_wording() {
    use forge_testkit::http::{MockResponse, MockServer};
    for name in ["anthropic", "openai"] {
        for (status, expected) in [
            (401, RunErrorKind::Auth),
            (403, RunErrorKind::Auth),
            (402, RunErrorKind::Quota),
            (429, RunErrorKind::RateLimited),
            (503, RunErrorKind::Server),
            (
                400,
                RunErrorKind::Unknown {
                    source: UnknownSource::Provider,
                },
            ),
        ] {
            let mut response = MockResponse::sse(
                "authentication_error quota billing_error rate_limit_error overloaded_error context_length_exceeded prompt is too long",
            );
            response.status = status;
            let server = MockServer::start(response).await;
            let mut h = Harness::new([]);
            let session = h.http_session(name, &server.endpoint, Some(KEY)).await;
            accepted(h.command(send(&session, "input", "hello")).await);
            let run = h.finished().await;
            let outcome = h
                .runtime
                .services
                .store
                .run(&run)
                .unwrap()
                .unwrap()
                .outcome
                .unwrap();
            assert!(
                matches!(outcome, RunOutcome::Failed { kind, http_status: Some(actual), retryable, .. } if kind == expected && actual == status && retryable == matches!(status, 429 | 500..=599))
            );
            server.request.await.unwrap();
            server.task.await.unwrap().unwrap();
        }
    }
}

#[tokio::test]
async fn run_transport_and_configuration_errors_use_network_and_auth_cards() {
    use forge_testkit::http::{MockResponse, MockServer};
    for name in ["anthropic", "openai"] {
        for (endpoint, key, expected, retryable) in [
            ("", Some(KEY), RunErrorKind::Auth, false),
            ("http://127.0.0.1:0", None, RunErrorKind::Auth, false),
            ("http://127.0.0.1:0", Some(KEY), RunErrorKind::Network, true),
        ] {
            let mut h = Harness::new([]);
            let session = h.http_session(name, endpoint, key).await;
            accepted(h.command(send(&session, "input", "hello")).await);
            let run = h.finished().await;
            let outcome = h
                .runtime
                .services
                .store
                .run(&run)
                .unwrap()
                .unwrap()
                .outcome
                .unwrap();
            assert!(
                matches!(outcome, RunOutcome::Failed { kind, http_status: None, retryable: actual, .. } if kind == expected && actual == retryable)
            );
        }
        for (body, expected, retryable) in [
            ("data: {bad}\n\n", RunErrorKind::Server, true),
            (": disconnected\n\n", RunErrorKind::Network, true),
        ] {
            let server = MockServer::start(MockResponse::sse(body)).await;
            let mut h = Harness::new([]);
            let session = h.http_session(name, &server.endpoint, Some(KEY)).await;
            accepted(h.command(send(&session, "input", "hello")).await);
            let (run, outcome) = loop {
                if let Event::RunFinished { run, outcome, .. } = h.next().await.event {
                    break (run, outcome);
                }
            };
            assert_eq!(
                h.runtime.services.store.run(&run).unwrap().unwrap().outcome,
                Some(outcome.clone())
            );
            assert!(
                matches!(outcome, RunOutcome::Failed { kind, http_status: None, retryable: actual, .. } if kind == expected && actual == retryable)
            );
            server.request.await.unwrap();
            server.task.await.unwrap().unwrap();
        }
    }
}

#[tokio::test]
async fn run_failure_messages_redact_header_segments_before_publish_and_persistence() {
    use forge_testkit::http::{MockResponse, MockServer};
    for name in ["anthropic", "openai"] {
        for newline in ["\n", "\r\n", r"\n", r"\r\n"] {
            for embedded_key in [false, true] {
                let mut lines = vec![
                    "provider rejected request".to_owned(),
                    format!("Authorization: Bearer {KEY}"),
                    format!("  pRoXy-AuThOrIzAtIoN = {KEY}"),
                    format!("x-api-key: {KEY}"),
                    format!("api-key: {KEY}"),
                    "{\"AuThOrIzAtIoN\": \"already masked\"}".into(),
                    "  \"x-API-key\" = \"[redacted]\"".into(),
                    format!("request headers: API-KEY = {KEY}"),
                    "trace = available".into(),
                ];
                let mut expected = vec![
                    "provider rejected request",
                    "[已移除请求头]",
                    "  [已移除请求头]",
                    "[已移除请求头]",
                    "[已移除请求头]",
                    "{\"[已移除请求头]",
                    "  \"[已移除请求头]",
                    "request headers: [已移除请求头]",
                    "trace = available",
                ];
                if embedded_key {
                    lines.push(format!("upstream echo: {KEY}"));
                    expected.push("upstream echo: [redacted]");
                }
                lines.push("Retry details remain.".into());
                expected.push("Retry details remain.");
                let body = lines.join(newline);
                let expected = expected.join(newline);
                let mut response = MockResponse::sse(&body);
                response.status = 401;
                let server = MockServer::start(response).await;
                let mut h = Harness::new([]);
                let session = h.http_session(name, &server.endpoint, Some(KEY)).await;
                accepted(h.command(send(&session, "input", "hello")).await);
                let (run, outcome) = loop {
                    if let Event::RunFinished { run, outcome, .. } = h.next().await.event {
                        break (run, outcome);
                    }
                };
                server.request.await.unwrap();
                server.task.await.unwrap().unwrap();
                let h = h.reopen();
                let persisted = h
                    .runtime
                    .services
                    .store
                    .run(&run)
                    .unwrap()
                    .unwrap()
                    .outcome
                    .unwrap();
                assert_eq!(outcome, persisted);
                for outcome in [outcome, persisted] {
                    let RunOutcome::Failed { message, .. } = outcome else {
                        panic!()
                    };
                    assert_eq!(message, expected);
                    assert!(!message.contains(KEY));
                    let lower = message.to_ascii_lowercase();
                    for header in [
                        "authorization",
                        "proxy-authorization",
                        "x-api-key",
                        "api-key",
                    ] {
                        assert!(!lower.contains(header));
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn json_401_headers_never_reach_events_sqlite_or_wal() {
    use forge_testkit::http::{MockResponse, MockServer};
    const PROXY_TOKEN: &str = "sentinel-proxy-credential-never-persist";
    const API_KEY: &str = "sentinel-echoed-api-key-never-persist";
    for provider in ["openai", "anthropic"] {
        let body = format!(
            r#"{{"error":{{"type":"authentication_error","message":"invalid key\nProxy-Authorization: Basic {PROXY_TOKEN}\nx-api-key: {API_KEY}"}}}}"#
        );
        let mut response = MockResponse::sse(&body);
        response.status = 401;
        let server = MockServer::start(response).await;
        let mut h = Harness::new([]);
        let session = h.http_session(provider, &server.endpoint, Some(KEY)).await;
        accepted(h.command(send(&session, "input", "hello")).await);
        let mut events = Vec::new();
        loop {
            let envelope = h.next().await;
            let finished = matches!(envelope.event, Event::RunFinished { .. });
            events.push(envelope);
            if finished {
                break;
            }
        }
        server.request.await.unwrap();
        server.task.await.unwrap().unwrap();
        accepted(h.command(Command::GetSnapshot).await);
        events.push(h.next().await);
        accepted(
            h.command(Command::LoadHistory {
                session,
                before: None,
                limit: 1,
            })
            .await,
        );
        events.push(h.next().await);
        let serialized = serde_json::to_string(&events).unwrap();
        for secret in [KEY, PROXY_TOKEN, API_KEY] {
            assert!(!serialized.contains(secret));
        }
        let Event::RunFinished {
            outcome: RunOutcome::Failed { message, .. },
            ..
        } = &events[events.len() - 3].event
        else {
            panic!()
        };
        assert!(message.contains(r#""message":"invalid key\n[已移除请求头]\n[已移除请求头]"#));
        let lower = message.to_ascii_lowercase();
        for header in ["proxy-authorization", "x-api-key"] {
            assert!(!lower.contains(header));
        }
        for file in ["forge.sqlite", "forge.sqlite-wal"] {
            let bytes = std::fs::read(h.directory.path().join(file)).unwrap();
            for secret in [KEY, PROXY_TOKEN, API_KEY] {
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes()),
                    "{file}: {secret}"
                );
            }
        }
        let h = h.reopen();
        let bytes = std::fs::read(h.directory.path().join("forge.sqlite")).unwrap();
        for secret in [KEY, PROXY_TOKEN, API_KEY] {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes())
            );
        }
    }
}

#[tokio::test]
async fn snapshot_reports_key_states_and_retries_unavailable_reads() {
    struct FailingReadStore {
        keys: Arc<MemorySecretStore>,
        provider: ProviderId,
    }
    impl SecretStore for FailingReadStore {
        fn get(&self, provider: &ProviderId) -> Result<Option<String>, SecretStoreError> {
            if provider == &self.provider {
                Err(SecretStoreError)
            } else {
                self.keys.get(provider)
            }
        }
        fn set(&self, provider: &ProviderId, key: &str) -> Result<(), SecretStoreError> {
            self.keys.set(provider, key)
        }
        fn delete(&self, provider: &ProviderId) -> Result<(), SecretStoreError> {
            self.keys.delete(provider)
        }
    }

    let mut h = Harness::new([]);
    let mut settings = common::settings("script");
    for provider in ["missing", "empty", "unavailable"] {
        let mut entry = settings.providers[0].clone();
        entry.id = provider.into();
        settings.providers.push(entry);
    }
    h.secrets.set(&"empty".into(), "").unwrap();
    h.secrets.set(&"unavailable".into(), KEY).unwrap();
    h.runtime.services.secrets = Arc::new(FailingReadStore {
        keys: h.secrets.clone(),
        provider: "unavailable".into(),
    });
    accepted(
        h.command(Command::UpdateSettings {
            settings: settings.clone(),
        })
        .await,
    );
    assert!(matches!(
        h.next().await.event,
        Event::SettingsUpdated { .. }
    ));

    for status in ["unavailable", "configured"] {
        accepted(h.command(Command::GetSnapshot).await);
        let envelope = h.next().await;
        let Event::Snapshot {
            providers,
            settings: actual,
            last_seq,
            ..
        } = envelope.event
        else {
            panic!()
        };
        assert_eq!(actual, settings);
        assert_eq!(last_seq, envelope.seq);
        assert_eq!(
            serde_json::to_value(providers).unwrap(),
            serde_json::json!([
                {"provider":"script", "key_status":"configured"},
                {"provider":"missing", "key_status":"missing"},
                {"provider":"empty", "key_status":"missing"},
                {"provider":"unavailable", "key_status":status}
            ])
        );
        h.runtime.services.secrets = h.secrets.clone();
    }
}

#[tokio::test]
async fn failed_outcomes_survive_restart_and_dropped_event_resync() {
    use forge_testkit::http::{MockResponse, MockServer};
    for restart in [false, true] {
        let mut response = MockResponse::sse(r#"{"error":{"message":"invalid key"}}"#);
        response.status = 401;
        let server = MockServer::start(response).await;
        let mut h = Harness::new([]);
        let session = h.http_session("openai", &server.endpoint, Some(KEY)).await;
        accepted(h.command(send(&session, "input", "hello")).await);
        let run_id = h
            .runtime
            .services
            .store
            .input(&"input".into())
            .unwrap()
            .unwrap()
            .run_id;
        let run = loop {
            let run = h.runtime.services.store.run(&run_id).unwrap().unwrap();
            if run.outcome.is_some() {
                break run;
            }
            h.runtime.dispatch_next().await.unwrap();
        };
        server.request.await.unwrap();
        server.task.await.unwrap().unwrap();
        assert!(matches!(
            run.outcome,
            Some(RunOutcome::Failed {
                kind: RunErrorKind::Auth,
                http_status: Some(401),
                retryable: false,
                ..
            })
        ));
        h.events = h.handle.subscribe();
        if restart {
            h = h.reopen();
        }
        accepted(h.command(Command::GetSnapshot).await);
        let snapshot = h.next().await;
        let Event::Snapshot {
            sessions,
            active_runs,
            last_seq,
            ..
        } = snapshot.event
        else {
            panic!()
        };
        assert_eq!(last_seq, snapshot.seq);
        assert!(active_runs.is_empty());
        assert_eq!(sessions[0].id, session);
        assert_eq!(sessions[0].state, SessionState::Idle);
        assert_eq!(
            sessions[0].last_finished_run,
            Some(FinishedRun {
                run: run_id,
                input_id: Some("input".into()),
                outcome: run.outcome.clone().unwrap(),
                started_at: run.started_at,
                finished_at: run.finished_at.unwrap(),
            })
        );
        accepted(
            h.command(Command::LoadHistory {
                session,
                before: None,
                limit: 1,
            })
            .await,
        );
        let Event::HistoryLoaded { messages, runs, .. } = h.next().await.event else {
            panic!()
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].run_id, run.id);
        assert_eq!(runs, vec![run]);
    }
}

#[tokio::test]
async fn snapshot_keeps_latest_finished_run_while_a_new_run_is_active() {
    let mut h = Harness::new([
        success("first"),
        success("second"),
        Script::Stream(vec![
            Step::Delay(Duration::from_secs(10)),
            Step::Event(StreamEvent::Finished),
        ]),
    ]);
    let session = h.session().await;
    accepted(h.command(Command::GetSnapshot).await);
    loop {
        if let Event::Snapshot { sessions, .. } = h.next().await.event {
            assert_eq!(sessions[0].last_finished_run, None);
            break;
        }
    }
    accepted(h.command(send(&session, "first", "hello")).await);
    h.finished().await;
    accepted(h.command(send(&session, "second", "hello again")).await);
    let second = h.finished().await;
    accepted(h.command(send(&session, "third", "still running")).await);
    accepted(h.command(Command::GetSnapshot).await);
    loop {
        if let Event::Snapshot {
            sessions,
            active_runs,
            ..
        } = h.next().await.event
        {
            let finished = sessions[0].last_finished_run.as_ref().unwrap();
            assert_eq!(finished.run, second);
            assert_eq!(finished.input_id, Some("second".into()));
            assert_eq!(finished.outcome, RunOutcome::Succeeded);
            assert_eq!(sessions[0].state, SessionState::Running);
            assert_eq!(active_runs.len(), 1);
            break;
        }
    }
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn startup_recovers_crashed_runs_and_accepts_new_sends() {
    use forge_tools::failpoints::{Action, Failpoints, RUN_BEFORE_FINAL_COMMIT, SEND_AFTER_COMMIT};
    for point in [SEND_AFTER_COMMIT, RUN_BEFORE_FINAL_COMMIT] {
        let mut h = Harness::new([success("answer"), success("after restart")]);
        let session = h.session().await;
        h.runtime.failpoints = Failpoints::with_actions([(point, Action::Panic)]);
        let Harness {
            mut runtime,
            handle,
            events,
            clock,
            provider,
            secrets,
            directory,
        } = h;
        let task = tokio::spawn(async move {
            loop {
                runtime.dispatch_next().await.unwrap();
            }
        });
        let result = handle.execute(send(&session, "input", "hello")).await;
        if point == SEND_AFTER_COMMIT {
            assert!(result.is_err());
        } else {
            accepted(result.unwrap());
        }
        assert!(task.await.unwrap_err().is_panic());
        drop(handle);
        drop(events);
        let store = Store::open(directory.path()).unwrap();
        let run_id = store.input(&"input".into()).unwrap().unwrap().run_id;
        clock.advance(Duration::from_secs(10));
        let now = clock.now().as_millis() as u64;
        let (runtime, handle) = Runtime::new(
            RuntimeServices {
                clock: clock.clone(),
                store,
                providers: Arc::new(FixedProviderFactory(provider.clone())),
                secrets: secrets.clone(),
                initial_settings: common::settings("script"),
            },
            0,
        )
        .unwrap();
        let events = handle.subscribe();
        let mut h = Harness {
            runtime,
            handle,
            events,
            clock,
            provider,
            secrets,
            directory,
        };
        let run = h.runtime.services.store.run(&run_id).unwrap().unwrap();
        assert_eq!(run.state, RunState::Failed);
        assert_eq!(run.finished_at, Some(now));
        let event_json: String = h.runtime.services.store.connection().query_row(
            "SELECT event_json FROM events WHERE json_extract(event_json, '$.type') = 'run_finished' ORDER BY seq DESC LIMIT 1",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            serde_json::from_str::<Event>(&event_json).unwrap(),
            Event::RunFinished {
                run: run.id.clone(),
                outcome: run.outcome.clone().unwrap(),
                finished_at: now,
            }
        );
        assert!(matches!(
            run.outcome,
            Some(RunOutcome::Failed {
                kind: RunErrorKind::Interrupted,
                retryable: true,
                ..
            })
        ));
        let seq = h.runtime.services.store.last_seq().unwrap();
        h.clock.advance(Duration::from_secs(10));
        h = h.reopen();
        assert_eq!(h.runtime.services.store.last_seq().unwrap(), seq);
        assert_eq!(
            h.runtime.services.store.run(&run_id).unwrap(),
            Some(run.clone())
        );
        accepted(h.command(Command::GetSnapshot).await);
        let Event::Snapshot {
            sessions,
            active_runs,
            ..
        } = h.next().await.event
        else {
            panic!()
        };
        assert!(active_runs.is_empty());
        assert_eq!(sessions[0].state, SessionState::Idle);
        assert_eq!(sessions[0].updated_at, now);
        assert_eq!(
            sessions[0].last_finished_run,
            Some(FinishedRun {
                run: run_id,
                input_id: Some("input".into()),
                outcome: run.outcome.unwrap(),
                started_at: run.started_at,
                finished_at: now,
            })
        );
        assert_eq!(
            accepted(h.command(send(&session, "new-input", "try again")).await),
            "new-input"
        );
        h.finished().await;
        accepted(h.command(Command::DeleteSession { session }).await);
    }
}
