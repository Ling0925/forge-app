# forge-state ← Core P1 protocol: adapter plan

**Status (built against core/p1 3ea2965b37e5a56f76fdbc221051e8af3cb3a59f): implemented.** The table below is the original plan; see "Implemented since" at the end.

Scope: what changes in `forge-state` once Core lands the agreed P1 protocol.
Every item is local to `forge-state` (+ the `forge-ui` backend glue); the view
layer keeps consuming the same view models.

| Protocol change | forge-state change | Tests to add |
|---|---|---|
| `RunFinished.outcome` = `success \| cancelled{by} \| failed{category, http_status, retry_after_ms, retryable, message}` | Replace `outcome_from_p0` (TODO(core-p1) in `outcome.rs`) with a 1:1 `From<protocol::RunOutcome> for RunOutcomeVm`; delete the reason-string parser and the `cancel_requested/shutting_down` inference. `FailureVm` already has the target shape. | Table test per category → `ErrorCardKind`; `cancelled{by: shutdown}` → no error card. |
| `LoadHistory{session, before, limit}` → `HistoryLoaded{messages, next_before}` | `Pending::LoadHistory` carries `before`; on `HistoryLoaded` prepend reduced items in front of `timeline`, store `next_before` (None ⇒ `HistoryState::Complete`). UI calls `MessageScrollerState::prepend(n)` so the viewport stays anchored. Add `AppState::load_older()` (scroll-to-top trigger, single in-flight). | Prepend keeps order; no duplicate in-flight; `next_before=None` stops paging. |
| `GetSnapshot` → `Snapshot{sessions, workspaces, settings(+key-configured flags), last_seq}` | New `AppState::apply_snapshot`: replace sessions/workspaces wholesale (keep drafts + UI prefs), `has_any_key` from the flags instead of `keychain_reference`, set `last_seq`. Issued at boot and on gap. | Snapshot replaces stale rows, keeps drafts. |
| Monotonic `seq` on every event | `apply` already drops `seq <= last_seq` and flags gaps (`needs_resync`); switch the gap path from "flag" to "push `GetSnapshot` once" and buffer nothing (events after the snapshot's `last_seq` are applied normally). | Gap ⇒ exactly one GetSnapshot; old seq ignored. |
| `Session.created_at/updated_at`, `Run.started_at/finished_at` (UTC ms) | Sidebar ordering by `updated_at` instead of `last_activity` seq; divider elapsed from `finished_at - started_at` (replaces the local clock), "可在 HH:mm 后重试" becomes possible once a wall-clock/tz formatter is added on the UI side. | Ordering; elapsed independent of local clock. |
| `available_tokens` → `context_remaining_estimate` | Rename in `ContextVm` reducer only. | Existing context tests renamed. |
| `Settings.ui_prefs` | `UiPrefs` (`layout.rs`) gets serde; load from snapshot/settings, write back via `UpdateSettings` debounced on drag end (sidebar/inspector width + visibility). | Round-trip; drag emits one update at mouse-up. |
| Idle Send ⇒ `RunStarted{input_id}` without a QueueItem | Match the pending Send by `input_id` (instead of "latest pending Send") to attach the user row and clear the composer; queued sends keep the QueueItem path. | Two quick sends map to the right runs. |

Open questions for Core: the 10 `ErrorKind` values vs the 9 spec cards
(`provider`/`storage` currently fall into the Unknown card); whether
`retry_after_ms` on `failed` is absolute or relative (UI assumes relative to
`finished_at`); `RetryRun` (P3) reject code until then (UI shows a toast).

## Implemented since (core/p1 3ea2965)

- Structured `RunOutcome` → `FailureVm` 1:1 (`outcome::failure_of`); `retry_after_ms` is relative to `finished_at`; ≤ 60000 ms ⇒ retry now (⌘R), > 60000 ⇒ wait card; when the countdown ends the body becomes `error.retry_after_ready` and one polite "现在可以重试" is announced.
- Details: `outcome::remove_request_headers` mirrors `forge-core/src/errors.rs` exactly (same header list, in-segment `[已移除请求头]`), then `sanitize_details` turns literal `\n` into lines and masks residual `sk-…` / Bearer / `api_key=`.
- `GetSnapshot` at boot and once per seq gap; snapshot restores active runs, keeps in-memory error cards, reloads the selected session's newest page (TODO(core): no per-run outcome for runs that ended inside the gap other than `last_finished_run`).
- R-1: `Session.last_finished_run` (Snapshot / SessionCreated / SessionUpdated) drives the sidebar status and finishes a run that ended unseen; `HistoryLoaded.runs[].outcome` rebuilds error card + divider right after that run's last message in the page, also for older pages. Restores are idempotent and never announce. An older run restored while paging never overrides `last_finished_run` in the sidebar.
- R-2: `failed{interrupted}` ("上次异常退出时中断", retryable=true) → Interrupted card with retry enabled.
- `SetProviderKey` / `ProviderKeyConfigured` / `Snapshot.providers[].key_configured` drive the no-key banner; "打开设置" opens the settings panel.
- `Settings.ui_prefs` round-trip (sidebar/inspector width + visibility), written on drag end.
- A session stuck in Running with no active run adopts its newest run so 停止 ⌘. still sends `CancelRun`; `invalid_state` rejections restore the draft.
