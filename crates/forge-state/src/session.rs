//! Per-session view model (spec §5.1 `SessionVm`, §6.2–§6.4, §6.9 Composer).

use forge_protocol::{
    InputId, ModelSelection, NoticeLevel, PermissionMode, QueueItem, QueueItemId, QueueItemState,
    QueuePauseReason, RunId, RunOutcome, Session, SessionId, SteerId, Usage,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Where a user message in the timeline came from (§6.4.1 "来自队列" tag).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageOrigin {
    Direct,
    Queue(QueueItemId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMessageVm {
    pub input_id: InputId,
    pub text: String,
    pub origin: MessageOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantMessageVm {
    pub run_id: RunId,
    pub text: String,
    pub reasoning: String,
    pub streaming: bool,
    /// Run was cancelled: keep received text and render the "（已停止）" suffix.
    pub stopped: bool,
}

/// Error classification for error cards / sidebar (§5.1 `ErrorVm.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Network,
    RateLimited,
    Auth,
    Quota,
    Server,
    ContextTooLong,
    Provider,
    Storage,
    Interrupted,
    Unknown,
}

impl ErrorKind {
    /// Best-effort mapping from `RunFinished.reason`. The protocol carries a free
    /// string today (see protocol gap notes), so only exact snake_case kind names
    /// are recognised.
    pub fn from_reason(reason: Option<&str>) -> Self {
        match reason.map(|r| r.trim()) {
            Some("network") => Self::Network,
            Some("rate_limited") => Self::RateLimited,
            Some("auth") => Self::Auth,
            Some("quota") => Self::Quota,
            Some("server") => Self::Server,
            Some("context_too_long") | Some("context") => Self::ContextTooLong,
            Some("provider") => Self::Provider,
            Some("storage") => Self::Storage,
            Some("interrupted") => Self::Interrupted,
            _ => Self::Unknown,
        }
    }

    /// Short label key for the sidebar second line ("失败 · 认证错误").
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Network => "error_kind.network",
            Self::RateLimited => "error_kind.rate_limited",
            Self::Auth => "error_kind.auth",
            Self::Quota => "error_kind.quota",
            Self::Server => "error_kind.server",
            Self::ContextTooLong => "error_kind.context",
            Self::Provider => "error_kind.provider",
            Self::Storage => "error_kind.storage",
            Self::Interrupted => "error_kind.interrupted",
            Self::Unknown => "error_kind.unknown",
        }
    }

    /// Title key for error cards and announcements (§10 `error.*`).
    pub fn title_key(self) -> &'static str {
        match self {
            Self::Network => "error.network.title",
            Self::RateLimited => "error.rate_limited.title",
            Self::Auth => "error.auth.title",
            Self::Quota => "error.quota.title",
            Self::Server => "error.server.title",
            Self::ContextTooLong => "error.context.title",
            Self::Interrupted => "error.interrupted.title",
            Self::Provider | Self::Storage | Self::Unknown => "error.unknown.title",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorVm {
    pub run_id: RunId,
    pub kind: ErrorKind,
    /// Raw detail, shown only under "详情".
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeVm {
    pub level: NoticeLevel,
    pub code: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineItem {
    User(UserMessageVm),
    Assistant(AssistantMessageVm),
    RunDivider { run_id: RunId, outcome: RunOutcome },
    Error(ErrorVm),
    Notice(NoticeVm),
}

/// `TimelineVm.history` (§6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryState {
    NotLoaded,
    Loading,
    HasMore,
    Complete,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunPhase {
    Starting,
    Thinking,
    Streaming,
    WaitingApproval,
    Retrying {
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },
    Cancelling,
    Finished(RunOutcome),
}

impl RunPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Finished(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunVm {
    pub id: RunId,
    pub session_id: SessionId,
    pub queue_item_id: Option<QueueItemId>,
    pub phase: RunPhase,
    pub usage: Usage,
    pub reason: Option<String>,
    /// UI-local time (ms) when `RunStarted` was applied; the protocol `Run`
    /// carries no timestamp.
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    /// `CancelRun` already sent for this run; ⌘. must not send it again (UI-CP-04).
    pub cancel_requested: bool,
}

/// Composer in-flight request (§6.9 "发送中").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InFlight {
    /// `queued`: the session was busy when the key was pressed (P3 queueing).
    Send {
        input_id: InputId,
        text: String,
        queued: bool,
    },
    Steer {
        steer_id: SteerId,
        text: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposerVm {
    pub draft: String,
    /// IME has an active composition (marked text). Enter belongs to the IME.
    pub ime_composing: bool,
    pub in_flight: Option<InFlight>,
    /// Localised "没有发送：<原因>" line.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextVm {
    pub used_tokens: u64,
    pub limit_tokens: u64,
    pub compacted_count: u32,
}

impl ContextVm {
    /// Percentage 0–100 (rounded down), `None` until the first `ContextUsage`.
    pub fn percent(&self) -> Option<u64> {
        (self.limit_tokens > 0).then(|| self.used_tokens.saturating_mul(100) / self.limit_tokens)
    }

    /// §6.2: ring turns `warning` at ≥80%.
    pub fn is_warning(&self) -> bool {
        self.percent().is_some_and(|p| p >= 80)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionVm {
    pub session: Session,
    pub timeline: Vec<TimelineItem>,
    pub history: HistoryState,
    pub runs: BTreeMap<RunId, RunVm>,
    pub active_run: Option<RunId>,
    pub composer: ComposerVm,
    pub context: ContextVm,
    pub queue: BTreeMap<QueueItemId, QueueItem>,
    pub queue_paused: Option<QueuePauseReason>,
    pub pending_approvals: BTreeSet<forge_protocol::ApprovalId>,
    pub last_outcome: Option<RunOutcome>,
    pub last_error: Option<ErrorKind>,
    /// Monotonic activity stamp (event seq) used to order the sidebar; the
    /// protocol `Session` has no `updated_at` yet.
    pub last_activity: u64,
    /// Accepted `Send`s whose run has not started yet (input id, text, direct?).
    pub(crate) accepted_inputs: VecDeque<(InputId, String, bool)>,
}

impl SessionVm {
    pub fn new(session: Session, activity: u64) -> Self {
        Self {
            session,
            timeline: Vec::new(),
            history: HistoryState::NotLoaded,
            runs: BTreeMap::new(),
            active_run: None,
            composer: ComposerVm::default(),
            context: ContextVm::default(),
            queue: BTreeMap::new(),
            queue_paused: None,
            pending_approvals: BTreeSet::new(),
            last_outcome: None,
            last_error: None,
            last_activity: activity,
            accepted_inputs: VecDeque::new(),
        }
    }

    pub fn id(&self) -> &SessionId {
        &self.session.id
    }

    pub fn title(&self) -> &str {
        &self.session.title
    }

    pub fn model(&self) -> &ModelSelection {
        &self.session.model
    }

    pub fn permission_mode(&self) -> &PermissionMode {
        &self.session.permission_mode
    }

    /// The active run if it is not terminal.
    pub fn running(&self) -> Option<&RunVm> {
        self.active_run
            .as_ref()
            .and_then(|id| self.runs.get(id))
            .filter(|run| !run.phase.is_terminal())
    }

    pub fn is_busy(&self) -> bool {
        self.running().is_some()
    }

    pub fn pending_queue_len(&self) -> usize {
        self.queue
            .values()
            .filter(|item| item.state == QueueItemState::Pending)
            .count()
    }

    pub fn uncertain_count(&self) -> usize {
        self.queue
            .values()
            .filter(|item| item.state == QueueItemState::Uncertain)
            .count()
    }

    /// Empty-session state (§6.3 "空（新会话）").
    pub fn is_empty(&self) -> bool {
        self.timeline.is_empty() && self.runs.is_empty()
    }

    pub(crate) fn assistant_mut(&mut self, run: &RunId) -> Option<&mut AssistantMessageVm> {
        self.timeline.iter_mut().rev().find_map(|item| match item {
            TimelineItem::Assistant(message) if &message.run_id == run => Some(message),
            _ => None,
        })
    }

    pub(crate) fn ensure_assistant(&mut self, run: &RunId) -> &mut AssistantMessageVm {
        if self.assistant_mut(run).is_none() {
            self.timeline
                .push(TimelineItem::Assistant(AssistantMessageVm {
                    run_id: run.clone(),
                    text: String::new(),
                    reasoning: String::new(),
                    streaming: true,
                    stopped: false,
                }));
        }
        self.assistant_mut(run)
            .expect("assistant message just inserted")
    }

    pub(crate) fn has_user_message(&self, input: &InputId) -> bool {
        self.timeline
            .iter()
            .any(|item| matches!(item, TimelineItem::User(m) if &m.input_id == input))
    }
}
