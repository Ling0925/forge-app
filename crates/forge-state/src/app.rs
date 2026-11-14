//! `AppState`: reduces protocol `EventEnvelope`s into view state and turns user
//! intents into protocol `Command`s (spec §5). Pure Rust, no GPUI.
//!
//! Flow: the host (forge-ui) calls an intent method, drains `take_outbox()`,
//! sends each `Command` to the core runtime and reports the reply with
//! `apply_result(token, result)`. Events from the runtime subscription go to
//! `apply(envelope)`. Time is injected through `set_now_ms` so tests are
//! deterministic.

use crate::i18n::{t, tf};
use crate::ids::{IdSource, RandomIds};
use crate::onboarding::OnboardingVm;
use crate::palette::PaletteVm;
use crate::quit::QuitState;
use crate::session::*;
use crate::settings::{ConfirmDialog, SettingsVm};
use forge_protocol::*;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// Correlates a sent `Command` with its `CommandResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommandToken(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub token: CommandToken,
    pub command: Command,
}

/// What the UI is waiting on for each in-flight command.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Pending {
    Send {
        session: SessionId,
        input_id: InputId,
    },
    Steer {
        session: SessionId,
        steer_id: SteerId,
    },
    Cancel {
        session: SessionId,
        run: RunId,
    },
    CreateSession,
    Rename {
        session: SessionId,
        previous: String,
    },
    Archive {
        session: SessionId,
    },
    Delete {
        session: SessionId,
    },
    LoadHistory {
        session: SessionId,
        before: Option<u64>,
    },
    Snapshot,
    TestKey {
        provider: ProviderId,
    },
    SetKey {
        provider: ProviderId,
    },
    UpdateSettings {
        previous: Settings,
    },
    SetPermissionMode {
        session: SessionId,
        previous: PermissionMode,
    },
    AddWorkspace,
    Retry,
    Shutdown,
}

/// Feature switches that follow PROTOCOL.md phases. P1 defaults: busy-time
/// queueing (`Send` while running) and `Steer` land in P3; `CancelRun` in P2
/// (the stop button still sends it; core answers `not_implemented` until P2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub queue_when_busy: bool,
    pub steer: bool,
}

impl Capabilities {
    pub const P1: Self = Self {
        queue_when_busy: false,
        steer: false,
    };
    pub const P3: Self = Self {
        queue_when_busy: true,
        steer: true,
    };
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::P1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootState {
    Booting,
    Ready,
    StorageError { code: String, path: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResyncState {
    Idle,
    /// A `seq` gap was seen; one `GetSnapshot` is in flight (PROTOCOL.md).
    /// Old state is kept so the UI does not flash.
    Resyncing {
        expected: u64,
        got: u64,
    },
}

/// `LoadHistory.limit` per page (PROTOCOL.md: newest `limit` messages before
/// the cursor, returned oldest → newest).
pub const HISTORY_PAGE: u32 = 50;

/// Crash-recovery banner input (§6.12). No protocol event carries this yet;
/// the host fills it from the core boot report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryReport {
    pub sessions_checked: u32,
    pub interrupted_runs: u32,
    pub uncertain_items: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnnouncementPriority {
    Low,
    Medium,
    High,
}

/// Announcement category; same-kind announcements within
/// [`ANNOUNCE_MERGE_MS`] are merged (§8.2 "同一类 2 秒内合并").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnnouncementKind {
    RunStarted,
    RunFinished,
    Approval,
    Retrying,
    QueuePaused,
    /// "需要处理" group count changed (UI-REF-03).
    SidebarCount,
    Other,
}

pub const ANNOUNCE_MERGE_MS: u64 = 2_000;

/// VoiceOver announcement (§8.2). The host forwards these to the AppKit bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    pub kind: AnnouncementKind,
    pub priority: AnnouncementPriority,
    pub text: String,
    pub at_ms: u64,
}

impl PartialEq<&str> for Announcement {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastVm {
    pub level: NoticeLevel,
    pub text: String,
}

/// Why the composer is disabled (§6.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledReason {
    NoKey,
    StorageError,
    NoSession,
}

/// Composer mode used by the view to pick the button, hint and placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerMode {
    Idle,
    Running,
    Cancelling,
    Sending,
    Disabled(DisabledReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerKey {
    Enter,
    ShiftEnter,
    AltEnter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    Empty,
    InFlight,
    Busy,
    Disabled,
    NoSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// IME is composing: let the input method handle the key; send nothing.
    PassToIme,
    InsertNewline,
    Sent(CommandToken),
    Steered(CommandToken),
    Ignored(IgnoreReason),
}

pub struct AppState {
    pub boot: BootState,
    pub capabilities: Capabilities,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: Option<WorkspaceId>,
    pub sessions: BTreeMap<SessionId, SessionVm>,
    pub selected_session: Option<SessionId>,
    /// Column widths / visibility (§4); persisted via `Settings.ui_prefs` later.
    pub ui: crate::layout::UiPrefs,
    pub show_archived: bool,
    pub settings: SettingsVm,
    pub onboarding: Option<OnboardingVm>,
    pub palette: PaletteVm,
    pub quit: QuitState,
    pub recovery: Option<RecoveryReport>,
    pub toasts: Vec<ToastVm>,
    pub last_seq: u64,
    pub resync: ResyncState,
    /// A `GetSnapshot` is in flight (boot or gap); a second one is not sent.
    pub(crate) snapshot_pending: bool,
    /// At least one `Snapshot` was applied.
    pub synced: bool,
    pub(crate) now_ms: u64,
    /// Local time zone offset from UTC (host supplies it; wall-clock texts).
    pub(crate) utc_offset_ms: i64,
    /// Last "需要处理" count read out, so changes are announced once.
    attention_count: usize,
    pub(crate) announcements: Vec<Announcement>,
    /// When each kind was last handed to the screen reader (merge window).
    pub(crate) announced_at: HashMap<AnnouncementKind, u64>,
    pub(crate) outbox: Vec<Outgoing>,
    /// In-flight commands. The command copy is kept for R10 replay, except for
    /// `SetProviderKey`, whose key must not linger in UI memory.
    pub(crate) pending: HashMap<CommandToken, (Pending, Option<Command>)>,
    next_token: u64,
    pub(crate) ids: Box<dyn IdSource>,
    /// run → session, for events that only carry a run id.
    pub(crate) run_sessions: HashMap<RunId, SessionId>,
    /// tool call → (run, tool name), for approvals.
    pub(crate) tool_calls: HashMap<ToolCallId, (RunId, String)>,
    /// approval → (session, tool call).
    pub(crate) approvals: HashMap<ApprovalId, (SessionId, ToolCallId)>,
    pub(crate) pending_create: u32,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(Box::new(RandomIds::default()))
    }
}

impl AppState {
    pub fn new(ids: Box<dyn IdSource>) -> Self {
        Self {
            boot: BootState::Booting,
            capabilities: Capabilities::default(),
            workspaces: Vec::new(),
            active_workspace: None,
            sessions: BTreeMap::new(),
            selected_session: None,
            ui: crate::layout::UiPrefs::default(),
            show_archived: false,
            settings: SettingsVm::default(),
            onboarding: None,
            palette: PaletteVm::default(),
            quit: QuitState::Idle,
            recovery: None,
            toasts: Vec::new(),
            last_seq: 0,
            resync: ResyncState::Idle,
            snapshot_pending: false,
            synced: false,
            now_ms: 0,
            announcements: Vec::new(),
            utc_offset_ms: 0,
            attention_count: 0,
            announced_at: HashMap::new(),
            outbox: Vec::new(),
            pending: HashMap::new(),
            next_token: 0,
            ids,
            run_sessions: HashMap::new(),
            tool_calls: HashMap::new(),
            approvals: HashMap::new(),
            pending_create: 0,
        }
    }

    // ---------------------------------------------------------------- plumbing

    /// Local UTC offset used for "可在 14:05 后重试" and day buckets.
    pub fn set_utc_offset_ms(&mut self, offset_ms: i64) {
        self.utc_offset_ms = offset_ms;
    }

    pub fn utc_offset_ms(&self) -> i64 {
        self.utc_offset_ms
    }

    pub fn set_now_ms(&mut self, now_ms: u64) {
        let before = self.now_ms;
        self.now_ms = now_ms;
        // D2: a Retry-After wait that just ran out is announced once (polite).
        let ready = self.sessions.values().any(|vm| {
            vm.timeline.iter().any(|item| {
                matches!(item, TimelineItem::Error(e)
                    if e.retry_at_ms.is_some_and(|at| before < at && at <= now_ms))
            })
        });
        if ready {
            self.announce(
                AnnouncementKind::Other,
                AnnouncementPriority::Low,
                crate::i18n::t("announce.retry_ready").to_owned(),
            );
        }
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// Commands produced by intents since the last call, in order.
    pub fn take_outbox(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outbox)
    }

    /// Announcements ready to speak now. A kind spoken less than
    /// [`ANNOUNCE_MERGE_MS`] ago stays queued (and keeps merging newer text of
    /// the same kind) until the window passes; the host polls every second.
    pub fn take_announcements(&mut self) -> Vec<Announcement> {
        let now = self.now_ms;
        let mut ready = Vec::new();
        let mut held = Vec::new();
        for announcement in std::mem::take(&mut self.announcements) {
            let recent = announcement.kind != AnnouncementKind::Other
                && self
                    .announced_at
                    .get(&announcement.kind)
                    .is_some_and(|at| now.saturating_sub(*at) < ANNOUNCE_MERGE_MS);
            if recent {
                held.push(announcement);
            } else {
                self.announced_at.insert(announcement.kind, now);
                ready.push(announcement);
            }
        }
        self.announcements = held;
        ready
    }

    pub fn has_pending(&self, token: CommandToken) -> bool {
        self.pending.contains_key(&token)
    }

    /// Re-issues a command whose reply was lost in transport (R10): same token,
    /// same command, therefore the same `input_id` / `steer_id`.
    pub fn replay(&mut self, token: CommandToken) -> Option<Outgoing> {
        let (_, command) = self.pending.get(&token)?;
        Some(Outgoing {
            token,
            command: command.clone()?,
        })
    }

    pub(crate) fn push(&mut self, pending: Pending, command: Command) -> CommandToken {
        let keep = !matches!(command, Command::SetProviderKey { .. });
        self.next_token += 1;
        let token = CommandToken(self.next_token);
        self.pending
            .insert(token, (pending, keep.then(|| command.clone())));
        self.outbox.push(Outgoing { token, command });
        token
    }

    /// Boot: ask Core for the state to start from (PROTOCOL.md `GetSnapshot`).
    pub fn request_snapshot(&mut self) -> Option<CommandToken> {
        if self.snapshot_pending {
            return None;
        }
        self.snapshot_pending = true;
        Some(self.push(Pending::Snapshot, Command::GetSnapshot))
    }

    pub(crate) fn announce(
        &mut self,
        kind: AnnouncementKind,
        priority: AnnouncementPriority,
        text: String,
    ) {
        let now = self.now_ms;
        // Merge into a still-queued announcement of the same kind (latest text
        // wins, highest priority kept) instead of queueing a second one.
        if let Some(queued) = self
            .announcements
            .iter_mut()
            .rev()
            .find(|a| a.kind == kind && kind != AnnouncementKind::Other)
        {
            queued.text = text;
            queued.priority = queued.priority.max(priority);
            queued.at_ms = now;
            return;
        }
        self.announcements.push(Announcement {
            kind,
            priority,
            text,
            at_ms: now,
        });
    }

    pub fn selected(&self) -> Option<&SessionVm> {
        self.selected_session
            .as_ref()
            .and_then(|id| self.sessions.get(id))
    }

    pub(crate) fn selected_mut(&mut self) -> Option<&mut SessionVm> {
        let id = self.selected_session.clone()?;
        self.sessions.get_mut(&id)
    }

    pub fn active_runs(&self) -> Vec<RunId> {
        self.sessions
            .values()
            .filter_map(|session| session.running().map(|run| run.id.clone()))
            .collect()
    }

    fn session_for_run(&self, run: &RunId) -> Option<SessionId> {
        self.run_sessions.get(run).cloned()
    }

    fn run_mut(&mut self, run: &RunId) -> Option<(&mut SessionVm, RunId)> {
        let session = self.session_for_run(run)?;
        let vm = self.sessions.get_mut(&session)?;
        Some((vm, run.clone()))
    }

    // ------------------------------------------------------------------ events

    /// Applies one event. Duplicate or stale `seq`s are ignored (idempotent
    /// replay). A gap sends exactly one `GetSnapshot` (the event itself is
    /// still applied; the snapshot supersedes it). A requested `Snapshot`
    /// resets the cursor to its `last_seq`; afterwards only `seq > last_seq`
    /// is applied.
    pub fn apply(&mut self, envelope: EventEnvelope) {
        if let Event::Snapshot { .. } = &envelope.event {
            let requested = self.snapshot_pending;
            if !requested && self.last_seq != 0 && envelope.seq <= self.last_seq {
                return;
            }
        } else {
            if self.last_seq != 0 && envelope.seq <= self.last_seq {
                return;
            }
            if self.last_seq != 0 && envelope.seq != self.last_seq + 1 && !self.snapshot_pending {
                self.resync = ResyncState::Resyncing {
                    expected: self.last_seq + 1,
                    got: envelope.seq,
                };
                self.request_snapshot();
            }
            self.last_seq = envelope.seq;
        }
        self.apply_event(envelope.event, envelope.seq);
        self.announce_attention_count();
    }

    /// UI-REF-03: read out the new "需要处理" count when it changes.
    fn announce_attention_count(&mut self) {
        let count = self.sidebar().needs_attention.len();
        if count == self.attention_count {
            return;
        }
        self.attention_count = count;
        let text = crate::sidebar::SidebarVm::group_label(
            crate::sidebar::SidebarGroup::NeedsAttention,
            count,
        );
        self.announce(
            AnnouncementKind::SidebarCount,
            AnnouncementPriority::Low,
            text,
        );
    }

    fn apply_event(&mut self, event: Event, seq: u64) {
        let _ = seq;
        match event {
            Event::Snapshot {
                sessions,
                workspaces,
                settings,
                providers,
                active_runs,
                last_seq,
            } => self.apply_snapshot(
                sessions,
                workspaces,
                settings,
                providers,
                active_runs,
                last_seq,
            ),
            Event::HistoryLoaded {
                session,
                messages,
                next_before,
                runs,
            } => self.on_history_loaded(session, messages, next_before, runs),
            Event::SettingsUpdated { settings } => self.load_settings(settings),
            Event::ProviderKeyConfigured {
                provider,
                configured,
            } => self.on_key_configured(provider, configured),
            Event::WorkspaceAdded { workspace } => {
                if self.active_workspace.is_none() {
                    self.active_workspace = Some(workspace.id.clone());
                }
                self.workspaces.retain(|w| w.id != workspace.id);
                self.workspaces.push(workspace);
            }
            Event::WorkspaceRemoved { workspace } => {
                self.workspaces.retain(|w| w.id != workspace);
                if self.active_workspace.as_ref() == Some(&workspace) {
                    self.active_workspace = self.workspaces.first().map(|w| w.id.clone());
                }
            }
            Event::ProviderKeyTested {
                provider,
                ok,
                error_kind,
            } => self.on_key_tested(provider, ok, error_kind),
            Event::SessionCreated { session } => {
                let id = session.id.clone();
                if !self.sessions.contains_key(&id) {
                    let mut vm = SessionVm::new(session.clone());
                    // A session we just created is empty and fully loaded.
                    vm.history = HistoryState::Complete;
                    self.sessions.insert(id.clone(), vm);
                }
                self.adopt_session(session);
                if self.pending_create > 0 {
                    self.pending_create -= 1;
                    self.selected_session = Some(id);
                }
            }
            Event::SessionUpdated { session } => self.adopt_session(session),
            Event::SessionArchived { session, archived } => {
                // Pick the neighbour while the row is still in the visible order.
                if archived && self.selected_session.as_ref() == Some(&session) {
                    self.select_neighbour(&session);
                }
                if let Some(vm) = self.sessions.get_mut(&session) {
                    vm.session.archived = archived;
                }
            }
            Event::SessionDeleted { session } => {
                if self.selected_session.as_ref() == Some(&session) {
                    self.select_neighbour(&session);
                }
                self.sessions.remove(&session);
                self.run_sessions.retain(|_, s| s != &session);
                self.approvals.retain(|_, (s, _)| s != &session);
            }
            Event::QueueItemAdded { item } | Event::QueueItemUpdated { item } => {
                self.on_queue_item(item)
            }
            Event::QueueItemRemoved { session, item } => {
                if let Some(vm) = self.sessions.get_mut(&session) {
                    vm.queue.remove(&item);
                }
            }
            Event::QueuePaused { session, reason } => {
                if let Some(vm) = self.sessions.get_mut(&session) {
                    vm.queue_paused = Some(reason.clone());
                    let reason_key = match reason {
                        QueuePauseReason::User => "queue.pause.user",
                        QueuePauseReason::Failed => "queue.pause.failed",
                        QueuePauseReason::Cancelled => "queue.pause.cancelled",
                        QueuePauseReason::Interrupted => "queue.pause.interrupted",
                        QueuePauseReason::Uncertain => "queue.pause.uncertain",
                    };
                    self.announce(
                        AnnouncementKind::QueuePaused,
                        AnnouncementPriority::Medium,
                        tf("announce.queue_paused", &[("reason", t(reason_key))]),
                    );
                }
            }
            Event::QueueResumed { session } => {
                if let Some(vm) = self.sessions.get_mut(&session) {
                    vm.queue_paused = None;
                }
            }
            Event::RunStarted { run, input_id } => self.on_run_started(run, input_id),
            Event::RunStateChanged { run, state } => self.on_run_state(run, state),
            Event::RunFinished { run, outcome } => self.finish_run(&run, outcome),
            Event::SteerApplied { .. } => {
                // Steer bubbles land in P3 (§11 phase mapping).
            }
            Event::TextDelta { run, text } => {
                let now = self.now_ms;
                if let Some((vm, run)) = self.run_mut(&run) {
                    if let Some(r) = vm.runs.get_mut(&run)
                        && !r.phase.is_terminal()
                        && !matches!(r.phase, RunPhase::Cancelling)
                    {
                        r.phase = RunPhase::Streaming;
                    }
                    let message = vm.ensure_assistant(&run);
                    if let Some(start) = message.reasoning_started_ms
                        && message.reasoning_ms.is_none()
                    {
                        message.reasoning_ms = Some(now.saturating_sub(start));
                    }
                    message.text.push_str(&text);
                }
            }
            Event::ReasoningDelta { run, text } => {
                let now = self.now_ms;
                if let Some((vm, run)) = self.run_mut(&run) {
                    if let Some(r) = vm.runs.get_mut(&run)
                        && matches!(r.phase, RunPhase::Starting | RunPhase::Retrying { .. })
                    {
                        r.phase = RunPhase::Thinking;
                    }
                    let message = vm.ensure_assistant(&run);
                    message.reasoning_started_ms.get_or_insert(now);
                    message.reasoning.push_str(&text);
                }
            }
            Event::ProviderAttempt { .. } => {}
            Event::ProviderRetrying {
                run,
                n,
                delay_ms,
                reason,
            } => {
                let now = self.now_ms;
                let mut announce = false;
                if let Some((vm, run)) = self.run_mut(&run)
                    && let Some(r) = vm.runs.get_mut(&run)
                    && !r.phase.is_terminal()
                {
                    r.phase = RunPhase::Retrying {
                        attempt: n,
                        delay_ms,
                        reason,
                        next_at_ms: now + delay_ms,
                    };
                    // §8.2 / UI-VO-03: only the first and the last retry speak.
                    announce = n == 1 || n == crate::outcome::RETRY_MAX_ATTEMPTS;
                }
                if announce {
                    self.announce(
                        AnnouncementKind::Retrying,
                        AnnouncementPriority::Low,
                        tf("announce.retrying", &[("n", &n.to_string())]),
                    );
                }
            }
            Event::GenerationReset { run } => {
                // R7: drop the unfinished output, leave no partial text behind.
                if let Some((vm, run)) = self.run_mut(&run)
                    && let Some(message) = vm.assistant_mut(&run)
                {
                    message.text.clear();
                    message.reasoning.clear();
                }
            }
            Event::ToolCallProposed { call } => {
                self.tool_calls
                    .insert(call.id.clone(), (call.run_id.clone(), call.tool.clone()));
            }
            Event::ToolCallStateChanged { .. } | Event::ToolOutputChunk { .. } => {
                // Tool step rows are P2 (§6.5).
            }
            Event::ApprovalRequested { approval } => self.on_approval_requested(approval),
            Event::ApprovalResolved { approval, .. } => {
                if let Some((session, _)) = self.approvals.remove(&approval)
                    && let Some(vm) = self.sessions.get_mut(&session)
                {
                    vm.pending_approvals.remove(&approval);
                    if vm.pending_approvals.is_empty()
                        && let Some(id) = vm.active_run.clone()
                        && let Some(r) = vm.runs.get_mut(&id)
                        && r.phase == RunPhase::WaitingApproval
                    {
                        r.phase = RunPhase::Streaming;
                    }
                }
            }
            // Not emitted in P1; the ring stays hidden until it arrives.
            Event::ContextUsage {
                run,
                used_tokens,
                context_remaining_estimate,
            } => {
                if let Some((vm, _)) = self.run_mut(&run) {
                    vm.context.used_tokens = used_tokens;
                    vm.context.limit_tokens =
                        used_tokens.saturating_add(context_remaining_estimate);
                }
            }
            Event::ContextCompacted {
                run, after_tokens, ..
            } => {
                if let Some((vm, _)) = self.run_mut(&run) {
                    vm.context.compacted_count += 1;
                    vm.context.used_tokens = after_tokens;
                    vm.timeline.push(TimelineItem::Notice(NoticeVm {
                        level: NoticeLevel::Info,
                        code: "context_compacted".into(),
                        text: t("notice.compacted").into(),
                    }));
                }
            }
            Event::CheckpointCreated { .. }
            | Event::RestorePreviewed { .. }
            | Event::CheckpointRestored { .. } => {
                // Inspector / restore dialog are P2 (§6.10, §6.11).
            }
            Event::UsageRecorded { record } => {
                if let Some((vm, run)) = self.run_mut(&record.run_id)
                    && let Some(r) = vm.runs.get_mut(&run)
                {
                    r.usage = record.usage;
                }
            }
            Event::Notice { level, message, .. } => self.toasts.push(ToastVm {
                level,
                text: message,
            }),
            Event::ShutdownComplete {
                stopped,
                unfinished,
            } => self.on_shutdown_complete(stopped, unfinished),
        }
    }

    fn on_queue_item(&mut self, item: QueueItem) {
        let Some(vm) = self.sessions.get_mut(&item.session_id) else {
            return;
        };
        if item.state == QueueItemState::Started {
            Self::insert_user_message(vm, &item.input_id, &item.content.text, Some(&item.id));
        }
        vm.queue.insert(item.id.clone(), item);
    }

    /// Inserts the user message once per input id. `queue` is the queue item it
    /// came from; whether it shows "来自队列" depends on whether the user sent
    /// it while the session was idle.
    fn insert_user_message(
        vm: &mut SessionVm,
        input: &InputId,
        text: &str,
        queue: Option<&QueueItemId>,
    ) {
        let direct = vm
            .accepted_inputs
            .iter()
            .find(|(id, _, _)| id == input)
            .map(|(_, _, direct)| *direct);
        vm.accepted_inputs.retain(|(id, _, _)| id != input);
        if vm.has_user_message(input) {
            return;
        }
        let origin = match (direct, queue) {
            (Some(false), Some(item)) | (None, Some(item)) => MessageOrigin::Queue(item.clone()),
            _ => MessageOrigin::Direct,
        };
        vm.timeline.push(TimelineItem::User(UserMessageVm {
            input_id: input.clone(),
            ordinal: None,
            text: text.to_owned(),
            origin,
        }));
    }

    fn on_run_started(&mut self, run: Run, input_id: InputId) {
        let session = run.session_id.clone();
        self.run_sessions.insert(run.id.clone(), session.clone());
        let Some(vm) = self.sessions.get_mut(&session) else {
            return;
        };
        // User message: from the queue item if there is one (P3), otherwise
        // the accepted / in-flight `Send` with this `input_id` (idle send, no
        // QueueItem in P1). `RunStarted` can arrive before `Accepted`.
        let from_queue = run
            .queue_item_id
            .as_ref()
            .and_then(|id| vm.queue.get(id))
            .map(|item| (item.content.text.clone(), item.id.clone()));
        match from_queue {
            Some((text, item)) => Self::insert_user_message(vm, &input_id, &text, Some(&item)),
            None => {
                let accepted = vm
                    .accepted_inputs
                    .iter()
                    .find(|(id, _, _)| id == &input_id)
                    .map(|(_, text, _)| text.clone());
                let in_flight = match &vm.composer.in_flight {
                    Some(InFlight::Send {
                        input_id: id, text, ..
                    }) if id == &input_id => Some(text.clone()),
                    _ => None,
                };
                if let Some(text) = accepted.or(in_flight) {
                    Self::insert_user_message(vm, &input_id, &text, None);
                }
            }
        }
        vm.runs.insert(run.id.clone(), RunVm::from_run(&run));
        vm.active_run = Some(run.id);
        self.announce(
            AnnouncementKind::RunStarted,
            AnnouncementPriority::Low,
            t("announce.run.started").into(),
        );
    }

    fn on_run_state(&mut self, run: RunId, state: RunState) {
        // Terminal states carry no outcome here; `RunFinished{outcome}` ends
        // the run.
        if matches!(
            state,
            RunState::Succeeded | RunState::Failed | RunState::Cancelled | RunState::Interrupted
        ) {
            return;
        }
        let Some((vm, run)) = self.run_mut(&run) else {
            return;
        };
        let Some(r) = vm.runs.get_mut(&run) else {
            return;
        };
        if r.phase.is_terminal() {
            return;
        }
        r.phase = match state {
            RunState::Running => match &r.phase {
                RunPhase::Thinking | RunPhase::Streaming => r.phase.clone(),
                _ if vm
                    .timeline
                    .iter()
                    .any(|i| matches!(i, TimelineItem::Assistant(m) if m.run_id == run)) =>
                {
                    RunPhase::Streaming
                }
                _ => RunPhase::Starting,
            },
            RunState::WaitingApproval => RunPhase::WaitingApproval,
            RunState::Retrying => match &r.phase {
                RunPhase::Retrying { .. } => r.phase.clone(),
                _ => RunPhase::Retrying {
                    attempt: 0,
                    delay_ms: 0,
                    reason: String::new(),
                    next_at_ms: 0,
                },
            },
            RunState::Cancelling => RunPhase::Cancelling,
            _ => unreachable!("terminal states handled above"),
        };
    }

    /// Terminal state is written once (R4 "终态只写一次"); later duplicates are
    /// ignored.
    fn finish_run(&mut self, run: &RunId, outcome: RunOutcome) {
        let now = self.now_ms;
        let Some((vm, run)) = self.run_mut(run) else {
            return;
        };
        let Some(r) = vm.runs.get_mut(&run) else {
            return;
        };
        if r.phase.is_terminal() {
            return;
        }
        r.phase = RunPhase::Finished(outcome.clone());
        let finished_at = *r.finished_at_ms.get_or_insert(now);
        let elapsed_secs = finished_at.saturating_sub(r.started_at_ms) / 1000;
        if vm.active_run.as_ref() == Some(&run) {
            vm.active_run = None;
        }
        let cancelled = matches!(outcome, RunOutcome::Cancelled { .. });
        if let Some(message) = vm.assistant_mut(&run) {
            message.streaming = false;
            message.stopped = cancelled;
            if let Some(start) = message.reasoning_started_ms
                && message.reasoning_ms.is_none()
            {
                message.reasoning_ms = Some(now.saturating_sub(start));
            }
        }
        let failure = crate::outcome::failure_of(&outcome);
        let error_kind = failure.as_ref().map(|f| f.category);
        let failure_title = failure.as_ref().map(crate::outcome::card_title);
        vm.last_error_source = failure.as_ref().and_then(|f| f.source.clone());
        // §6.7: the error card sits after the run's content, before the divider.
        if let Some(failure) = failure {
            vm.timeline.push(TimelineItem::Error(ErrorVm {
                run_id: run.clone(),
                kind: failure.category,
                // Only a long Retry-After (> 60 s, R7a) gates the retry button;
                // it counts from when the run ended.
                retry_at_ms: failure
                    .retry_after_ms
                    .filter(|ms| *ms > crate::outcome::LONG_RETRY_AFTER_MS)
                    .map(|ms| finished_at + ms),
                failure,
            }));
        }
        vm.timeline.push(TimelineItem::RunDivider {
            run_id: run.clone(),
            outcome: outcome.clone(),
        });
        vm.last_error = error_kind;
        vm.last_outcome = Some(outcome.clone());
        // R2 / R2a: a non-succeeded run pauses the queue when items wait. Core
        // confirms with `QueuePaused`; set it now so the sidebar does not lag.
        if vm.queue_paused.is_none() && vm.pending_queue_len() > 0 {
            vm.queue_paused = match &outcome {
                RunOutcome::Succeeded => None,
                RunOutcome::Cancelled { .. } => Some(QueuePauseReason::Cancelled),
                o if crate::outcome::is_interrupted(o) => Some(QueuePauseReason::Interrupted),
                RunOutcome::Failed { .. } => Some(QueuePauseReason::Failed),
            };
        }
        let text = match (&outcome, error_kind) {
            (RunOutcome::Succeeded, _) => tf(
                "announce.run.succeeded",
                &[("duration", &crate::i18n::format_elapsed(elapsed_secs))],
            ),
            (RunOutcome::Cancelled { .. }, _) => t("announce.run.cancelled").into(),
            (_, _) => tf(
                "announce.run.failed",
                &[(
                    "title",
                    failure_title.unwrap_or_else(|| t(ErrorKind::Unknown.title_key())),
                )],
            ),
        };
        let priority = if error_kind.is_some() {
            AnnouncementPriority::High
        } else {
            AnnouncementPriority::Medium
        };
        self.announce(AnnouncementKind::RunFinished, priority, text);
    }

    fn on_approval_requested(&mut self, approval: Approval) {
        let Some((run, tool)) = self.tool_calls.get(&approval.tool_call_id).cloned() else {
            return;
        };
        let Some(session) = self.session_for_run(&run) else {
            return;
        };
        if let Some(vm) = self.sessions.get_mut(&session) {
            vm.pending_approvals.insert(approval.id.clone());
            if let Some(r) = vm.runs.get_mut(&run)
                && !r.phase.is_terminal()
            {
                r.phase = RunPhase::WaitingApproval;
            }
        }
        self.approvals
            .insert(approval.id, (session, approval.tool_call_id));
        self.announce(
            AnnouncementKind::Approval,
            AnnouncementPriority::High,
            tf("announce.approval", &[("action", &tool)]),
        );
    }

    /// Tool name of the earliest pending approval of a session (sidebar text).
    pub fn first_approval_action(&self, session: &SessionId) -> Option<String> {
        let vm = self.sessions.get(session)?;
        let approval = vm.pending_approvals.iter().next()?;
        let (_, call) = self.approvals.get(approval)?;
        self.tool_calls.get(call).map(|(_, tool)| tool.clone())
    }

    // ------------------------------------------------------- command results

    pub fn apply_result(&mut self, token: CommandToken, result: CommandResult) {
        let Some((pending, _)) = self.pending.remove(&token) else {
            return;
        };
        let rejected = match &result {
            CommandResult::Accepted { .. } => None,
            CommandResult::Rejected { code, message } => Some((code.clone(), message.clone())),
        };
        match pending {
            Pending::Send { session, input_id } => self.on_send_result(session, input_id, rejected),
            Pending::Steer { session, steer_id } => {
                let Some(vm) = self.sessions.get_mut(&session) else {
                    return;
                };
                if !matches!(&vm.composer.in_flight, Some(InFlight::Steer { steer_id: id, .. }) if id == &steer_id)
                {
                    return;
                }
                let Some(InFlight::Steer { text, .. }) = vm.composer.in_flight.take() else {
                    return;
                };
                match rejected {
                    None => vm.composer.draft.clear(),
                    Some((code, message)) => {
                        vm.composer.draft = text;
                        vm.composer.error = Some(not_sent(&code, &message));
                    }
                }
            }
            Pending::Cancel { session, run } => {
                if let Some((code, message)) = rejected {
                    let finished =
                        matches!(code, RejectCode::AlreadyFinished | RejectCode::RunFinished);
                    if let Some(r) = self
                        .sessions
                        .get_mut(&session)
                        .and_then(|vm| vm.runs.get_mut(&run))
                        && !finished
                    {
                        r.cancel_requested = false;
                    }
                    if !finished {
                        self.toast_rejected(&code, &message);
                    }
                }
            }
            Pending::CreateSession => {
                if let Some((code, message)) = rejected {
                    self.pending_create = self.pending_create.saturating_sub(1);
                    self.toast_rejected(&code, &message);
                }
            }
            Pending::Rename { session, previous } => {
                if let Some((code, message)) = rejected {
                    if let Some(vm) = self.sessions.get_mut(&session) {
                        vm.session.title = previous;
                    }
                    self.toast_rejected(&code, &message);
                }
            }
            Pending::Archive { .. }
            | Pending::Delete { .. }
            | Pending::AddWorkspace
            | Pending::Retry => {
                if let Some((code, message)) = rejected {
                    self.toast_rejected(&code, &message);
                }
            }
            Pending::LoadHistory { session, before } => {
                // Accepted: the page arrives as `HistoryLoaded`.
                if rejected.is_some()
                    && let Some(vm) = self.sessions.get_mut(&session)
                {
                    vm.history = match before {
                        // An older page failed: keep what is shown, allow retry.
                        Some(_) => HistoryState::HasMore,
                        None => HistoryState::Error,
                    };
                }
            }
            Pending::Snapshot => {
                if let Some((code, message)) = rejected {
                    self.snapshot_pending = false;
                    self.toast_rejected(&code, &message);
                }
            }
            Pending::SetKey { provider } => {
                if let Some((code, message)) = rejected {
                    self.on_key_set_rejected(provider, code, message);
                }
            }
            Pending::TestKey { provider } => {
                if let Some((code, message)) = rejected {
                    self.on_key_test_rejected(provider, code, message);
                }
            }
            Pending::UpdateSettings { previous } => {
                if let Some((code, message)) = rejected {
                    self.ui.apply_map(&previous.ui_prefs);
                    self.settings.settings = Some(previous);
                    self.settings.error = Some(not_sent(&code, &message));
                }
            }
            Pending::SetPermissionMode { session, previous } => {
                if let Some((code, message)) = rejected {
                    if let Some(vm) = self.sessions.get_mut(&session) {
                        vm.session.permission_mode = previous;
                    }
                    self.toast_rejected(&code, &message);
                }
            }
            Pending::Shutdown => {
                if rejected.is_some() {
                    self.on_shutdown_rejected();
                }
            }
        }
    }

    /// The host could not deliver a command at all (runtime closed). For
    /// requests with a visible pending state this behaves like a rejection.
    pub fn transport_failed(&mut self, token: CommandToken) {
        self.apply_result(
            token,
            CommandResult::Rejected {
                code: RejectCode::Unavailable,
                message: t("reject.transport").into(),
            },
        );
    }

    fn on_send_result(
        &mut self,
        session: SessionId,
        input_id: InputId,
        rejected: Option<(RejectCode, String)>,
    ) {
        let Some(vm) = self.sessions.get_mut(&session) else {
            return;
        };
        let matches_in_flight = matches!(
            &vm.composer.in_flight,
            Some(InFlight::Send { input_id: id, .. }) if id == &input_id
        );
        if !matches_in_flight {
            return;
        }
        let Some(InFlight::Send { text, queued, .. }) = vm.composer.in_flight.take() else {
            return;
        };
        match rejected {
            None => {
                if !vm.has_user_message(&input_id) {
                    vm.accepted_inputs.push_back((input_id, text, !queued));
                }
                // Keep anything typed while the send was in flight? The
                // composer is read-only while sending (§6.9), so clear.
                vm.composer.draft.clear();
                vm.composer.error = None;
            }
            Some((code, message)) => {
                vm.composer.draft = text;
                vm.composer.error = Some(not_sent(&code, &message));
            }
        }
    }

    pub(crate) fn toast_rejected(&mut self, code: &RejectCode, message: &str) {
        self.toasts.push(ToastVm {
            level: NoticeLevel::Error,
            text: not_sent(code, message),
        });
    }

    // ------------------------------------------------------- session intents

    /// Selects a session; issues `LoadHistory` the first time it is opened.
    pub fn select_session(&mut self, id: &SessionId) -> bool {
        if !self.sessions.contains_key(id) {
            return false;
        }
        self.selected_session = Some(id.clone());
        let needs_history = self.sessions[id].history == HistoryState::NotLoaded;
        if needs_history {
            if let Some(vm) = self.sessions.get_mut(id) {
                vm.history = HistoryState::Loading;
            }
            self.push_load_history(id, None);
        }
        true
    }

    fn push_load_history(&mut self, id: &SessionId, before: Option<u64>) -> CommandToken {
        self.push(
            Pending::LoadHistory {
                session: id.clone(),
                before,
            },
            Command::LoadHistory {
                session: id.clone(),
                before,
                limit: HISTORY_PAGE,
            },
        )
    }

    /// Scrolled to the top of the timeline: fetch the next older page
    /// (`LoadHistory{before: next_before}`). One page in flight at a time;
    /// nothing once the oldest message is shown.
    pub fn load_older(&mut self) -> Option<CommandToken> {
        let vm = self.selected_mut()?;
        if vm.history != HistoryState::HasMore {
            return None;
        }
        let before = vm.history_before?;
        vm.history = HistoryState::LoadingOlder;
        let id = vm.session.id.clone();
        Some(self.push_load_history(&id, Some(before)))
    }

    /// Retry after `HistoryState::Error` (§6.3 "重试").
    pub fn retry_history(&mut self, id: &SessionId) {
        if let Some(vm) = self.sessions.get_mut(id)
            && vm.history == HistoryState::Error
        {
            vm.history = HistoryState::NotLoaded;
            self.select_session(id);
        }
    }

    fn select_neighbour(&mut self, removed: &SessionId) {
        let order = self.navigation_order();
        let position = order.iter().position(|id| id == removed);
        let next = position.and_then(|p| {
            order
                .get(p + 1)
                .or_else(|| p.checked_sub(1).and_then(|q| order.get(q)))
                .cloned()
        });
        self.selected_session = next.filter(|id| id != removed);
    }

    /// ⌘[ / ⌘] (§6.1). `forward = true` is the next session.
    pub fn select_adjacent(&mut self, forward: bool) -> bool {
        let order = self.navigation_order();
        if order.is_empty() {
            return false;
        }
        let current = self
            .selected_session
            .as_ref()
            .and_then(|id| order.iter().position(|o| o == id));
        let next = match (current, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(order.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        let id = order[next].clone();
        self.select_session(&id)
    }

    /// "＋ 新会话 ⌘N". Needs an active workspace and settings (model and
    /// default permission mode).
    pub fn create_session(&mut self) -> Option<CommandToken> {
        let workspace = self.active_workspace.clone()?;
        let settings = self.settings.settings.as_ref()?;
        let command = Command::CreateSession {
            workspace,
            title: t("session.default_title").into(),
            model: settings.model.clone(),
            permission_mode: settings.permission_mode.clone(),
        };
        self.pending_create += 1;
        Some(self.push(Pending::CreateSession, command))
    }

    /// Inline rename; empty titles are not submitted (§6.1).
    pub fn rename_session(&mut self, id: &SessionId, title: &str) -> Option<CommandToken> {
        let title = title.trim();
        if title.is_empty() {
            return None;
        }
        let vm = self.sessions.get_mut(id)?;
        if vm.session.title == title {
            return None;
        }
        let previous = std::mem::replace(&mut vm.session.title, title.to_owned());
        Some(self.push(
            Pending::Rename {
                session: id.clone(),
                previous,
            },
            Command::RenameSession {
                session: id.clone(),
                title: title.to_owned(),
            },
        ))
    }

    pub fn archive_session(&mut self, id: &SessionId, archived: bool) -> Option<CommandToken> {
        self.sessions.get(id)?;
        Some(self.push(
            Pending::Archive {
                session: id.clone(),
            },
            Command::ArchiveSession {
                session: id.clone(),
                archived,
            },
        ))
    }

    /// Deleting a running session is not allowed (UI-SB-05: menu item disabled).
    pub fn can_delete(&self, id: &SessionId) -> bool {
        self.sessions.get(id).is_some_and(|vm| !vm.is_busy())
    }

    /// Call after the user confirmed the delete dialog.
    pub fn delete_session(&mut self, id: &SessionId) -> Option<CommandToken> {
        if !self.can_delete(id) {
            return None;
        }
        Some(self.push(
            Pending::Delete {
                session: id.clone(),
            },
            Command::DeleteSession {
                session: id.clone(),
            },
        ))
    }

    pub fn toggle_sidebar(&mut self) {
        self.ui.sidebar_visible = !self.ui.sidebar_visible;
    }

    pub fn set_show_archived(&mut self, show: bool) {
        self.show_archived = show;
    }

    pub fn dismiss_recovery(&mut self) {
        self.recovery = None;
    }

    pub fn take_toasts(&mut self) -> Vec<ToastVm> {
        std::mem::take(&mut self.toasts)
    }

    /// Changes a session's permission mode. Switching to `Unrestricted` opens a
    /// confirm dialog first (§6.2, UI-TB-02); call `confirm_dialog_accept`.
    pub fn request_session_permission_mode(
        &mut self,
        id: &SessionId,
        mode: PermissionMode,
    ) -> Option<CommandToken> {
        let vm = self.sessions.get(id)?;
        if vm.session.permission_mode == mode {
            return None;
        }
        if mode == PermissionMode::Unrestricted {
            self.settings.confirm = Some(ConfirmDialog::UnrestrictedSession(id.clone()));
            return None;
        }
        self.send_session_permission_mode(id, mode)
    }

    pub(crate) fn send_session_permission_mode(
        &mut self,
        id: &SessionId,
        mode: PermissionMode,
    ) -> Option<CommandToken> {
        let vm = self.sessions.get_mut(id)?;
        let previous = std::mem::replace(&mut vm.session.permission_mode, mode.clone());
        Some(self.push(
            Pending::SetPermissionMode {
                session: id.clone(),
                previous,
            },
            Command::SetPermissionMode {
                session: id.clone(),
                mode,
            },
        ))
    }

    // ------------------------------------------------------ composer intents

    pub fn has_api_key(&self) -> bool {
        self.settings.has_any_key()
    }

    pub fn composer_mode(&self) -> ComposerMode {
        if matches!(self.boot, BootState::StorageError { .. }) {
            return ComposerMode::Disabled(DisabledReason::StorageError);
        }
        let Some(vm) = self.selected() else {
            return ComposerMode::Disabled(DisabledReason::NoSession);
        };
        if !self.has_api_key() {
            return ComposerMode::Disabled(DisabledReason::NoKey);
        }
        if vm.composer.in_flight.is_some() {
            return ComposerMode::Sending;
        }
        match vm.running() {
            Some(run) if run.phase == RunPhase::Cancelling => ComposerMode::Cancelling,
            Some(_) => ComposerMode::Running,
            None => ComposerMode::Idle,
        }
    }

    /// Hint key under the composer (§6.9 / §10).
    pub fn composer_hint_key(&self) -> &'static str {
        match self.composer_mode() {
            ComposerMode::Running | ComposerMode::Cancelling => {
                if self.capabilities.queue_when_busy {
                    "composer.hint.running"
                } else {
                    "composer.hint.running_p1"
                }
            }
            ComposerMode::Disabled(DisabledReason::NoKey) => "composer.no_key",
            _ => "composer.hint.idle",
        }
    }

    /// Whether the primary button can be pressed (send needs non-empty text).
    pub fn can_send(&self) -> bool {
        let Some(vm) = self.selected() else {
            return false;
        };
        let text_ok = !vm.composer.draft.trim().is_empty();
        match self.composer_mode() {
            ComposerMode::Idle => text_ok,
            ComposerMode::Running | ComposerMode::Cancelling => {
                text_ok && self.capabilities.queue_when_busy
            }
            _ => false,
        }
    }

    pub fn set_draft(&mut self, text: impl Into<String>) {
        if let Some(vm) = self.selected_mut()
            && vm.composer.in_flight.is_none()
        {
            vm.composer.draft = text.into();
            vm.composer.error = None;
        }
    }

    pub fn set_ime_composing(&mut self, composing: bool) {
        if let Some(vm) = self.selected_mut() {
            vm.composer.ime_composing = composing;
        }
    }

    /// Empty-state example: fills the composer, never sends (UI-TL-04).
    pub fn fill_example(&mut self, index: usize) -> bool {
        let key = match index {
            1 => "empty.example.1",
            2 => "empty.example.2",
            3 => "empty.example.3",
            _ => return false,
        };
        self.set_draft(t(key));
        true
    }

    /// Composer key handling (§6.9 发送规则, §7.1). The view calls this for
    /// Enter / ⇧Enter / ⌥Enter before the text input sees them.
    pub fn composer_key(&mut self, key: ComposerKey) -> KeyOutcome {
        let mode = self.composer_mode();
        let caps = self.capabilities;
        let Some(vm) = self.selected() else {
            return KeyOutcome::Ignored(IgnoreReason::NoSession);
        };
        if vm.composer.ime_composing {
            return KeyOutcome::PassToIme;
        }
        if key == ComposerKey::ShiftEnter {
            return KeyOutcome::InsertNewline;
        }
        match mode {
            ComposerMode::Disabled(_) => return KeyOutcome::Ignored(IgnoreReason::Disabled),
            ComposerMode::Sending => return KeyOutcome::Ignored(IgnoreReason::InFlight),
            _ => {}
        }
        if vm.composer.draft.trim().is_empty() {
            return KeyOutcome::Ignored(IgnoreReason::Empty);
        }
        let busy = matches!(mode, ComposerMode::Running | ComposerMode::Cancelling);
        match (key, busy) {
            (_, false) => KeyOutcome::Sent(self.send_draft()),
            (ComposerKey::Enter, true) if caps.queue_when_busy => {
                KeyOutcome::Sent(self.send_draft())
            }
            (ComposerKey::AltEnter, true) if caps.steer => match self.steer_draft() {
                Some(token) => KeyOutcome::Steered(token),
                None => KeyOutcome::Ignored(IgnoreReason::Busy),
            },
            _ => KeyOutcome::Ignored(IgnoreReason::Busy),
        }
    }

    /// Primary button click: same as Enter when idle, Stop when running.
    pub fn primary_button(&mut self) -> Option<CommandToken> {
        match self.composer_mode() {
            ComposerMode::Running => self.stop_run(),
            ComposerMode::Idle => match self.composer_key(ComposerKey::Enter) {
                KeyOutcome::Sent(token) => Some(token),
                _ => None,
            },
            _ => None,
        }
    }

    fn send_draft(&mut self) -> CommandToken {
        let input_id = InputId(self.ids.next_id("in"));
        let vm = self.selected_mut().expect("checked by composer_key");
        let session = vm.session.id.clone();
        let text = vm.composer.draft.clone();
        let queued = vm.is_busy();
        vm.composer.error = None;
        vm.composer.in_flight = Some(InFlight::Send {
            input_id: input_id.clone(),
            text: text.clone(),
            queued,
        });
        self.push(
            Pending::Send {
                session: session.clone(),
                input_id: input_id.clone(),
            },
            Command::Send {
                session,
                input_id,
                content: Content {
                    text,
                    attachments: Vec::new(),
                },
            },
        )
    }

    fn steer_draft(&mut self) -> Option<CommandToken> {
        let steer_id = SteerId(self.ids.next_id("steer"));
        let vm = self.selected_mut()?;
        let run = vm.running()?.id.clone();
        let session = vm.session.id.clone();
        let text = vm.composer.draft.clone();
        vm.composer.error = None;
        vm.composer.in_flight = Some(InFlight::Steer {
            steer_id: steer_id.clone(),
            text: text.clone(),
        });
        Some(self.push(
            Pending::Steer {
                session,
                steer_id: steer_id.clone(),
            },
            Command::Steer {
                run,
                steer_id,
                content: Content {
                    text,
                    attachments: Vec::new(),
                },
            },
        ))
    }

    /// ⌘. — sends `CancelRun` once; repeated presses while cancelling or after
    /// it was sent do nothing (UI-CP-04).
    pub fn stop_run(&mut self) -> Option<CommandToken> {
        let vm = self.selected_mut()?;
        let session = vm.session.id.clone();
        let run = vm.active_run.clone()?;
        let r = vm.runs.get_mut(&run)?;
        if r.phase.is_terminal() || r.phase == RunPhase::Cancelling || r.cancel_requested {
            return None;
        }
        r.cancel_requested = true;
        Some(self.push(
            Pending::Cancel {
                session,
                run: run.clone(),
            },
            Command::CancelRun { run },
        ))
    }

    /// Workspace from the onboarding folder picker or ⌘O (G1).
    pub fn add_workspace(&mut self, path: PathBuf) -> CommandToken {
        self.push(Pending::AddWorkspace, Command::AddWorkspace { path })
    }
}

/// "没有发送：<原因>" with a localised reason for known reject codes.
pub(crate) fn not_sent(code: &RejectCode, message: &str) -> String {
    let key = match code {
        RejectCode::RunFinished => "reject.run_finished",
        RejectCode::AlreadyFinished => "reject.already_finished",
        RejectCode::ApprovalClosed => "reject.approval_closed",
        RejectCode::Conflict => "reject.conflict",
        RejectCode::NotFound => "reject.not_found",
        RejectCode::InvalidState => "reject.invalid_state",
        RejectCode::Storage => "reject.storage",
        RejectCode::Unavailable => "reject.unavailable",
        RejectCode::NotImplemented => "reject.not_implemented",
    };
    let reason = if *code == RejectCode::Unavailable && message == t("reject.transport") {
        t("reject.transport")
    } else {
        t(key)
    };
    tf("composer.not_sent", &[("reason", reason)])
}
