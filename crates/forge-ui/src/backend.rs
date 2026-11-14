//! Bridge from the UI thread to `forge-core`.
//!
//! The core runtime runs on its own thread with a single-threaded tokio
//! runtime; the UI awaits command results on gpui's executor and receives
//! events through `RuntimeHandle::subscribe`.

use forge_protocol::{Command, CommandResult, EventEnvelope};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

/// `None` means the transport failed (core unreachable).
pub type CommandFuture = Pin<Box<dyn Future<Output = Option<CommandResult>> + Send>>;
pub type EventReceiver = tokio::sync::mpsc::UnboundedReceiver<EventEnvelope>;

pub trait Backend: 'static {
    fn execute(&self, command: Command) -> CommandFuture;
}

/// Why the core could not start (store open failure → §6.12 storage error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartError {
    pub code: String,
    pub path: PathBuf,
    pub message: String,
}

/// `~/Library/Application Support/forge-app` on macOS; `$FORGE_APP_DATA_DIR`
/// overrides it (tests, manual QA).
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FORGE_APP_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/forge-app")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("forge-app")
    }
}

/// P0 has no provider wiring yet; every request fails as a configuration error.
struct UnconfiguredProvider;

impl forge_provider::Provider for UnconfiguredProvider {
    fn request(
        &self,
        _request: forge_provider::ProviderRequest,
    ) -> forge_provider::ProviderFuture<
        '_,
        Result<Box<dyn forge_provider::ProviderStream>, forge_provider::ProviderError>,
    > {
        Box::pin(async {
            Err(forge_provider::ProviderError {
                kind: forge_provider::ErrorKind::Configuration,
                message: "no provider configured".into(),
            })
        })
    }
}

pub struct CoreBackend {
    handle: forge_core::RuntimeHandle,
}

impl CoreBackend {
    /// Opens the store under `dir` and starts the core runtime thread.
    pub fn start(dir: PathBuf) -> Result<(Self, EventReceiver), StartError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread_dir = dir.clone();
        std::thread::Builder::new()
            .name("forge-core".into())
            .spawn(move || {
                let fail = |code: &str, message: String| StartError {
                    code: code.into(),
                    path: thread_dir.clone(),
                    message,
                };
                if let Err(error) = std::fs::create_dir_all(&thread_dir) {
                    let _ = ready_tx.send(Err(fail("storage_unavailable", error.to_string())));
                    return;
                }
                let store = match forge_store::Store::open(&thread_dir) {
                    Ok(store) => store,
                    Err(error) => {
                        let _ = ready_tx.send(Err(fail("storage_unavailable", error.to_string())));
                        return;
                    }
                };
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build();
                let runtime = match runtime {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(fail("runtime_failed", error.to_string())));
                        return;
                    }
                };
                // P0: the store does not expose its last event seq yet; start at 0.
                let (mut core, handle) = forge_core::Runtime::new(
                    forge_core::RuntimeServices {
                        clock: Arc::new(forge_tools::clock::RealClock::default()),
                        store,
                        provider: Arc::new(UnconfiguredProvider),
                    },
                    0,
                );
                let events = handle.subscribe();
                if ready_tx.send(Ok((handle, events))).is_err() {
                    return;
                }
                runtime.block_on(async move { while let Ok(true) = core.dispatch_next().await {} });
            })
            .map_err(|error| StartError {
                code: "runtime_failed".into(),
                path: dir.clone(),
                message: error.to_string(),
            })?;
        let (handle, events) = ready_rx.recv().map_err(|_| StartError {
            code: "runtime_failed".into(),
            path: dir,
            message: "core thread exited during startup".into(),
        })??;
        Ok((Self { handle }, events))
    }
}

impl Backend for CoreBackend {
    fn execute(&self, command: Command) -> CommandFuture {
        let handle = self.handle.clone();
        Box::pin(async move { handle.execute(command).await.ok() })
    }
}

/// Used when the core failed to start: every command is a transport failure.
pub struct OfflineBackend;

impl Backend for OfflineBackend {
    fn execute(&self, _command: Command) -> CommandFuture {
        Box::pin(async { None })
    }
}
