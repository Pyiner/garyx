# Thread Running-State Projection — Event-Driven, Backend Source of Truth

Status: approved for implementation (user-confirmed direction "按 v2 干").

## Goal

The thread list (pinned + recent) shows a per-row "running" badge (the
three-dot typing indicator). Today each client decides it differently:

- iOS `homeThreadRunningThreadIds` mixes three sources (local run tracker,
  per-thread committed run-state, server `run_state` fallback).
- Desktop ignores the server field entirely and infers busy purely from its
  local `messageState` stream machine, so runs started elsewhere never light up.
- Widget reads the server snapshot only.

We converge on a single rule: **the backend owns one `run_state` field per
thread; all clients dumb-read it.** No client recomputes "is it running".

## Source Of Truth

The authoritative per-thread value is `recent_threads.run_state` (and its
mirror `active_run_id`), which already exists in the SQLite projection
(`garyx_db.rs`). What changes is *how it is decided*:

> A thread is `running` iff its transcript's last run-lifecycle control is an
> open (`run_start` with no paired close) **AND** the bridge's in-memory run
> index still holds that run as active.

The two-condition AND is deliberate:

- **transcript event** drives the steady state immediately. `run_complete` /
  `run_interrupted` / `interrupt_confirmed` flips the reduced state to
  not-busy the moment it is committed, independent of any in-memory cleanup
  ordering.
- **in-memory confirmation** vetoes orphans. After a crash / SIGKILL the
  transcript may keep a dangling `run_start` (the close was never written), but
  the bridge run index is rebuilt empty on restart, so the orphan resolves to
  `idle`.

This is still event-driven (transcript run-lifecycle controls are the events);
the in-memory index only removes the one failure mode events cannot cover
(process death with no close event).

### Why not "pure in-memory truth"

`active_runs.remove` happens in the run task's cleanup block *after*
`run_complete` is committed (`run_management.rs:2092-2095`). A projection that
read in-memory state on the `run_complete`-triggered re-projection would still
see the run present and fail to clear the badge. Gating on
`transcript_busy AND memory_active` avoids touching bridge ordering: once
`run_complete` is committed, `transcript_busy` is already false, so the result
is `idle` regardless of remove timing.

## State Machine (`idle` ⇄ `running`)

We only care about the **thread itself**, not sub-tasks. A thread is serial:
sending a new message first `abort_thread_runs` the in-flight run
(`run_management.rs:1451`), and `max_concurrent_runs` is a global cap, not
per-thread. So a thread has at most one of its own runs at a time → the field
is a boolean (has its own active run / not). Child runs live on their own
threads (already `exclude_from_recent`) and never aggregate onto the parent
row.

- **Open → running:** `run_start` (your message, automation, another device).
- **Close → idle:** `run_complete` (status completed / error / interrupted /
  aborted), `run_interrupted`, `interrupt_confirmed`.

### Completeness

| End scenario | Event | Covered by |
| --- | --- | --- |
| Normal finish | `run_complete` (completed) | transcript event |
| Error / provider failure | `run_complete` (error) | transcript event |
| User stop | `run_interrupted` / `interrupt_confirmed` | transcript event |
| New message preempts in-flight | `run_complete` (interrupted) via `abort_thread_runs` | transcript event |
| Program/system abort | `run_complete` (aborted) | transcript event |
| Same thread concurrency | n/a (serial) | only ever 0 or 1 |
| **Process crash / SIGKILL / power loss** | **no event** | in-memory confirm + startup reconcile + SIGTERM abort-all |

Events cover every process-alive end path. The single blind spot (process death
with no close event) is closed by the in-memory AND + the startup reconcile +
graceful-shutdown abort.

## Backend Changes (garyx-gateway, garyx-bridge)

1. **bridge**: add `MultiProviderBridge::abort_all_active_runs()` — iterate
   `get_active_runs()` and `abort_run` each. `is_run_active(run_id)` already
   exists (`run_management.rs:2825`) and is reused as the in-memory probe.
2. **gateway**: introduce an `ActiveRunProbe` trait and a
   `BridgeActiveRunProbe(Weak<MultiProviderBridge>)` adapter. **Weak** is
   required: the bridge holds `Arc<thread_store>` (the projecting store) via
   `set_thread_store_blocking`, so the projecting store must not hold an
   `Arc<MultiProviderBridge>` back (would leak/cycle).
3. **gateway**: `RecentThreadProjectingStore` gains the probe. In
   `project_thread`, after computing the transcript `active_run_id`, drop it to
   `None` when `!probe.is_run_active(id)`. Same gate in
   `reconcile_active_recent_thread_projection` (startup orphan clear — at boot
   the index is empty, so every stale `running` row clears to `idle`).
4. **gateway**: `server.rs` `serve` — after graceful shutdown returns, call
   `bridge.abort_all_active_runs()` with a bounded timeout so a clean restart
   writes close events and leaves no orphan; the startup reconcile only backs
   up true hard crashes.

The steady-state write path (`project_thread` on every `thread_store.set`) is
unchanged in shape; only the `active_run_id` value gains the memory veto.
Contract preserved: recent_threads stays write-time maintained, read routes do
not repair (`docs/agents/repository-contracts.md`).

## Client Changes (dumb-read the one field)

- **Desktop**: `DesktopThreadSummary` gains `runState`; `gary-client.ts`
  `mapThreadSummary` parses `run_state`; `AppShell.tsx` `recentThreadRows` /
  `pinnedThreadRows` `isBusy` reads `thread.runState === "running"` instead of
  `isRuntimeBusy(selectThreadRuntime(messageState, …))`. Keep the local stream
  machine for the open thread's transcript; only the list badge switches source.
  A light list refresh reuses existing poll/stream hooks.
- **iOS**: `homeThreadRunningThreadIds` collapses to
  `threads.filter { $0.run_state == "running" }`. Remove the runTracker /
  runStateByThread contributions to the *list badge* (they remain for the open
  thread's composer/steer state). Drop the
  `GaryxThreadSummaryRunStateResolver` local override of `thread.runState`.
  Keep the background refresh loop alive on home-visible+ready (decouple it from
  the local-candidate gate).
- **Widget**: already dumb-reads `run_state`/`active_run_id`; converge to
  `run_state == "running"` only; verify against the host snapshot.

### Optional immediacy (not required)

Pure polling means a locally-sent message lights the badge on the next poll
(≤1.5s on iOS). Optional: the locally-interacting thread may optimistically
light its badge as a display-only override that yields to the backend field —
never delays the off. Ship without it first.

## Migration / Validation

- Backend lands first (steady state unchanged; only orphan handling + the AND
  gate change behavior). Each client is independent and ships separately because
  the field already exists in the payload.
- Tests: `cargo test -p garyx-gateway --all-targets`,
  `cargo test -p garyx-bridge --all-targets`. Add: project clears `running` when
  probe reports inactive; reconcile clears all `running` on empty index;
  `abort_all_active_runs` writes close controls.
- Orphan e2e: start a run, `kill -9` gateway, restart, assert
  `/api/recent-threads` `run_state != "running"`.
- Cross-end: one running thread, assert badge parity across desktop + iOS +
  widget + `/api/recent-threads`.
- Review: Claude implements → Codex adversarial review (`garyx task create
  --agent codex --workspace-dir <worktree>`).
