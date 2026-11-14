//! Quit with active runs (user decision 2026-11-14, Core `Shutdown`):
//! confirm → `Shutdown` → wait `ShutdownComplete{stopped, unfinished}` (core
//! waits ≤3 s on its injected clock) → exit. Runs still unfinished are marked
//! `interrupted` by R9 on next launch, so the UI never blocks exit on them.

use crate::app::{AppState, CommandToken, Pending};
use forge_protocol::{Command, RunId};

/// How long the UI waits for `ShutdownComplete` before exiting anyway. Core's
/// own window is 3 s; the extra second covers event delivery.
pub const UI_SHUTDOWN_TIMEOUT_MS: u64 = 4_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuitState {
    Idle,
    /// Confirm dialog "有 {n} 个运行进行中" is open; default button is 取消.
    Confirming {
        active_runs: usize,
    },
    /// `Shutdown` sent at `since_ms`; waiting for `ShutdownComplete`.
    ShuttingDown {
        token: CommandToken,
        since_ms: u64,
    },
    /// The host may exit now.
    ReadyToExit {
        stopped: Vec<RunId>,
        unfinished: Vec<RunId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitDecision {
    /// No active runs: exit immediately, nothing is sent.
    ExitNow,
    /// Confirm dialog opened.
    NeedsConfirm { active_runs: usize },
    /// A quit is already being confirmed or shut down.
    InProgress,
}

impl AppState {
    /// ⌘Q / "退出 Forge" / last-window close that quits.
    pub fn request_quit(&mut self) -> QuitDecision {
        match self.quit {
            QuitState::Confirming { .. } | QuitState::ShuttingDown { .. } => {
                return QuitDecision::InProgress;
            }
            QuitState::ReadyToExit { .. } => return QuitDecision::ExitNow,
            QuitState::Idle => {}
        }
        let active_runs = self.active_runs().len();
        if active_runs == 0 {
            self.quit = QuitState::ReadyToExit {
                stopped: Vec::new(),
                unfinished: Vec::new(),
            };
            QuitDecision::ExitNow
        } else {
            self.quit = QuitState::Confirming { active_runs };
            QuitDecision::NeedsConfirm { active_runs }
        }
    }

    /// Dialog "取消" / Esc.
    pub fn cancel_quit(&mut self) {
        if matches!(self.quit, QuitState::Confirming { .. }) {
            self.quit = QuitState::Idle;
        }
    }

    /// Dialog "停止并退出": sends `Shutdown` exactly once.
    pub fn confirm_quit(&mut self) -> Option<CommandToken> {
        if !matches!(self.quit, QuitState::Confirming { .. }) {
            return None;
        }
        let token = self.push(Pending::Shutdown, Command::Shutdown);
        self.quit = QuitState::ShuttingDown {
            token,
            since_ms: self.now_ms,
        };
        Some(token)
    }

    /// Host timer tick while shutting down; exits after the UI timeout.
    pub fn poll_quit_timeout(&mut self) -> bool {
        if let QuitState::ShuttingDown { since_ms, .. } = self.quit
            && self.now_ms.saturating_sub(since_ms) >= UI_SHUTDOWN_TIMEOUT_MS
        {
            self.quit = QuitState::ReadyToExit {
                stopped: Vec::new(),
                unfinished: self.active_runs(),
            };
        }
        self.should_exit()
    }

    pub fn should_exit(&self) -> bool {
        matches!(self.quit, QuitState::ReadyToExit { .. })
    }

    pub(crate) fn on_shutdown_complete(&mut self, stopped: Vec<RunId>, unfinished: Vec<RunId>) {
        if matches!(self.quit, QuitState::ShuttingDown { .. }) {
            self.quit = QuitState::ReadyToExit {
                stopped,
                unfinished,
            };
        }
    }

    /// Core rejected `Shutdown` (e.g. `not_implemented` before P2): exit
    /// anyway; R9 marks the leftovers interrupted on next launch.
    pub(crate) fn on_shutdown_rejected(&mut self) {
        if matches!(self.quit, QuitState::ShuttingDown { .. }) {
            self.quit = QuitState::ReadyToExit {
                stopped: Vec::new(),
                unfinished: self.active_runs(),
            };
        }
    }
}
