use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

macro_rules! ids {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl From<&str> for $name {
            fn from(value: &str) -> Self { Self(value.to_owned()) }
        }
    )+};
}

ids!(
    WorkspaceId,
    SessionId,
    QueueItemId,
    InputId,
    SteerId,
    RunId,
    TurnId,
    MessageId,
    ToolCallId,
    ApprovalId,
    CheckpointId,
    UsageRecordId,
    CommandId,
    ProviderId,
    BlobHash
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    ReadOnly,
    Ask,
    Unrestricted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Running,
    WaitingApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueItemState {
    Pending,
    Started,
    Done,
    Failed,
    Cancelled,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    Active,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    WaitingApproval,
    Retrying,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunOutcome {
    Succeeded,
    Cancelled {
        source: CancelSource,
    },
    Failed {
        kind: RunErrorKind,
        message: String,
        http_status: Option<u16>,
        retry_after_ms: Option<u64>,
        retryable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelSource {
    User,
    AppQuit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunErrorKind {
    Network,
    RateLimited,
    Auth,
    Quota,
    Server,
    ContextTooLong,
    Interrupted,
    Unknown { source: UnknownSource },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownSource {
    Provider,
    Storage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallState {
    Proposed,
    AwaitingApproval,
    Approved,
    Denied,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    Once,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalResolvedBy {
    User,
    Cancel,
    TimeoutNone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    pub provider: ProviderId,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub root: PathBuf,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub model: ModelSelection,
    pub permission_mode: PermissionMode,
    pub state: SessionState,
    pub archived: bool,
    pub queue_state: QueueState,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub media_type: String,
    pub blob: BlobHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: QueueItemId,
    pub input_id: InputId,
    pub session_id: SessionId,
    pub content: Content,
    pub state: QueueItemState,
    pub ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteerInput {
    pub id: SteerId,
    pub run_id: RunId,
    pub content: Content,
    pub consumed: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub requests: u64,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub session_id: SessionId,
    pub queue_item_id: Option<QueueItemId>,
    pub state: RunState,
    pub outcome: Option<RunOutcome>,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub id: TurnId,
    pub run_id: RunId,
    pub ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub ordinal: u64,
    pub input_id: Option<InputId>,
    pub run_id: RunId,
    pub turn_id: TurnId,
    pub role: MessageRole,
    pub content: Content,
    pub tool_call_id: Option<ToolCallId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub run_id: RunId,
    pub tool: String,
    pub arguments_json: String,
    pub state: ToolCallState,
    pub result: Option<Content>,
    pub has_side_effects: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub id: ApprovalId,
    pub tool_call_id: ToolCallId,
    pub requested_at_unix_ms: u64,
    pub state: ApprovalState,
    pub decision: Option<ApprovalDecision>,
    pub resolved_by: Option<ApprovalResolvedBy>,
    pub scope: ApprovalScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointFile {
    pub path: PathBuf,
    pub before: Option<BlobHash>,
    pub after: Option<BlobHash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: CheckpointId,
    pub run_id: RunId,
    pub files: Vec<CheckpointFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestorePreviewStatus {
    Clean,
    ModifiedExternally,
    DeletedExternally,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePreviewFile {
    pub path: PathBuf,
    pub status: RestorePreviewStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreFailure {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: UsageRecordId,
    pub run_id: RunId,
    pub provider: ProviderId,
    pub model: String,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub id: ProviderId,
    pub endpoint: Option<String>,
    pub keychain_reference: Option<String>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKeyErrorKind {
    Network,
    RateLimited,
    Auth,
    Quota,
    Server,
    Configuration,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub ui_prefs: BTreeMap<String, String>,
    pub providers: Vec<ProviderSettings>,
    pub model: ModelSelection,
    pub permission_mode: PermissionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderKeyStatus {
    pub provider: ProviderId,
    pub key_configured: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveRun {
    pub run: Run,
    pub input_id: InputId,
    pub text: String,
    pub reasoning: String,
}
