//! Snapshot / history synchronisation with Core (PROTOCOL.md `GetSnapshot`,
//! `LoadHistory`): boot state, seq-gap resync and timeline paging.

use crate::app::{AppState, ResyncState};
use crate::session::*;
use forge_protocol::*;
use std::collections::{BTreeMap, BTreeSet};

impl AppState {
    /// `Snapshot`: replaces sessions, workspaces, settings, key flags and
    /// active-run state; drafts, disclosure state and loaded timelines stay.
    /// The cursor becomes `last_seq`.
    pub(crate) fn apply_snapshot(
        &mut self,
        sessions: Vec<Session>,
        workspaces: Vec<Workspace>,
        settings: Settings,
        providers: Vec<ProviderKeyStatus>,
        active_runs: Vec<ActiveRun>,
        last_seq: u64,
    ) {
        let first = !self.synced;
        self.snapshot_pending = false;
        self.synced = true;
        self.resync = ResyncState::Idle;
        self.last_seq = last_seq;

        // Workspaces.
        self.workspaces = workspaces;
        let active_ok = self
            .active_workspace
            .as_ref()
            .is_some_and(|id| self.workspaces.iter().any(|w| &w.id == id));
        if !active_ok {
            self.active_workspace = self.workspaces.first().map(|w| w.id.clone());
        }

        // Settings + key flags (no secrets in the snapshot).
        self.load_settings(settings);
        self.settings.core_key_status.clear();
        for status in providers {
            self.on_key_status(status.provider, status.key_status);
        }
        self.announce_key_retry_result();

        // Sessions: replace rows, keep per-session UI state.
        let keep: BTreeSet<SessionId> = sessions.iter().map(|s| s.id.clone()).collect();
        self.sessions.retain(|id, _| keep.contains(id));
        self.run_sessions.retain(|_, s| keep.contains(s));
        self.approvals.retain(|_, (s, _)| keep.contains(s));
        if self
            .selected_session
            .as_ref()
            .is_some_and(|id| !keep.contains(id))
        {
            self.selected_session = None;
        }
        let mut adopt = Vec::new();
        for session in sessions {
            match self.sessions.get_mut(&session.id) {
                Some(vm) => vm.session = session.clone(),
                None => {
                    self.sessions
                        .insert(session.id.clone(), SessionVm::new(session.clone()));
                }
            }
            adopt.push(session);
        }

        // Active runs: the snapshot is authoritative. Error cards already in
        // memory are kept for runs the snapshot does not contradict (the
        // timeline is not cleared). Runs that finished during a seq gap are
        // restored after this snapshot: the selected session reloads its
        // newest page, and `HistoryLoaded.runs` supplies each run's outcome
        // (`restore_finished_run`). `Session.last_finished_run` only covers
        // the latest one (sidebar / R-1).
        let active: BTreeSet<RunId> = active_runs.iter().map(|a| a.run.id.clone()).collect();
        for vm in self.sessions.values_mut() {
            let Some(id) = vm.active_run.clone() else {
                continue;
            };
            if active.contains(&id) {
                continue;
            }
            // Ended during the gap without a `RunFinished` we saw.
            if let Some(run) = vm.runs.get_mut(&id)
                && !run.phase.is_terminal()
            {
                run.phase = RunPhase::EndedUnseen;
            }
            if let Some(message) = vm.assistant_mut(&id) {
                message.streaming = false;
            }
            vm.active_run = None;
        }
        for active_run in active_runs {
            self.restore_active_run(active_run);
        }
        // R-1: `Session.last_finished_run` gives the sidebar its last outcome
        // after a restart and fills in runs that ended during a gap.
        for session in adopt {
            self.adopt_session(session);
        }

        // Selection and history.
        if first
            && self.selected_session.is_none()
            && let Some(id) = self.navigation_order().first().cloned()
        {
            self.select_session(&id);
        }
        {
            // Resync / first snapshot: messages may have been persisted
            // during a gap. The selected session reloads its newest page now
            // (merged, no duplicates) so `HistoryLoaded.runs` can restore
            // every finished run's outcome — not just `last_finished_run`.
            // Other sessions reload when next opened. A page already in
            // flight is left alone.
            let selected = self.selected_session.clone();
            for (id, vm) in self.sessions.iter_mut() {
                if Some(id) != selected.as_ref()
                    && !matches!(vm.history, HistoryState::NotLoaded | HistoryState::Loading)
                {
                    vm.history = HistoryState::NotLoaded;
                }
            }
            if let Some(id) = selected {
                let reload = self
                    .sessions
                    .get(&id)
                    .is_some_and(|vm| vm.history != HistoryState::Loading);
                if reload {
                    self.reload_newest(&id);
                }
            }
        }
    }

    fn reload_newest(&mut self, id: &SessionId) {
        use crate::app::{HISTORY_PAGE, Pending};
        self.push(
            Pending::LoadHistory {
                session: id.clone(),
                before: None,
            },
            Command::LoadHistory {
                session: id.clone(),
                before: None,
                limit: HISTORY_PAGE,
            },
        );
    }

    fn restore_active_run(&mut self, active: ActiveRun) {
        let ActiveRun {
            run,
            input_id,
            text,
            reasoning,
        } = active;
        let session = run.session_id.clone();
        self.run_sessions.insert(run.id.clone(), session.clone());
        let Some(vm) = self.sessions.get_mut(&session) else {
            return;
        };
        let entry = vm
            .runs
            .entry(run.id.clone())
            .or_insert_with(|| RunVm::from_run(&run));
        if !entry.phase.is_terminal() {
            entry.usage = run.usage.clone();
            entry.phase = match run.state {
                RunState::WaitingApproval => RunPhase::WaitingApproval,
                RunState::Cancelling => RunPhase::Cancelling,
                _ if !text.is_empty() => RunPhase::Streaming,
                _ if !reasoning.is_empty() => RunPhase::Thinking,
                _ => match &entry.phase {
                    RunPhase::Retrying { .. } => entry.phase.clone(),
                    _ => RunPhase::Starting,
                },
            };
        }
        vm.accepted_inputs.retain(|(id, _, _)| id != &input_id);
        vm.active_run = Some(run.id.clone());
        if !text.is_empty() || !reasoning.is_empty() {
            // The snapshot carries everything published so far: replace.
            let message = vm.ensure_assistant(&run.id);
            message.text = text;
            message.reasoning = reasoning;
        }
    }

    /// `SessionCreated` / `SessionUpdated` / snapshot rows: store the session
    /// and adopt `last_finished_run` (R-1).
    ///
    /// In-memory knowledge wins. A run we saw finish is left alone; a run
    /// that ended unseen (seq gap) gets its outcome, card and divider now.
    /// Otherwise the finished run only sets the sidebar status (last
    /// outcome / error), unless a newer finished run is already known. Its
    /// card is placed when the history page carrying the run arrives
    /// (`HistoryLoaded.runs`), since only that has the run's messages and
    /// start time.
    pub(crate) fn adopt_session(&mut self, session: Session) {
        let id = session.id.clone();
        let finished = session.last_finished_run.clone();
        match self.sessions.get_mut(&id) {
            Some(vm) => vm.session = session,
            None => {
                self.sessions.insert(id.clone(), SessionVm::new(session));
            }
        }
        let Some(finished) = finished else {
            return;
        };
        let Some(vm) = self.sessions.get_mut(&id) else {
            return;
        };
        self.run_sessions.insert(finished.run.clone(), id.clone());
        match vm.runs.get_mut(&finished.run).map(|r| r.phase.clone()) {
            Some(RunPhase::Finished(_)) => return,
            Some(RunPhase::EndedUnseen) => {
                if let Some(r) = vm.runs.get_mut(&finished.run) {
                    r.phase = RunPhase::Finished(finished.outcome.clone());
                    if let Some(started) = crate::outcome::known_time(finished.started_at) {
                        r.started_at_ms = started;
                    }
                    r.finished_at_ms = crate::outcome::known_time(finished.finished_at);
                }
                place_outcome(
                    vm,
                    &finished.run,
                    &finished.outcome,
                    crate::outcome::known_time(finished.finished_at),
                    None,
                );
            }
            // Still running in memory: `RunFinished` follows (Core sends
            // SessionUpdated first); only the sidebar status moves early.
            _ => {}
        }
        note_latest(
            vm,
            &finished.outcome,
            crate::outcome::known_time(finished.finished_at),
        );
    }

    /// Rebuilds the error card + divider of a finished run reported by Core
    /// after the fact (`HistoryLoaded.runs`, R-1), placed after the run's
    /// last message in the page (`anchor` ordinal) or, without one, after its
    /// assistant message / at the end. Idempotent: a run already finished in
    /// memory is left alone; no announcement (it is history).
    pub fn restore_finished_run(&mut self, run: Run) {
        self.restore_run_at(run, None);
    }

    fn restore_run_at(&mut self, run: Run, anchor: Option<u64>) {
        let Some(outcome) = run.outcome.clone() else {
            return;
        };
        let Some(vm) = self.sessions.get_mut(&run.session_id) else {
            return;
        };
        self.run_sessions
            .insert(run.id.clone(), run.session_id.clone());
        if vm
            .runs
            .get(&run.id)
            .is_some_and(|r| matches!(r.phase, RunPhase::Finished(_)))
        {
            return;
        }
        if vm.active_run.as_ref() == Some(&run.id) {
            vm.active_run = None;
        }
        // 0 / missing timestamps stay unknown; never fall back to started_at
        // (which may itself be 0) or to the UI clock.
        let finished_at = crate::outcome::known_optional_time(run.finished_at);
        vm.runs.insert(run.id.clone(), RunVm::from_run(&run));
        if let Some(message) = vm.assistant_mut(&run.id) {
            message.streaming = false;
            message.stopped = matches!(outcome, RunOutcome::Cancelled { .. });
        }
        place_outcome(vm, &run.id, &outcome, finished_at, anchor);
        // `Session.last_finished_run` (R-1) is authoritative for the sidebar:
        // an older run restored while paging back never takes it over.
        let is_latest = vm
            .session
            .last_finished_run
            .as_ref()
            .is_none_or(|f| f.run == run.id);
        if is_latest {
            note_latest(vm, &outcome, finished_at);
        }
    }

    /// `HistoryLoaded`: merges a page into the timeline (oldest → newest,
    /// deduplicated by `input_id` / `run_id`) and advances the cursor.
    pub(crate) fn on_history_loaded(
        &mut self,
        session: SessionId,
        messages: Vec<Message>,
        next_before: Option<u64>,
        runs: Vec<Run>,
    ) {
        // Each run's last message in this page anchors its card / divider.
        let mut anchors: BTreeMap<RunId, u64> = BTreeMap::new();
        for m in &messages {
            let e = anchors.entry(m.run_id.clone()).or_insert(m.ordinal);
            *e = (*e).max(m.ordinal);
        }
        let Some(vm) = self.sessions.get_mut(&session) else {
            return;
        };
        // A session Core still marks Running with no active run in the
        // snapshot (crash before P3 R9 recovery): adopt the newest page's run
        // so the composer shows "运行中" and ⌘. / 停止 can send CancelRun.
        if vm.session.state == SessionState::Running
            && vm.active_run.is_none()
            && let Some(newest) = messages.iter().max_by_key(|m| m.ordinal)
            && !vm
                .runs
                .get(&newest.run_id)
                .is_some_and(|r| r.phase.is_terminal())
        {
            let run_id = newest.run_id.clone();
            vm.runs.entry(run_id.clone()).or_insert_with(|| RunVm {
                id: run_id.clone(),
                session_id: session.clone(),
                queue_item_id: None,
                phase: RunPhase::Starting,
                usage: Usage::default(),
                started_at_ms: vm.session.updated_at,
                finished_at_ms: None,
                cancel_requested: false,
            });
            vm.active_run = Some(run_id.clone());
            self.run_sessions.insert(run_id, session.clone());
        }
        let Some(vm) = self.sessions.get_mut(&session) else {
            return;
        };
        let older_page = vm.history == HistoryState::LoadingOlder;
        let previous_oldest = vm.oldest_ordinal;
        let page_min = messages.iter().map(|m| m.ordinal).min();
        merge_messages(vm, messages);
        let keep_cursor = !older_page
            && previous_oldest.is_some_and(|old| page_min.is_none_or(|min| old <= min))
            && matches!(vm.history, HistoryState::HasMore | HistoryState::Complete);
        if !keep_cursor {
            vm.history_before = next_before;
            vm.history = match next_before {
                Some(_) => HistoryState::HasMore,
                None => HistoryState::Complete,
            };
        }
        // R-1: outcomes of the page's runs (older failures too when paging).
        for run in runs {
            if run.session_id == session && run.outcome.is_some() {
                let anchor = anchors.get(&run.id).copied();
                self.restore_run_at(run, anchor);
            }
        }
    }
}

fn item_ordinal(item: &TimelineItem) -> Option<u64> {
    match item {
        TimelineItem::User(m) => m.ordinal,
        TimelineItem::Assistant(m) => m.ordinal,
        _ => None,
    }
}

fn is_message(item: &TimelineItem) -> bool {
    matches!(item, TimelineItem::User(_) | TimelineItem::Assistant(_))
}

/// Inserts persisted messages in ordinal order. A message already shown
/// (same `input_id` for user rows, same `run_id` for assistant rows) only gets
/// its ordinal and persisted text. New rows go before the first shown message
/// with a larger ordinal, or after the last message with an ordinal (and the
/// divider / error card that follow it) when there is none.
pub(crate) fn merge_messages(vm: &mut SessionVm, mut messages: Vec<Message>) {
    messages.sort_by_key(|m| m.ordinal);
    for message in messages {
        let ordinal = message.ordinal;
        vm.oldest_ordinal = Some(vm.oldest_ordinal.map_or(ordinal, |o| o.min(ordinal)));
        let item = match message.role {
            MessageRole::User => {
                let input_id = message
                    .input_id
                    .clone()
                    .unwrap_or_else(|| InputId(message.id.0.clone()));
                let existing = vm.timeline.iter_mut().find_map(|item| match item {
                    TimelineItem::User(m) if m.input_id == input_id => Some(m),
                    _ => None,
                });
                if let Some(existing) = existing {
                    existing.ordinal = Some(ordinal);
                    continue;
                }
                TimelineItem::User(UserMessageVm {
                    input_id,
                    ordinal: Some(ordinal),
                    text: message.content.text,
                    origin: MessageOrigin::Direct,
                })
            }
            MessageRole::Assistant => {
                let run_id = message.run_id.clone();
                let existing = vm.timeline.iter_mut().find_map(|item| match item {
                    TimelineItem::Assistant(m) if m.run_id == run_id => Some(m),
                    _ => None,
                });
                if let Some(existing) = existing {
                    existing.ordinal = Some(ordinal);
                    if !existing.streaming {
                        existing.text = message.content.text;
                    }
                    continue;
                }
                TimelineItem::Assistant(AssistantMessageVm {
                    run_id,
                    ordinal: Some(ordinal),
                    text: message.content.text,
                    reasoning: String::new(),
                    streaming: false,
                    stopped: false,
                    reasoning_started_ms: None,
                    reasoning_ms: None,
                })
            }
            // Tool rows are P2 (§6.5).
            MessageRole::Tool => continue,
        };
        let position = vm
            .timeline
            .iter()
            .position(|i| item_ordinal(i).is_some_and(|o| o > ordinal))
            .unwrap_or_else(|| {
                match vm.timeline.iter().rposition(|i| item_ordinal(i).is_some()) {
                    // Skip the divider / card / notice rows that belong to it.
                    Some(last) => {
                        last + 1
                            + vm.timeline[last + 1..]
                                .iter()
                                .take_while(|i| !is_message(i))
                                .count()
                    }
                    // Nothing persisted shown yet: history goes before live rows.
                    None => 0,
                }
            });
        vm.timeline.insert(position, item);
    }
}

/// Error card (failures) + divider for `run`, unless its divider exists.
fn place_outcome(
    vm: &mut SessionVm,
    run: &RunId,
    outcome: &RunOutcome,
    finished_at: Option<u64>,
    anchor: Option<u64>,
) {
    if vm
        .timeline
        .iter()
        .any(|i| matches!(i, TimelineItem::RunDivider { run_id, .. } if run_id == run))
    {
        return;
    }
    let mut items = Vec::new();
    if let Some(failure) = crate::outcome::failure_of(outcome) {
        items.push(TimelineItem::Error(ErrorVm {
            run_id: run.clone(),
            kind: failure.category,
            retry_at_ms: failure
                .retry_after_ms
                .filter(|ms| *ms > crate::outcome::LONG_RETRY_AFTER_MS)
                .and_then(|ms| finished_at.map(|at| at + ms)),
            failure,
        }));
    }
    items.push(TimelineItem::RunDivider {
        run_id: run.clone(),
        outcome: outcome.clone(),
    });
    let after_anchor = anchor.and_then(|ordinal| {
        vm.timeline
            .iter()
            .rposition(|i| item_ordinal(i) == Some(ordinal))
    });
    let after_assistant = || {
        vm.timeline
            .iter()
            .rposition(|i| matches!(i, TimelineItem::Assistant(m) if &m.run_id == run))
    };
    let at = after_anchor
        .or_else(after_assistant)
        .map_or(vm.timeline.len(), |i| i + 1);
    vm.timeline.splice(at..at, items);
}

/// Sidebar status follows the newest finished run we know of.
fn note_latest(vm: &mut SessionVm, outcome: &RunOutcome, finished_at: Option<u64>) {
    // When this finish time is unknown, the caller (last_finished_run or the
    // latest history run) is authoritative: do not let some other run's known
    // clock skip the update.
    let newer = finished_at.is_some_and(|finished_at| {
        vm.runs.values().any(|r| {
            matches!(r.phase, RunPhase::Finished(_))
                && r.finished_at_ms.is_some_and(|t| t > finished_at)
        })
    });
    if newer {
        return;
    }
    let failure = crate::outcome::failure_of(outcome);
    vm.last_error = failure.as_ref().map(|f| f.category);
    vm.last_error_source = failure.and_then(|f| f.source);
    vm.last_outcome = Some(outcome.clone());
}
