# forge-state ← Core P1 protocol: adapter plan (not implemented yet)

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
