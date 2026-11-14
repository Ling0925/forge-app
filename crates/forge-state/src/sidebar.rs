//! Sidebar derivation (§6.1): groups 需要处理 / 进行中 / 最近, row status text
//! and accessible labels. Derived on demand from `AppState`; nothing cached.

use crate::app::AppState;
use crate::i18n::{format_elapsed, t, tf};
use crate::session::{ErrorKind, RunPhase, SessionVm};
use forge_protocol::{QueuePauseReason, RunOutcome, SessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarGroup {
    NeedsAttention,
    InProgress,
    Recent,
    Archived,
}

impl SidebarGroup {
    pub fn label_key(self) -> &'static str {
        match self {
            Self::NeedsAttention => "sidebar.group.attention",
            Self::InProgress => "sidebar.group.running",
            Self::Recent => "sidebar.group.recent",
            Self::Archived => "sidebar.archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Running { started_at_ms: u64 },
    WaitingApproval,
    Cancelling,
}

/// Fixed status icon set (§3.4); every state has icon + text (UI-SB-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusIcon {
    None,
    Spinner,
    Approval,
    Failed,
    Stopped,
    Interrupted,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRowVm {
    pub id: SessionId,
    pub title: String,
    pub activity: Activity,
    pub approvals: u32,
    pub uncertain: u32,
    pub queue_paused: Option<QueuePauseReason>,
    pub last_outcome: Option<RunOutcome>,
    pub last_error: Option<ErrorKind>,
    pub group: SidebarGroup,
    pub icon: StatusIcon,
    pub status_text: String,
    pub a11y_label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarVm {
    pub visible: bool,
    pub needs_attention: Vec<SessionRowVm>,
    pub in_progress: Vec<SessionRowVm>,
    pub recent: Vec<SessionRowVm>,
    pub archived: Vec<SessionRowVm>,
    pub archived_count: usize,
    pub show_archived: bool,
}

impl SidebarVm {
    /// "还没有会话" empty state.
    pub fn is_empty(&self) -> bool {
        self.needs_attention.is_empty() && self.in_progress.is_empty() && self.recent.is_empty()
    }

    pub fn group_label(group: SidebarGroup, count: usize) -> String {
        tf(
            "a11y.sidebar.group",
            &[("group", t(group.label_key())), ("n", &count.to_string())],
        )
    }
}

impl AppState {
    fn in_active_workspace(&self, vm: &SessionVm) -> bool {
        self.active_workspace
            .as_ref()
            .is_none_or(|w| &vm.session.workspace_id == w)
    }

    pub fn session_row(&self, vm: &SessionVm) -> SessionRowVm {
        let running = vm.running();
        let activity = match running.map(|r| &r.phase) {
            None => Activity::Idle,
            Some(RunPhase::Cancelling) => Activity::Cancelling,
            Some(RunPhase::WaitingApproval) => Activity::WaitingApproval,
            Some(_) => Activity::Running {
                started_at_ms: running.map(|r| r.started_at_ms).unwrap_or_default(),
            },
        };
        let approvals = vm.pending_approvals.len() as u32;
        let uncertain = vm.uncertain_count() as u32;
        let pending = vm.pending_queue_len();
        let paused_attention = matches!(
            vm.queue_paused,
            Some(QueuePauseReason::Failed | QueuePauseReason::Cancelled)
        ) && pending > 0;
        let group = if vm.session.archived {
            SidebarGroup::Archived
        } else if approvals > 0 || uncertain > 0 || paused_attention {
            SidebarGroup::NeedsAttention
        } else if activity != Activity::Idle {
            SidebarGroup::InProgress
        } else {
            SidebarGroup::Recent
        };
        let error_label = t(vm.last_error.unwrap_or(ErrorKind::Unknown).label_key());
        let (icon, status_text) = if approvals > 0 {
            let action = self
                .first_approval_action(&vm.session.id)
                .unwrap_or_default();
            (
                StatusIcon::Approval,
                tf("row.waiting_approval", &[("action", &action)]),
            )
        } else if uncertain > 0 {
            (
                StatusIcon::Uncertain,
                tf("row.uncertain", &[("n", &uncertain.to_string())]),
            )
        } else {
            match activity {
                Activity::Cancelling => (StatusIcon::Spinner, t("row.cancelling").into()),
                Activity::Running { started_at_ms } => {
                    let secs = self.now_ms.saturating_sub(started_at_ms) / 1000;
                    (
                        StatusIcon::Spinner,
                        tf("row.running", &[("elapsed", &format_elapsed(secs))]),
                    )
                }
                Activity::WaitingApproval => (StatusIcon::Approval, String::new()),
                _ => match (&vm.last_outcome, vm.queue_paused.as_ref()) {
                    (Some(RunOutcome::Failed), Some(QueuePauseReason::Failed)) => (
                        StatusIcon::Failed,
                        tf("row.failed_paused", &[("error_kind", error_label)]),
                    ),
                    (Some(RunOutcome::Failed), _) => (
                        StatusIcon::Failed,
                        tf("row.failed", &[("error_kind", error_label)]),
                    ),
                    (Some(RunOutcome::Cancelled), _) => {
                        (StatusIcon::Stopped, t("row.cancelled").into())
                    }
                    (Some(RunOutcome::Interrupted), _) => {
                        (StatusIcon::Interrupted, t("row.interrupted").into())
                    }
                    // Idle: relative time + diffstat need protocol timestamps
                    // and checkpoints (P2); show nothing for now.
                    _ => (StatusIcon::None, String::new()),
                },
            }
        };
        let a11y_label = if approvals > 0 {
            tf(
                "a11y.sidebar.row.approvals",
                &[
                    ("title", &vm.session.title),
                    ("status", &status_text),
                    ("n", &approvals.to_string()),
                ],
            )
        } else if status_text.is_empty() {
            vm.session.title.clone()
        } else {
            tf(
                "a11y.sidebar.row",
                &[("title", &vm.session.title), ("status", &status_text)],
            )
        };
        SessionRowVm {
            id: vm.session.id.clone(),
            title: vm.session.title.clone(),
            activity,
            approvals,
            uncertain,
            queue_paused: vm.queue_paused.clone(),
            last_outcome: vm.last_outcome.clone(),
            last_error: vm.last_error,
            group,
            icon,
            status_text,
            a11y_label,
            selected: self.selected_session.as_ref() == Some(&vm.session.id),
        }
    }

    pub fn sidebar(&self) -> SidebarVm {
        let mut sessions: Vec<&SessionVm> = self
            .sessions
            .values()
            .filter(|vm| self.in_active_workspace(vm))
            .collect();
        // Most recent activity first; id breaks ties for a stable order.
        sessions.sort_by(|a, b| {
            b.last_activity
                .cmp(&a.last_activity)
                .then_with(|| a.session.id.cmp(&b.session.id))
        });
        let mut vm = SidebarVm {
            visible: self.ui.sidebar_visible,
            needs_attention: Vec::new(),
            in_progress: Vec::new(),
            recent: Vec::new(),
            archived: Vec::new(),
            archived_count: 0,
            show_archived: self.show_archived,
        };
        for session in sessions {
            let row = self.session_row(session);
            match row.group {
                SidebarGroup::NeedsAttention => vm.needs_attention.push(row),
                SidebarGroup::InProgress => vm.in_progress.push(row),
                SidebarGroup::Recent => vm.recent.push(row),
                SidebarGroup::Archived => {
                    vm.archived_count += 1;
                    if self.show_archived {
                        vm.archived.push(row);
                    }
                }
            }
        }
        vm
    }

    /// Visual order of rows, used by ↑/↓, ⌘[ / ⌘] and the palette.
    pub fn navigation_order(&self) -> Vec<SessionId> {
        let sidebar = self.sidebar();
        sidebar
            .needs_attention
            .iter()
            .chain(&sidebar.in_progress)
            .chain(&sidebar.recent)
            .chain(&sidebar.archived)
            .map(|row| row.id.clone())
            .collect()
    }
}
