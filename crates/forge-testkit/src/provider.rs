use forge_provider::{
    ErrorKind, Provider, ProviderError, ProviderFuture, ProviderRequest, ProviderStream,
    StreamEvent,
};
use forge_tools::clock::Clock;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

pub enum Step {
    Event(StreamEvent),
    Delay(Duration),
    Error(ProviderError),
    Disconnect,
}

pub enum Script {
    Stream(Vec<Step>),
    Error(ProviderError),
}

struct ScriptState {
    scripts: VecDeque<Script>,
    requests: Vec<ProviderRequest>,
}

pub struct ScriptedProvider {
    state: Mutex<ScriptState>,
    clock: Arc<dyn Clock>,
}

impl ScriptedProvider {
    pub fn new(clock: Arc<dyn Clock>, scripts: impl IntoIterator<Item = Script>) -> Self {
        Self {
            state: Mutex::new(ScriptState {
                scripts: scripts.into_iter().collect(),
                requests: Vec::new(),
            }),
            clock,
        }
    }

    pub fn requests(&self) -> Vec<ProviderRequest> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl Provider for ScriptedProvider {
    fn request(
        &self,
        request: ProviderRequest,
    ) -> ProviderFuture<'_, Result<Box<dyn ProviderStream>, ProviderError>> {
        let script = {
            let mut state = self.state.lock().unwrap();
            state.requests.push(request);
            state.scripts.pop_front()
        };
        Box::pin(async move {
            match script {
                Some(Script::Stream(steps)) => Ok(Box::new(ScriptedStream {
                    steps: steps.into(),
                    clock: self.clock.clone(),
                }) as Box<dyn ProviderStream>),
                Some(Script::Error(error)) => Err(error),
                None => Err(ProviderError {
                    code: None,
                    kind: ErrorKind::Configuration,
                    message: "provider script exhausted".into(),
                }),
            }
        })
    }
}

struct ScriptedStream {
    steps: VecDeque<Step>,
    clock: Arc<dyn Clock>,
}

impl ProviderStream for ScriptedStream {
    fn next(&mut self) -> ProviderFuture<'_, Option<Result<StreamEvent, ProviderError>>> {
        Box::pin(async move {
            while let Some(step) = self.steps.pop_front() {
                match step {
                    Step::Event(event) => return Some(Ok(event)),
                    Step::Delay(duration) => self.clock.sleep(duration).await,
                    Step::Error(error) => {
                        self.steps.clear();
                        return Some(Err(error));
                    }
                    Step::Disconnect => {
                        self.steps.clear();
                        return Some(Err(ProviderError {
                            code: None,
                            kind: ErrorKind::Disconnected,
                            message: "scripted stream disconnected".into(),
                        }));
                    }
                }
            }
            None
        })
    }
}

pub struct FixedProviderFactory(pub Arc<dyn Provider>);
impl forge_provider::ProviderFactory for FixedProviderFactory {
    fn create(
        &self,
        _settings: &forge_protocol::ProviderSettings,
        _key: Option<String>,
    ) -> Result<Arc<dyn Provider>, ProviderError> {
        Ok(self.0.clone())
    }
}
