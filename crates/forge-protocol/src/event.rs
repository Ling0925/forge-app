use crate::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuePauseReason {
    User,
    Failed,
    Cancelled,
    Interrupted,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    SettingsUpdated {
        settings: Settings,
    },
    ProviderKeyConfigured {
        provider: ProviderId,
        key_status: KeyStatus,
    },
    HistoryLoaded {
        session: SessionId,
        messages: Vec<Message>,
        next_before: Option<u64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        runs: Vec<Run>,
    },
    Snapshot {
        sessions: Vec<Session>,
        workspaces: Vec<Workspace>,
        settings: Settings,
        providers: Vec<ProviderKeyStatus>,
        active_runs: Vec<ActiveRun>,
        last_seq: u64,
    },
    WorkspaceAdded {
        workspace: Workspace,
    },
    WorkspaceRemoved {
        workspace: WorkspaceId,
    },
    ProviderKeyTested {
        provider: ProviderId,
        ok: bool,
        error_kind: Option<ProviderKeyErrorKind>,
    },
    SessionCreated {
        session: Session,
    },
    SessionUpdated {
        session: Session,
    },
    SessionArchived {
        session: SessionId,
        archived: bool,
    },
    SessionDeleted {
        session: SessionId,
    },
    QueueItemAdded {
        item: QueueItem,
    },
    QueueItemUpdated {
        item: QueueItem,
    },
    QueueItemRemoved {
        session: SessionId,
        item: QueueItemId,
    },
    QueuePaused {
        session: SessionId,
        reason: QueuePauseReason,
    },
    QueueResumed {
        session: SessionId,
    },
    RunStarted {
        run: Run,
        input_id: InputId,
    },
    RunStateChanged {
        run: RunId,
        state: RunState,
    },
    RunFinished {
        run: RunId,
        outcome: RunOutcome,
        #[serde(default)]
        finished_at: u64,
    },
    SteerApplied {
        run: RunId,
        steer_id: SteerId,
    },
    TextDelta {
        run: RunId,
        text: String,
    },
    ReasoningDelta {
        run: RunId,
        text: String,
    },
    ProviderAttempt {
        run: RunId,
        n: u32,
    },
    ProviderRetrying {
        run: RunId,
        n: u32,
        delay_ms: u64,
        reason: String,
    },
    GenerationReset {
        run: RunId,
    },
    ToolCallProposed {
        call: ToolCall,
    },
    ToolCallStateChanged {
        call: ToolCallId,
        state: ToolCallState,
        result: Option<Content>,
    },
    ToolOutputChunk {
        call: ToolCallId,
        stream: OutputStream,
        bytes: Vec<u8>,
    },
    ApprovalRequested {
        approval: Approval,
    },
    ApprovalResolved {
        approval: ApprovalId,
        decision: ApprovalDecision,
        scope: ApprovalScope,
        by: ApprovalResolvedBy,
    },
    ContextUsage {
        run: RunId,
        used_tokens: u64,
        context_remaining_estimate: u64,
    },
    ContextCompacted {
        run: RunId,
        before_tokens: u64,
        after_tokens: u64,
    },
    CheckpointCreated {
        checkpoint: Checkpoint,
    },
    RestorePreviewed {
        checkpoint: CheckpointId,
        files: Vec<RestorePreviewFile>,
    },
    CheckpointRestored {
        checkpoint: CheckpointId,
        restored: Vec<PathBuf>,
        skipped: Vec<PathBuf>,
        failed: Vec<RestoreFailure>,
    },
    UsageRecorded {
        record: UsageRecord,
    },
    Notice {
        level: NoticeLevel,
        code: String,
        message: String,
    },
    ShutdownComplete {
        stopped: Vec<RunId>,
        unfinished: Vec<RunId>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub seq: u64,
    pub event: Event,
}
