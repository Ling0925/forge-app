mod errors;
pub mod secrets;
mod tool_loop;

use forge_protocol::*;
use forge_provider::{
    ErrorKind, ProviderError, ProviderFactory, ProviderMessage, ProviderRequest, ProviderToolCall,
    StreamEvent,
};
use forge_store::{StoreError, WriteTransaction};
use forge_tools::{cancellation::CancellationToken, clock::Clock};
use secrets::SecretStore;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    mem::take,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Error)]
#[error("runtime channel closed")]
pub struct RuntimeClosed;

pub struct CommandRequest {
    pub command: Command,
    response: oneshot::Sender<CommandResult>,
}
impl CommandRequest {
    pub fn respond(self, result: CommandResult) -> Result<(), CommandResult> {
        self.response.send(result)
    }
}
type Subscribers = Arc<Mutex<Vec<mpsc::UnboundedSender<EventEnvelope>>>>;

pub struct RuntimeServices {
    pub clock: Arc<dyn Clock>,
    pub store: forge_store::Store,
    pub providers: Arc<dyn ProviderFactory>,
    pub secrets: Arc<dyn SecretStore>,
    pub initial_settings: Settings,
}
struct Running {
    active: ActiveRun,
    turn: TurnId,
    usage: Usage,
    request_usage: Usage,
    key: Option<String>,
    model: ModelSelection,
    configuration: Result<(ProviderSettings, Option<String>), ProviderError>,
    workspace: PathBuf,
    generation: String,
    calls: Vec<PendingCall>,
    remaining: VecDeque<ToolCall>,
    /// An assistant Message row already exists for this run (written alongside
    /// the first tool batch), so an empty final response adds no row.
    assistant_persisted: bool,
    cancellation: CancellationToken,
}
struct PendingCall {
    id: ToolCallId,
    name: String,
    arguments: String,
}
impl Running {
    /// One provider request just ended; fold its usage into the run total.
    fn end_request(&mut self) {
        let request = take(&mut self.request_usage);
        self.usage.input_tokens += request.input_tokens;
        self.usage.output_tokens += request.output_tokens;
        self.usage.cached_tokens += request.cached_tokens;
        self.usage.requests += request.requests;
        if let Some(cost) = request.estimated_cost_usd {
            *self.usage.estimated_cost_usd.get_or_insert(0.0) += cost;
        }
    }
}
enum Internal {
    Stream(RunId, Result<StreamEvent, ProviderError>),
    KeyTested(ProviderId, Result<(), ProviderError>),
    ToolFinished(RunId, ToolCall, tool_loop::Outcome),
}
struct ProviderJob {
    request: ProviderRequest,
    configuration: Result<(ProviderSettings, Option<String>), ProviderError>,
    test_key: bool,
}
struct Completed {
    id: CommandId,
    events: Vec<EventEnvelope>,
    job: Option<ProviderJob>,
}
#[derive(Debug, Error)]
enum CommandError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("{1}")]
    Rejected(RejectCode, String),
}
impl Completed {
    fn new(id: String, events: Vec<EventEnvelope>) -> Self {
        Self {
            id: CommandId(id),
            events,
            job: None,
        }
    }
}
pub struct Runtime {
    commands: mpsc::UnboundedReceiver<CommandRequest>,
    subscribers: Subscribers,
    seq: u64,
    internal: mpsc::UnboundedReceiver<Internal>,
    sender: mpsc::UnboundedSender<Internal>,
    active: BTreeMap<RunId, Running>,
    pub services: RuntimeServices,
    #[cfg(feature = "failpoints")]
    pub failpoints: forge_tools::failpoints::Failpoints,
}
#[derive(Clone)]
pub struct RuntimeHandle {
    commands: mpsc::UnboundedSender<CommandRequest>,
    subscribers: Subscribers,
}

impl Runtime {
    pub fn new(
        mut services: RuntimeServices,
        last_seq: u64,
    ) -> Result<(Self, RuntimeHandle), StoreError> {
        if services.store.settings()?.is_none() {
            let tx = services.store.write()?;
            tx.settings(&services.initial_settings)?;
            tx.commit()?;
        }
        let seq = last_seq.max(services.store.last_seq()?);
        let (commands, receiver) = mpsc::unbounded_channel();
        let (sender, internal) = mpsc::unbounded_channel();
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = Self {
            commands: receiver,
            subscribers: subscribers.clone(),
            seq,
            services,
            internal,
            sender,
            active: BTreeMap::new(),
            #[cfg(feature = "failpoints")]
            failpoints: forge_tools::failpoints::Failpoints::default(),
        };
        runtime.recover_running_runs()?;
        Ok((
            runtime,
            RuntimeHandle {
                commands,
                subscribers,
            },
        ))
    }
    fn recover_running_runs(&mut self) -> Result<(), StoreError> {
        let now = self.now();
        for mut run in self.services.store.running_runs()? {
            let mut session = self
                .services
                .store
                .session(&run.session_id)?
                .expect("run has session");
            let outcome = RunOutcome::Failed {
                kind: RunErrorKind::Interrupted,
                message: "上次异常退出时中断".into(),
                http_status: None,
                retry_after_ms: None,
                retryable: true,
            };
            run.state = RunState::Failed;
            run.outcome = Some(outcome.clone());
            run.finished_at = Some(now);
            session.state = SessionState::Idle;
            session.updated_at = now;
            session.last_finished_run = Some(FinishedRun {
                run: run.id.clone(),
                input_id: self.services.store.run_input_id(&run.id)?,
                outcome: outcome.clone(),
                started_at: run.started_at,
                finished_at: now,
            });
            let events = self.commit(
                vec![
                    Event::SessionUpdated {
                        session: session.clone(),
                    },
                    Event::RunFinished {
                        run: run.id.clone(),
                        outcome,
                        finished_at: now,
                    },
                ],
                |tx| {
                    tx.run(&run)?;
                    tx.session(&session)
                },
            )?;
            for event in events {
                self.broadcast(event);
            }
        }
        Ok(())
    }
    pub async fn next_command(&mut self) -> Option<CommandRequest> {
        self.commands.recv().await
    }
    pub async fn dispatch_next(&mut self) -> Result<bool, StoreError> {
        tokio::select! {
            request = self.commands.recv() => {
                let Some(request) = request else { return Ok(false); };
                let CommandRequest { command, response } = request;
                match self.command(command) {
                    Ok(completed) => {
                        let _ = response.send(CommandResult::Accepted { id: completed.id });
                        for envelope in completed.events { self.broadcast(envelope); }
                        if let Some(job) = completed.job { self.start(job); }
                    }
                    Err(error) => {
                        let code = match &error { CommandError::Store(_) => RejectCode::Storage, CommandError::Rejected(code,_) => code.clone() };
                        let _ = response.send(CommandResult::Rejected { code, message: error.to_string() });
                    }
                }
            }
            Some(message) = self.internal.recv() => self.internal_message(message)?,
        }
        Ok(true)
    }
    fn envelopes(&self, events: Vec<Event>) -> Vec<EventEnvelope> {
        events
            .into_iter()
            .enumerate()
            .map(|(i, event)| EventEnvelope {
                seq: self.seq + i as u64 + 1,
                event,
            })
            .collect()
    }
    fn commit(
        &mut self,
        events: Vec<Event>,
        write: impl FnOnce(&WriteTransaction<'_>) -> Result<(), StoreError>,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let envelopes = self.envelopes(events);
        let tx = self.services.store.write()?;
        write(&tx)?;
        tx.events(&envelopes)?;
        tx.commit()?;
        Ok(envelopes)
    }
    fn broadcast(&mut self, envelope: EventEnvelope) {
        self.seq = envelope.seq;
        self.subscribers
            .lock()
            .unwrap()
            .retain(|subscriber| subscriber.send(envelope.clone()).is_ok());
    }
    pub fn publish(&mut self, event: Event) -> Result<EventEnvelope, StoreError> {
        let envelope = if matches!(
            event,
            Event::TextDelta { .. }
                | Event::ReasoningDelta { .. }
                | Event::HistoryLoaded { .. }
                | Event::Snapshot { .. }
                | Event::ProviderKeyTested { .. }
        ) {
            self.envelopes(vec![event]).remove(0)
        } else {
            self.commit(vec![event], |_| Ok(()))?.remove(0)
        };
        self.broadcast(envelope.clone());
        Ok(envelope)
    }
    fn session(&self, id: &SessionId) -> Result<Session, CommandError> {
        self.services
            .store
            .session(id)?
            .ok_or_else(|| reject(RejectCode::NotFound, "session not found"))
    }
    fn settings(&self) -> Result<Settings, StoreError> {
        Ok(self
            .services
            .store
            .settings()?
            .expect("settings initialized by Runtime::new"))
    }
    fn now(&self) -> u64 {
        self.services.clock.now().as_millis() as u64
    }
    fn configuration(
        &self,
        provider: &ProviderId,
    ) -> Result<(ProviderSettings, Option<String>), ProviderError> {
        let settings = self
            .settings()
            .map_err(|_| configuration("settings could not be read"))?;
        let settings = settings
            .providers
            .into_iter()
            .find(|s| &s.id == provider)
            .ok_or_else(|| configuration("provider settings are missing"))?;
        let key = self
            .services
            .secrets
            .get(provider)
            .map_err(|_| configuration("provider key could not be read"))?;
        if key.as_ref().is_none_or(|key| key.is_empty()) {
            return Err(configuration("provider key is missing"));
        }
        Ok((settings, key))
    }
    fn command(&mut self, command: Command) -> Result<Completed, CommandError> {
        match command {
            Command::AddWorkspace { path } => {
                let root = path
                    .canonicalize()
                    .map_err(|e| reject(RejectCode::InvalidState, &e.to_string()))?;
                if let Some(workspace) = self
                    .services
                    .store
                    .workspaces()?
                    .into_iter()
                    .find(|w| w.root == root)
                {
                    return Ok(Completed::new(workspace.id.0, vec![]));
                }
                let workspace = Workspace {
                    id: WorkspaceId(self.services.store.new_id()?),
                    name: root
                        .file_name()
                        .unwrap_or(root.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                    root,
                };
                let events = self.commit(
                    vec![Event::WorkspaceAdded {
                        workspace: workspace.clone(),
                    }],
                    |tx| tx.workspace(&workspace),
                )?;
                Ok(Completed::new(workspace.id.0, events))
            }
            Command::RemoveWorkspace { workspace } => {
                if !self
                    .services
                    .store
                    .workspaces()?
                    .iter()
                    .any(|w| w.id == workspace)
                {
                    return Err(reject(RejectCode::NotFound, "workspace not found"));
                }
                if self
                    .services
                    .store
                    .sessions()?
                    .iter()
                    .any(|s| s.workspace_id == workspace)
                {
                    return Err(reject(RejectCode::InvalidState, "workspace has sessions"));
                }
                let events = self.commit(
                    vec![Event::WorkspaceRemoved {
                        workspace: workspace.clone(),
                    }],
                    |tx| tx.remove_workspace(&workspace),
                )?;
                Ok(Completed::new(workspace.0, events))
            }
            Command::CreateSession {
                workspace,
                title,
                model,
                permission_mode,
            } => {
                if !self
                    .services
                    .store
                    .workspaces()?
                    .iter()
                    .any(|w| w.id == workspace)
                {
                    return Err(reject(RejectCode::NotFound, "workspace not found"));
                }
                let now = self.now();
                let session = Session {
                    id: SessionId(self.services.store.new_id()?),
                    workspace_id: workspace,
                    title,
                    model,
                    permission_mode,
                    state: SessionState::Idle,
                    archived: false,
                    queue_state: QueueState::Active,
                    created_at: now,
                    updated_at: now,
                    last_finished_run: None,
                };
                let events = self.commit(
                    vec![Event::SessionCreated {
                        session: session.clone(),
                    }],
                    |tx| tx.session(&session),
                )?;
                Ok(Completed::new(session.id.0, events))
            }
            Command::RenameSession { session, title } => {
                let mut session = self.session(&session)?;
                session.title = title;
                session.updated_at = self.now();
                let events = self.commit(
                    vec![Event::SessionUpdated {
                        session: session.clone(),
                    }],
                    |tx| tx.session(&session),
                )?;
                Ok(Completed::new(session.id.0, events))
            }
            Command::ArchiveSession { session, archived } => {
                let mut session = self.session(&session)?;
                session.archived = archived;
                session.updated_at = self.now();
                let events = self.commit(
                    vec![
                        Event::SessionUpdated {
                            session: session.clone(),
                        },
                        Event::SessionArchived {
                            session: session.id.clone(),
                            archived,
                        },
                    ],
                    |tx| tx.session(&session),
                )?;
                Ok(Completed::new(session.id.0, events))
            }
            Command::DeleteSession { session } => {
                let stored = self.session(&session)?;
                if stored.state != SessionState::Idle {
                    return Err(reject(
                        RejectCode::InvalidState,
                        "session has an active run",
                    ));
                }
                let events = self.commit(
                    vec![Event::SessionDeleted {
                        session: session.clone(),
                    }],
                    |tx| tx.delete_session(&session),
                )?;
                Ok(Completed::new(session.0, events))
            }
            Command::UpdateSettings { settings } => {
                let id = self.services.store.new_id()?;
                let events = self.commit(
                    vec![Event::SettingsUpdated {
                        settings: settings.clone(),
                    }],
                    |tx| tx.settings(&settings),
                )?;
                Ok(Completed::new(id, events))
            }
            Command::SetProviderKey { provider, key } => {
                match &key {
                    Some(key) => self.services.secrets.set(&provider, key),
                    None => self.services.secrets.delete(&provider),
                }
                .map_err(|e| reject(RejectCode::Unavailable, &e.to_string()))?;
                let key_status = if key.as_ref().is_some_and(|key| !key.is_empty()) {
                    KeyStatus::Configured
                } else {
                    KeyStatus::Missing
                };
                let events = self.commit(
                    vec![Event::ProviderKeyConfigured {
                        provider: provider.clone(),
                        key_status,
                    }],
                    |_| Ok(()),
                )?;
                Ok(Completed::new(provider.0, events))
            }
            Command::LoadHistory {
                session,
                before,
                limit,
            } => {
                self.session(&session)?;
                let (messages, next_before) =
                    self.services.store.history(&session, before, limit)?;
                let runs = messages
                    .iter()
                    .map(|message| &message.run_id)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|id| {
                        self.services
                            .store
                            .run(id)
                            .map(|run| run.expect("message has run"))
                    })
                    .collect::<Result<_, _>>()?;
                let events = self.envelopes(vec![Event::HistoryLoaded {
                    session: session.clone(),
                    messages,
                    next_before,
                    runs,
                }]);
                Ok(Completed::new(session.0, events))
            }
            Command::GetSnapshot => {
                let id = self.services.store.new_id()?;
                let settings = self.settings()?;
                let providers = settings
                    .providers
                    .iter()
                    .map(|s| ProviderKeyStatus {
                        provider: s.id.clone(),
                        key_status: match self.services.secrets.get(&s.id) {
                            Ok(Some(key)) if !key.is_empty() => KeyStatus::Configured,
                            Ok(_) => KeyStatus::Missing,
                            Err(_) => KeyStatus::Unavailable,
                        },
                    })
                    .collect();
                let event = Event::Snapshot {
                    sessions: self.services.store.sessions()?,
                    workspaces: self.services.store.workspaces()?,
                    settings,
                    providers,
                    active_runs: self.active.values().map(|r| r.active.clone()).collect(),
                    last_seq: self.seq + 1,
                };
                let events = self.envelopes(vec![event]);
                Ok(Completed::new(id, events))
            }
            Command::Send {
                session,
                input_id,
                content,
            } => self.send(session, input_id, content),
            Command::TestProviderKey { provider } => {
                let settings = self.settings()?;
                let model = if settings.model.provider == provider {
                    Some(settings.model)
                } else {
                    self.services
                        .store
                        .sessions()?
                        .into_iter()
                        .find(|s| s.model.provider == provider)
                        .map(|s| s.model)
                };
                let id = self.services.store.new_id()?;
                let Some(model) = model else {
                    let events = self.envelopes(vec![Event::ProviderKeyTested {
                        provider,
                        ok: false,
                        error_kind: Some(ProviderKeyErrorKind::Configuration),
                    }]);
                    return Ok(Completed::new(id, events));
                };
                let configuration = self.configuration(&provider).map(|(mut settings, key)| {
                    settings.max_output_tokens = Some(1);
                    (settings, key)
                });
                let request = ProviderRequest {
                    run: RunId(id.clone()),
                    model,
                    system: None,
                    messages: vec![ProviderMessage {
                        role: MessageRole::User,
                        content: Content {
                            text: "Hi".into(),
                            attachments: vec![],
                        },
                        tool_call_id: None,
                        tool_calls: vec![],
                        is_error: false,
                    }],
                    tools: vec![],
                };
                Ok(Completed {
                    id: CommandId(id),
                    events: vec![],
                    job: Some(ProviderJob {
                        request,
                        configuration,
                        test_key: true,
                    }),
                })
            }
            _ => Err(reject(
                RejectCode::NotImplemented,
                "command is not implemented in P1",
            )),
        }
    }
    fn send(
        &mut self,
        session: SessionId,
        input_id: InputId,
        content: Content,
    ) -> Result<Completed, CommandError> {
        if let Some(previous) = self.services.store.input(&input_id)? {
            let run = self
                .services
                .store
                .run(&previous.run_id)?
                .expect("message references run");
            if previous.content != content || run.session_id != session {
                return Err(reject(
                    RejectCode::Conflict,
                    "input_id already has different input",
                ));
            }
            return Ok(Completed::new(input_id.0, vec![]));
        }
        let mut session = self.session(&session)?;
        if session.state != SessionState::Idle {
            return Err(reject(
                RejectCode::InvalidState,
                "session has an active run",
            ));
        }
        let configuration = self.configuration(&session.model.provider);
        let workspace = self
            .services
            .store
            .workspaces()?
            .into_iter()
            .find(|workspace| workspace.id == session.workspace_id)
            .expect("session has workspace")
            .root;
        let now = self.now();
        let run = Run {
            id: RunId(self.services.store.new_id()?),
            session_id: session.id.clone(),
            queue_item_id: None,
            state: RunState::Running,
            outcome: None,
            usage: Usage::default(),
            started_at: now,
            finished_at: None,
        };
        let turn = Turn {
            id: TurnId(self.services.store.new_id()?),
            run_id: run.id.clone(),
            ordinal: 0,
        };
        let message = Message {
            id: MessageId(self.services.store.new_id()?),
            ordinal: 0,
            input_id: Some(input_id.clone()),
            run_id: run.id.clone(),
            turn_id: turn.id.clone(),
            role: MessageRole::User,
            content,
            tool_call_id: None,
        };
        let mut messages = self.provider_messages(&session.id)?;
        messages.push(ProviderMessage {
            role: message.role.clone(),
            content: message.content.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            is_error: false,
        });
        session.state = SessionState::Running;
        session.updated_at = now;
        let events = self.envelopes(vec![
            Event::RunStarted {
                run: run.clone(),
                input_id: input_id.clone(),
            },
            Event::SessionUpdated {
                session: session.clone(),
            },
        ]);
        let tx = self.services.store.write()?;
        tx.run(&run)?;
        tx.turn(&turn)?;
        tx.message(&message)?;
        tx.session(&session)?;
        tx.events(&events)?;
        forge_tools::failpoint!(self.failpoints, forge_tools::failpoints::SEND_BEFORE_COMMIT);
        tx.commit()?;
        forge_tools::failpoint!(self.failpoints, forge_tools::failpoints::SEND_AFTER_COMMIT);
        let request = ProviderRequest {
            run: run.id.clone(),
            model: session.model.clone(),
            system: None,
            messages,
            tools: tool_loop::definitions(),
        };
        self.active.insert(
            run.id.clone(),
            Running {
                active: ActiveRun {
                    run,
                    input_id: input_id.clone(),
                    text: String::new(),
                    reasoning: String::new(),
                },
                turn: turn.id,
                usage: Usage::default(),
                request_usage: Usage::default(),
                key: configuration.as_ref().ok().and_then(|(_, key)| key.clone()),
                model: session.model,
                configuration: configuration.clone(),
                workspace,
                generation: String::new(),
                calls: Vec::new(),
                remaining: VecDeque::new(),
                assistant_persisted: false,
                cancellation: CancellationToken::default(),
            },
        );
        Ok(Completed {
            id: CommandId(input_id.0),
            events,
            job: Some(ProviderJob {
                request,
                configuration,
                test_key: false,
            }),
        })
    }
    fn start(&self, job: ProviderJob) {
        let factory = self.services.providers.clone();
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let run = job.request.run.clone();
            let provider_id = job.request.model.provider.clone();
            let key = job
                .configuration
                .as_ref()
                .ok()
                .and_then(|(_, key)| key.clone());
            let result = async {
                let (settings, key) = job.configuration?;
                let provider = factory.create(&settings, key)?;
                let mut stream = provider.request(job.request).await?;
                while let Some(event) = stream.next().await {
                    let event = event?;
                    if event == StreamEvent::Finished {
                        return Ok(());
                    }
                    if !job.test_key
                        && sender
                            .send(Internal::Stream(run.clone(), Ok(event)))
                            .is_err()
                    {
                        return Ok(());
                    }
                }
                Err(ProviderError {
                    code: None,
                    kind: ErrorKind::Disconnected,
                    message: "provider stream ended before its terminal event".into(),
                })
            }
            .await
            .map_err(|error| match key {
                Some(key) => error.redacted(&key),
                None => error,
            });
            let message = if job.test_key {
                Internal::KeyTested(provider_id, result)
            } else {
                Internal::Stream(run, result.map(|()| StreamEvent::Finished))
            };
            let _ = sender.send(message);
        });
    }
    fn internal_message(&mut self, message: Internal) -> Result<(), StoreError> {
        match message {
            Internal::KeyTested(provider, result) => {
                let error_kind = result
                    .as_ref()
                    .err()
                    .map(|error| errors::key_error(error, &provider));
                self.publish(Event::ProviderKeyTested {
                    provider,
                    ok: result.is_ok(),
                    error_kind,
                })?;
            }
            Internal::Stream(run, result) => {
                if let Err(error) = self.stream_message(run.clone(), result) {
                    self.storage_failed(run, error);
                }
            }
            Internal::ToolFinished(run, call, outcome) => {
                if let Err(error) = self.tool_finished(run.clone(), call, outcome) {
                    self.storage_failed(run, error);
                }
            }
        }
        Ok(())
    }
    fn stream_message(
        &mut self,
        run: RunId,
        result: Result<StreamEvent, ProviderError>,
    ) -> Result<(), StoreError> {
        // The provider task drains its stream even after the run reached a
        // terminal state (e.g. a malformed stream failed it mid-response);
        // events arriving after that are stale and must not touch the run.
        if !self.active.contains_key(&run) {
            return Ok(());
        }
        match result {
            Ok(StreamEvent::TextDelta(text)) => {
                let running = self.active.get_mut(&run).expect("stream has active run");
                running.active.text.push_str(&text);
                running.generation.push_str(&text);
                self.publish(Event::TextDelta { run, text })?;
            }
            Ok(StreamEvent::ReasoningDelta(text)) => {
                self.active
                    .get_mut(&run)
                    .expect("stream has active run")
                    .active
                    .reasoning
                    .push_str(&text);
                self.publish(Event::ReasoningDelta { run, text })?;
            }
            Ok(StreamEvent::Usage(usage)) => {
                self.active
                    .get_mut(&run)
                    .expect("stream has active run")
                    .request_usage = usage;
            }
            Ok(StreamEvent::ToolCallStarted { id, name }) => {
                let running = self.active.get_mut(&run).expect("stream has active run");
                // A call id is unique across the conversation, not within one
                // response: reusing an id already persisted by an earlier
                // round or run would upsert that call's row and hang the new
                // result on the old tool/arguments.
                if running.calls.iter().any(|call| call.id == id)
                    || self.services.store.tool_call(&id)?.is_some()
                {
                    self.finish(
                        run,
                        Err(invalid_response("provider repeated a tool call id")),
                    )?;
                } else {
                    running.calls.push(PendingCall {
                        id,
                        name,
                        arguments: String::new(),
                    });
                }
            }
            Ok(StreamEvent::ToolArgumentsDelta { id, json }) => {
                let running = self.active.get_mut(&run).expect("stream has active run");
                match running.calls.iter().position(|call| call.id == id) {
                    Some(index) => running.calls[index].arguments.push_str(&json),
                    None => self.finish(
                        run,
                        Err(invalid_response(
                            "provider sent tool arguments for an unknown call",
                        )),
                    )?,
                }
            }
            Ok(StreamEvent::Finished) => {
                let tools = {
                    let running = self.active.get_mut(&run).expect("stream has active run");
                    running.end_request();
                    !running.calls.is_empty()
                };
                if tools {
                    self.execute_tools(run)?;
                } else {
                    self.finish(run, Ok(()))?;
                }
            }
            Err(error) => {
                self.active
                    .get_mut(&run)
                    .expect("stream has active run")
                    .end_request();
                self.finish(run, Err(error))?;
            }
        }
        Ok(())
    }

    /// The provider response ended with tool calls: propose every call, then run
    /// them sequentially so each terminal state is published before the next starts.
    /// The assistant message that produced the calls and the `tool_calls` rows
    /// land in the same commit, so persisted context always pairs correctly.
    fn execute_tools(&mut self, run: RunId) -> Result<(), StoreError> {
        let (first, assistant, calls, events) = {
            let running = self.active.get_mut(&run).expect("stream has active run");
            let assistant = Message {
                id: MessageId(self.services.store.new_id()?),
                ordinal: 0,
                input_id: None,
                run_id: run.clone(),
                turn_id: running.turn.clone(),
                role: MessageRole::Assistant,
                content: Content {
                    text: take(&mut running.generation),
                    attachments: vec![],
                },
                tool_call_id: None,
            };
            let calls: Vec<ToolCall> = take(&mut running.calls)
                .into_iter()
                .map(|pending| {
                    let has_side_effects = tool_loop::has_side_effects(&pending.name);
                    ToolCall {
                        id: pending.id,
                        run_id: run.clone(),
                        tool: pending.name,
                        arguments_json: if pending.arguments.is_empty() {
                            "{}".into()
                        } else {
                            pending.arguments
                        },
                        state: ToolCallState::Proposed,
                        result: None,
                        has_side_effects,
                    }
                })
                .collect();
            let first = calls[0].clone();
            running.remaining = calls.iter().skip(1).cloned().collect();
            let mut events: Vec<Event> = calls
                .iter()
                .cloned()
                .map(|call| Event::ToolCallProposed { call })
                .collect();
            events.push(Event::ToolCallStateChanged {
                call: first.id.clone(),
                state: ToolCallState::Running,
                result: None,
            });
            (first, assistant, calls, events)
        };
        let mut running_first = first.clone();
        running_first.state = ToolCallState::Running;
        for envelope in self.commit(events, |tx| {
            tx.message(&assistant)?;
            for call in &calls {
                tx.tool_call(call)?;
            }
            tx.tool_call(&running_first)
        })? {
            self.broadcast(envelope);
        }
        self.active
            .get_mut(&run)
            .expect("stream has active run")
            .assistant_persisted = true;
        self.start_tool(run, first);
        Ok(())
    }

    fn start_tool(&self, run: RunId, call: ToolCall) {
        let running = self.active.get(&run).expect("stream has active run");
        let workspace = running.workspace.clone();
        let cancellation = running.cancellation.clone();
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let outcome = tool_loop::execute(call.clone(), workspace, cancellation).await;
            let _ = sender.send(Internal::ToolFinished(run, call, outcome));
        });
    }

    fn tool_finished(
        &mut self,
        run: RunId,
        mut call: ToolCall,
        outcome: tool_loop::Outcome,
    ) -> Result<(), StoreError> {
        // A tool task can outlive its run (the run already finished or was
        // removed); its late completion is stale and must not touch the run.
        if !self.active.contains_key(&run) {
            return Ok(());
        }
        let (state, result) = match outcome {
            tool_loop::Outcome::Succeeded(text) => (ToolCallState::Succeeded, Some(text)),
            tool_loop::Outcome::Failed(text) => (ToolCallState::Failed, Some(text)),
            tool_loop::Outcome::Cancelled => (ToolCallState::Cancelled, None),
        };
        let result = result.map(|text| Content {
            text,
            attachments: vec![],
        });
        call.state = state.clone();
        call.result = result.clone();
        let (next, message) = {
            let running = self.active.get_mut(&run).expect("stream has active run");
            if state == ToolCallState::Cancelled {
                running.remaining.clear();
            }
            let id = self.services.store.new_id()?;
            let message = result.clone().map(|content| Message {
                id: MessageId(id),
                ordinal: 0,
                input_id: None,
                run_id: run.clone(),
                turn_id: running.turn.clone(),
                role: MessageRole::Tool,
                content,
                tool_call_id: Some(call.id.clone()),
            });
            (running.remaining.pop_front(), message)
        };
        for envelope in self.commit(
            vec![Event::ToolCallStateChanged {
                call: call.id.clone(),
                state: state.clone(),
                result,
            }],
            |tx| {
                tx.tool_call(&call)?;
                if let Some(message) = &message {
                    tx.message(message)?;
                }
                Ok(())
            },
        )? {
            self.broadcast(envelope);
        }
        match (state, next) {
            (ToolCallState::Cancelled, _) => self.finish_with(
                run,
                RunOutcome::Cancelled {
                    source: CancelSource::User,
                },
            )?,
            (_, Some(next)) => {
                let mut next = next;
                next.state = ToolCallState::Running;
                for envelope in self.commit(
                    vec![Event::ToolCallStateChanged {
                        call: next.id.clone(),
                        state: ToolCallState::Running,
                        result: None,
                    }],
                    |tx| tx.tool_call(&next),
                )? {
                    self.broadcast(envelope);
                }
                self.start_tool(run, next);
            }
            (_, None) => self.continue_run(run)?,
        }
        Ok(())
    }

    /// Rebuild the provider-visible transcript from persisted rows. Each
    /// `role: tool` message pairs with the assistant row that produced its
    /// call — the most recent assistant message — so the emitted assistant
    /// carries exactly the calls that have a persisted result. Calls whose
    /// result was never written (a run interrupted mid-execution, or one
    /// cancelled before completing) leave no tool message and are dropped from
    /// the transcript rather than breaking the providers' pairing contract.
    fn provider_messages(&self, session: &SessionId) -> Result<Vec<ProviderMessage>, StoreError> {
        let mut messages: Vec<ProviderMessage> = Vec::new();
        let mut owner: Option<usize> = None;
        for message in self.services.store.messages(session)? {
            match message.role {
                MessageRole::Assistant => {
                    owner = Some(messages.len());
                    messages.push(ProviderMessage {
                        role: MessageRole::Assistant,
                        content: message.content,
                        tool_call_id: None,
                        tool_calls: vec![],
                        is_error: false,
                    });
                }
                MessageRole::User => {
                    owner = None;
                    messages.push(ProviderMessage {
                        role: MessageRole::User,
                        content: message.content,
                        tool_call_id: None,
                        tool_calls: vec![],
                        is_error: false,
                    });
                }
                MessageRole::Tool => {
                    let (Some(id), Some(owner)) = (message.tool_call_id.clone(), owner) else {
                        continue;
                    };
                    let Some(call) = self.services.store.tool_call(&id)? else {
                        continue;
                    };
                    messages[owner].tool_calls.push(ProviderToolCall {
                        id: call.id.clone(),
                        name: call.tool.clone(),
                        arguments_json: call.arguments_json.clone(),
                    });
                    messages.push(ProviderMessage {
                        role: MessageRole::Tool,
                        content: message.content,
                        tool_call_id: Some(id),
                        tool_calls: vec![],
                        is_error: call.state == ToolCallState::Failed,
                    });
                }
            }
        }
        Ok(messages)
    }

    /// All tool calls of a provider response are done; feed the results back and
    /// keep the same run going with a follow-up provider request.
    fn continue_run(&mut self, run: RunId) -> Result<(), StoreError> {
        let (model, configuration, session_id) = {
            let running = self.active.get(&run).expect("stream has active run");
            (
                running.model.clone(),
                running.configuration.clone(),
                running.active.run.session_id.clone(),
            )
        };
        let messages = self.provider_messages(&session_id)?;
        self.start(ProviderJob {
            request: ProviderRequest {
                run,
                model,
                system: None,
                messages,
                tools: tool_loop::definitions(),
            },
            configuration,
            test_key: false,
        });
        Ok(())
    }
    fn storage_failed(&mut self, id: RunId, error: StoreError) {
        let running = self.active.remove(&id).expect("stream has active run");
        let outcome = errors::storage_outcome(error, running.key.as_deref());
        let mut run = running.active.run;
        let now = self.now();
        run.state = RunState::Failed;
        run.outcome = Some(outcome.clone());
        run.finished_at = Some(now);
        run.usage = running.usage;
        let session = self.services.store.session(&run.session_id).ok().flatten();
        let session = session.map(|mut session| {
            session.state = SessionState::Idle;
            session.updated_at = now;
            session.last_finished_run = Some(FinishedRun {
                run: run.id.clone(),
                input_id: Some(running.active.input_id),
                outcome: outcome.clone(),
                started_at: run.started_at,
                finished_at: now,
            });
            session
        });
        let mut events = Vec::new();
        if let Some(session) = &session {
            events.push(Event::SessionUpdated {
                session: session.clone(),
            });
        }
        events.push(Event::RunFinished {
            run: id,
            outcome,
            finished_at: now,
        });
        let envelopes = self.envelopes(events.clone());
        if self
            .commit(events, |tx| {
                tx.run(&run)?;
                if let Some(session) = &session {
                    tx.session(session)?;
                }
                Ok(())
            })
            .is_err()
            && let Some(session) = session
        {
            let _ = self.services.store.write().and_then(|tx| {
                tx.session(&session)?;
                tx.events(&envelopes[..1])?;
                tx.commit()
            });
        }
        for event in envelopes {
            self.broadcast(event);
        }
    }
    fn finish(&mut self, id: RunId, result: Result<(), ProviderError>) -> Result<(), StoreError> {
        let outcome = match result {
            Ok(()) => RunOutcome::Succeeded,
            Err(error) => {
                let running = self.active.get(&id).expect("stream has active run");
                let provider = self
                    .services
                    .store
                    .session(&running.active.run.session_id)?
                    .expect("active run has session")
                    .model
                    .provider;
                errors::outcome(error, self.services.clock.now(), &provider)
            }
        };
        self.finish_with(id, outcome)
    }
    fn finish_with(&mut self, id: RunId, outcome: RunOutcome) -> Result<(), StoreError> {
        let running = self.active.get_mut(&id).expect("stream has active run");
        let mut run = running.active.run.clone();
        // The persisted assistant text is this response's share only; earlier
        // responses each persisted their own row alongside their tool calls.
        let text = take(&mut running.generation);
        let persist_message = !text.is_empty() || !running.assistant_persisted;
        let usage = running.usage.clone();
        let input_id = running.active.input_id.clone();
        let turn = running.turn.clone();
        let mut session = self
            .services
            .store
            .session(&run.session_id)?
            .expect("active run has session");
        let success = outcome == RunOutcome::Succeeded;
        run.state = match outcome {
            RunOutcome::Succeeded => RunState::Succeeded,
            RunOutcome::Cancelled { .. } => RunState::Cancelled,
            RunOutcome::Failed { .. } => RunState::Failed,
        };
        run.outcome = Some(outcome.clone());
        let now = self.now();
        run.finished_at = Some(now);
        run.usage = usage;
        session.state = SessionState::Idle;
        session.updated_at = now;
        session.last_finished_run = Some(FinishedRun {
            run: run.id.clone(),
            input_id: Some(input_id),
            outcome: outcome.clone(),
            started_at: run.started_at,
            finished_at: now,
        });
        let message = Message {
            id: MessageId(self.services.store.new_id()?),
            ordinal: 0,
            input_id: None,
            run_id: run.id.clone(),
            turn_id: turn,
            role: MessageRole::Assistant,
            content: Content {
                text,
                attachments: vec![],
            },
            tool_call_id: None,
        };
        let record = UsageRecord {
            id: UsageRecordId(self.services.store.new_id()?),
            run_id: run.id.clone(),
            provider: session.model.provider.clone(),
            model: session.model.model.clone(),
            usage: run.usage.clone(),
        };
        let events = self.envelopes(vec![
            Event::UsageRecorded {
                record: record.clone(),
            },
            Event::SessionUpdated {
                session: session.clone(),
            },
            Event::RunFinished {
                run: id.clone(),
                outcome,
                finished_at: now,
            },
        ]);
        let tx = self.services.store.write()?;
        if success && persist_message {
            tx.message(&message)?;
        }
        tx.run(&run)?;
        tx.usage(&record)?;
        tx.session(&session)?;
        tx.events(&events)?;
        forge_tools::failpoint!(
            self.failpoints,
            forge_tools::failpoints::RUN_BEFORE_FINAL_COMMIT
        );
        tx.commit()?;
        self.active.remove(&id);
        for event in events {
            self.broadcast(event);
        }
        Ok(())
    }
}
fn reject(code: RejectCode, message: &str) -> CommandError {
    CommandError::Rejected(code, message.into())
}
fn configuration(message: &str) -> ProviderError {
    ProviderError {
        code: None,
        kind: ErrorKind::Configuration,
        message: message.into(),
    }
}
fn invalid_response(message: &str) -> ProviderError {
    ProviderError {
        code: None,
        kind: ErrorKind::InvalidResponse,
        message: message.into(),
    }
}
impl RuntimeHandle {
    pub async fn execute(&self, command: Command) -> Result<CommandResult, RuntimeClosed> {
        let (response, receiver) = oneshot::channel();
        self.commands
            .send(CommandRequest { command, response })
            .map_err(|_| RuntimeClosed)?;
        receiver.await.map_err(|_| RuntimeClosed)
    }
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<EventEnvelope> {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.subscribers.lock().unwrap().push(sender);
        receiver
    }
}
