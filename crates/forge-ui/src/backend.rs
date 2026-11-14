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

/// First-run settings (Core writes them only when the store has none). Core
/// ships no model names; the UI picks the defaults. Anthropic requires
/// `max_output_tokens`.
pub fn initial_settings() -> forge_protocol::Settings {
    use forge_protocol::*;
    Settings {
        ui_prefs: Default::default(),
        providers: vec![
            ProviderSettings {
                id: "openai".into(),
                endpoint: None,
                keychain_reference: None,
                max_output_tokens: None,
            },
            ProviderSettings {
                id: "anthropic".into(),
                endpoint: None,
                keychain_reference: None,
                max_output_tokens: Some(8_192),
            },
        ],
        model: ModelSelection {
            provider: "openai".into(),
            model: "gpt-5".into(),
        },
        permission_mode: PermissionMode::Ask,
    }
}

/// macOS Keychain; elsewhere (dev/CI on Linux) an in-memory store — Core has
/// no persistent `SecretStore` off macOS, so keys last for one launch there.
fn secret_store() -> Arc<dyn forge_core::secrets::SecretStore> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(forge_core::secrets::KeychainSecretStore)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(forge_core::secrets::MemorySecretStore::default())
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
                // HTTP providers need the IO driver as well as timers.
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let runtime = match runtime {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(fail("runtime_failed", error.to_string())));
                        return;
                    }
                };
                // Core resumes from the store's own last seq.
                let started = runtime.block_on(async {
                    forge_core::Runtime::new(
                        forge_core::RuntimeServices {
                            clock: Arc::new(forge_tools::clock::RealClock),
                            store,
                            providers: Arc::new(forge_provider::HttpProviderFactory),
                            secrets: secret_store(),
                            initial_settings: initial_settings(),
                        },
                        0,
                    )
                });
                let (mut core, handle) = match started {
                    Ok(pair) => pair,
                    Err(error) => {
                        let _ = ready_tx.send(Err(fail("storage_unavailable", error.to_string())));
                        return;
                    }
                };
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

#[cfg(test)]
mod tests {
    use super::*;
    use forge_protocol::PermissionMode;

    #[test]
    fn initial_settings_picks_default_model_and_providers() {
        let settings = initial_settings();
        assert_eq!(
            settings.model,
            forge_protocol::ModelSelection {
                provider: "openai".into(),
                model: "gpt-5".into(),
            }
        );
        assert_eq!(settings.permission_mode, PermissionMode::Ask);
        let ids: Vec<&str> = settings
            .providers
            .iter()
            .map(|provider| provider.id.0.as_str())
            .collect();
        assert_eq!(ids, ["openai", "anthropic"]);
        // Anthropic requires max_output_tokens; OpenAI does not.
        assert_eq!(settings.providers[0].max_output_tokens, None);
        assert_eq!(settings.providers[1].max_output_tokens, Some(8_192));
    }
}
