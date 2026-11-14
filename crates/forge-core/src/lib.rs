use forge_protocol::{Command, CommandResult, Event, EventEnvelope, RejectCode};
use forge_tools::clock::Clock;
use std::sync::{Arc, Mutex};
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
    pub provider: Arc<dyn forge_provider::Provider>,
}

pub struct Runtime {
    commands: mpsc::UnboundedReceiver<CommandRequest>,
    subscribers: Subscribers,
    seq: u64,
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
    pub fn new(services: RuntimeServices, last_seq: u64) -> (Self, RuntimeHandle) {
        let (commands, receiver) = mpsc::unbounded_channel();
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                commands: receiver,
                subscribers: subscribers.clone(),
                seq: last_seq,
                services,
                #[cfg(feature = "failpoints")]
                failpoints: forge_tools::failpoints::Failpoints::default(),
            },
            RuntimeHandle {
                commands,
                subscribers,
            },
        )
    }

    pub async fn next_command(&mut self) -> Option<CommandRequest> {
        self.commands.recv().await
    }

    pub async fn dispatch_next(&mut self) -> Result<bool, RuntimeClosed> {
        let Some(request) = self.next_command().await else {
            return Ok(false);
        };
        request
            .respond(CommandResult::Rejected {
                code: RejectCode::NotImplemented,
                message: "command is not implemented in P0".into(),
            })
            .map_err(|_| RuntimeClosed)?;
        Ok(true)
    }

    pub fn publish(&mut self, event: Event) -> EventEnvelope {
        self.seq += 1;
        let envelope = EventEnvelope {
            seq: self.seq,
            event,
        };
        self.subscribers
            .lock()
            .unwrap()
            .retain(|subscriber| subscriber.send(envelope.clone()).is_ok());
        envelope
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
