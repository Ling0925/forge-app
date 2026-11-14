use crate::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    AddWorkspace {
        path: PathBuf,
    },
    RemoveWorkspace {
        workspace: WorkspaceId,
    },
    TestProviderKey {
        provider: ProviderId,
    },
    CreateSession {
        workspace: WorkspaceId,
        title: String,
        model: ModelSelection,
        permission_mode: PermissionMode,
    },
    RenameSession {
        session: SessionId,
        title: String,
    },
    ArchiveSession {
        session: SessionId,
        archived: bool,
    },
    DeleteSession {
        session: SessionId,
    },
    LoadHistory {
        session: SessionId,
        before_seq: Option<u64>,
    },
    Send {
        session: SessionId,
        input_id: InputId,
        content: Content,
    },
    Steer {
        run: RunId,
        steer_id: SteerId,
        content: Content,
    },
    EditQueueItem {
        item: QueueItemId,
        content: Content,
    },
    RemoveQueueItem {
        item: QueueItemId,
    },
    ReorderQueueItem {
        item: QueueItemId,
        before: Option<QueueItemId>,
    },
    ResolveUncertain {
        item: QueueItemId,
        action: UncertainAction,
    },
    PauseQueue {
        session: SessionId,
    },
    ResumeQueue {
        session: SessionId,
    },
    CancelRun {
        run: RunId,
    },
    RetryRun {
        run: RunId,
    },
    RetryNow {
        run: RunId,
    },
    DecideApproval {
        approval: ApprovalId,
        decision: ApprovalDecision,
        scope: ApprovalScope,
    },
    SetPermissionMode {
        session: SessionId,
        mode: PermissionMode,
    },
    PreviewRestore {
        checkpoint: CheckpointId,
    },
    RestoreCheckpoint {
        checkpoint: CheckpointId,
        overwrite: Vec<PathBuf>,
    },
    UpdateSettings {
        settings: Settings,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertainAction {
    Requeue,
    Discard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectCode {
    RunFinished,
    AlreadyFinished,
    ApprovalClosed,
    Conflict,
    NotFound,
    InvalidState,
    Storage,
    Unavailable,
    NotImplemented,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandResult {
    Accepted { id: CommandId },
    Rejected { code: RejectCode, message: String },
}
