# Gateway Control Plane Optimization Plan

Date: 2026-07-07
Tracking: #TASK-1829
Basis: `gateway-control-plane-performance-investigation.md` (2026-07-07) plus an
independent verification pass done the same day.

## Goal

Keep control-plane latency (health, tasks, threads, desktop/mobile API) in the
low-millisecond range even while large provider runs or restart-wake resumes
are active, and stop the CLI from amplifying transient backend delays into
"gateway not reachable" hard failures.

Non-goals for this plan:

- Reducing the token/memory cost of a single large resume (1.68M input tokens,
  5.5GB peak footprint). That is a provider/resume-architecture topic and is
  tracked separately (see Batch 6 notes).
- Splitting the gateway into separate processes. Phase-3-style separation is
  deferred until the in-process fixes below are measured.

## Verified Evidence Base

Verification confirmed the investigation's phenomena and corrected two of its
code-level claims:

- Confirmed, runtime side: repeated restart-wake resumes costing ~1.68M input
  tokens / ~94s (`15:38:48` dispatch → `15:40:22` completion), kernel-recorded
  `Physical footprint (peak): 5.5G`, 41.6M input tokens of run traffic in one
  day, `workflow.ts is required` warning emitted 198,142 times (24.5% of all
  log lines, one scan every 3-4s) until fixed, 230MB un-rotated `stderr.log`.
- Confirmed, code side:
  - Both SQLite services wrap `rusqlite::Connection` in `std::sync::Mutex`
    (`garyx-gateway/src/garyx_db/mod.rs:490`, `garyx-gateway/src/app_db.rs:230`)
    with synchronous methods called directly from async handlers; there is no
    `spawn_blocking` around any SQLite access in the workspace.
  - The tasks list route runs projection backfill on the request thread
    (`garyx-gateway/src/tasks.rs:570-587`). The router task paths do the
    same through `TaskProjectionReader::ensure_current`
    (`garyx-router/src/tasks.rs:391,731,829,959` →
    `garyx-gateway/src/task_projection.rs:56`), so `GET /api/tasks/{id}` —
    the exact lookup `garyx thread send task` issues — can absorb a full
    thread-store rescan when the persisted projection is stale (projection
    version bump, wipe, or missed writes).
  - Neither database tunes SQLite at open: `initialize_connection` sets only
    `foreign_keys ON` (`garyx_db/mod.rs:2998`, `app_db.rs:1027`). Default
    DELETE journal + `synchronous=FULL` means every commit pays
    journal-file create/fsync/delete cost while the connection mutex is
    held, and there is no `busy_timeout`.
  - Thread-history inline images use synchronous `std::fs::read` + base64 on
    the request path (`garyx-gateway/src/api.rs:1386-1394`).
  - Workflow definition listing does a synchronous directory scan + manifest
    parse per call (`garyx-gateway/src/workflows/definitions.rs:92,132`).
  - CLI gateway JSON helper: 5s timeout, single attempt, a fresh
    `reqwest::Client` per call (`garyx/src/commands/gateway_client.rs:174-183`).
- Corrected (investigation overclaimed):
  - `/health` is already strictly lightweight (`routes.rs:607-613`, reads one
    `Instant`). Its observed stalls are worker starvation, not handler weight.
  - Restart-wake dispatch is already bounded: hard cap 16, serial dispatch,
    overload backoff with ≤8 attempts (`restart_wake.rs:27,247,302`). The
    remaining risk is concurrent execution cost of the detached runs, not
    unbounded fan-out.
  - "Locks held across `.await`" does not apply to the std locks: their guards
    are not `Send`, the compiler already prevents it. The real mechanism is
    synchronous blocking of Tokio workers.
- New finding (live repro while filing #TASK-1829, 16:15 local): `/health`
  3.6ms and `GET /api/tasks` 70ms while `POST /api/tasks` timed out twice
  (>5s), concurrent with outbound network stalls (Telegram/minolab/Discord).
  The task-create write path appears to wait on outbound work (notification
  and/or auto-dispatch) before responding.

## Batches

Each batch is independently implementable, verifiable, and committable, in
priority order. Claude-authored batches get codex review tasks
(`--notify current-thread`).

### Batch 1 — CLI resilience (stop the bleeding)

Scope: `garyx/src/commands/gateway_client.rs` (+ call sites if signatures
change).

- Share one `reqwest::Client` (static `OnceLock`) instead of building a new
  client per request; keeps connection pooling.
- Retry policy in the JSON helpers:
  - Idempotent requests (GET): retry on `is_connect()` and `is_timeout()`,
    short exponential backoff (e.g. 250ms, 750ms), total budget ≤ ~8s.
  - Non-idempotent requests (POST/PUT/DELETE): retry only `is_connect()`
    failures (connection refused means the request never reached the gateway,
    so retry is safe — this covers the restart window). Do not auto-retry
    timeouts (the write may have landed); keep the "may be busy" message and
    add a hint to re-check before retrying (e.g. `garyx task list`).
- Keep the two error texts distinguishable (`not reachable` = connect failure,
  `did not respond in time` = timeout); they are diagnostic signals.

Acceptance:

- Unit tests with a mock server: refused connection then healthy → GET and
  POST both succeed via retry; slow-then-healthy → GET succeeds via retry,
  POST fails fast with the busy message.
- Manual: `garyx thread send task` issued during `garyx gateway restart`
  succeeds.

Risk: low. Watch out for the `--json` output contract (errors stay structured).

### Batch 2 — Scheduler-lag observability

Scope: `garyx-gateway` (small; AppState + one background task + `/health`).

- Spawn a watchdog task: every 500ms record the delta between expected and
  actual wake time into an `AtomicU64` on `AppState` (plus a small max-lag
  window).
- Extend `/health` response with `scheduler_lag_ms` and `active_run_count`
  from existing in-memory state. No locks, no DB — `/health` stays a pure
  memory read.
- `WARN` log when lag exceeds a threshold (e.g. 500ms) with the current lag,
  so saturation windows are identifiable in `stderr.log` after the fact.

Acceptance:

- Idle gateway: `scheduler_lag_ms` ~0.
- During a heavy resume (or synthetic CPU load on the runtime), `/health`
  still responds and reports elevated lag; the WARN line appears.

Risk: low. This is the measurement baseline for Batches 3-5.

### Batch 3 — SQLite off the async workers (main fix)

Scope: `garyx-gateway/src/garyx_db/mod.rs`, `garyx-gateway/src/app_db.rs`, and
their async call sites (`routes.rs`, `tasks.rs`, `api.rs`, projection paths).

- Open both connections with `journal_mode=WAL`, `synchronous=NORMAL`, and
  `busy_timeout=5000` (keep `foreign_keys=ON`). WAL removes the per-commit
  journal create/fsync/delete cycle, directly shortening every mutex hold.
  In-memory test databases treat the WAL pragma as a no-op.
- Add async wrappers on the Arc-shared DB services:
  `pub async fn xxx(&self, ...)` = `tokio::task::spawn_blocking` around the
  existing synchronous method (services are already `Arc`, connections already
  `Mutex`-guarded; locking inside the blocking pool is fine — it no longer
  pins a runtime worker).
- Keep the synchronous methods (startup reconciliation, tests, non-async
  contexts) but make the async wrappers the only entry from handlers and
  run-side projection writers.
- Convert call sites in two commits: (a) HTTP read/write handlers
  (`routes.rs:1696,1731`, `tasks.rs:587`, and the rest found by a full grep),
  (b) run-side projection writers (`task_projection.rs`,
  `thread_meta_projection.rs`, `composition/app_state.rs` reconcile paths).
- Error mapping: `JoinError` → 500 with a distinct log line.

Acceptance:

- Behavior conservation: existing gateway tests pass unchanged.
- Before/after benchmark (see Verification Harness): during a large-resume
  window, P99 of `GET /api/tasks` and `GET /api/recent-threads` drops from
  seconds to <100ms; `scheduler_lag_ms` during the same window drops
  correspondingly.

Risk: medium. Many call sites; closures need owned arguments; do not introduce
new lock scopes. Mechanical, reviewable per commit.

### Batch 4 — Request-path hygiene (three small knives)

Scope: `tasks.rs`, `workflows/definitions.rs`, `api.rs`.

- Move `backfill_task_projection_if_incomplete` out of the tasks list route
  (`tasks.rs:570-587`) into startup reconciliation, per the repository
  contract that read routes must not repair projections. First verify the
  `projection_states` version gate semantics so upgrade backfill still runs
  exactly once at startup. The router-side `ensure_current` fallback
  (`find_task_by_number`, `list_tasks`, `ensure_task_index`,
  `thread_task_has_running_subtasks`) stays as a correctness net for
  mid-life projection loss, but after the startup warm-up the common flow —
  including `GET /api/tasks/{id}` right after a version-bump restart — must
  not rescan on the request path.
- Cache workflow definition listing (`definitions.rs`): short-TTL (2-5s) or
  root-mtime-keyed in-memory cache so 3-4s-interval polling stops re-scanning
  the package directory and re-parsing manifests every call. File-backed
  packages remain the source of truth (no DB rows).
- Inline history images: replace `std::fs::metadata`/`std::fs::read`
  (`api.rs:1386-1394`) with `tokio::fs` equivalents.

Acceptance:

- Tasks list returns correct data with both complete and incomplete
  projections; backfill runs at startup (log marker) and never on the read
  route.
- Workflow list reflects a manifest edit within the TTL; no scan logs between
  polls.
- A thread history containing inline images renders identically (byte-equal
  response on a fixture thread).

Risk: backfill relocation medium (semantics), the other two low.

### Batch 5 — POST /api/tasks synchronous-chain audit

Scope: `garyx-gateway/src/tasks.rs` (create handler) and whatever the audit
finds on its synchronous path.

- Reproduced live: task create timed out (>5s, twice) while `/health` and the
  tasks read route were fast, concurrent with outbound-network stalls. Audit
  what create awaits before responding (notification delivery, auto-dispatch,
  DB writes behind the shared conn lock).
- Target semantics: create persists the task and returns; notification and
  run dispatch are enqueued and executed in the background (dispatch state is
  already observable via task records/`Dispatch:` field).
- If the audit shows the delay was purely the Batch-3 conn lock, fold the fix
  into Batch 3 and close this batch with the audit note.

Acceptance:

- With outbound egress artificially slowed/blocked, `POST /api/tasks` returns
  in <500ms and the notification/dispatch happen (or fail with their own log
  lines) in the background.

Risk: medium — touches notification ordering; keep the existing "task created
→ auto-dispatch queued" observable behavior.

### Batch 6 (optional, separate sign-off) — Restart-wake execution gate

Scope: `garyx-gateway/src/restart_wake.rs`.

- Dispatch is serial but the dispatched runs execute detached and concurrently.
  Up to 16 wakes × multi-GB resumes could exceed machine memory. Add a small
  execution-concurrency gate (semaphore of 2-4) for restart-wake-originated
  runs only.
- The larger lever — reducing single-resume cost (1.68M tokens / 5.5GB) — is
  explicitly out of scope here; it belongs with the windowed-resume /
  transcript-cache line of work and needs its own design.

## Verification Harness

A small script (dev-only, e.g. `scripts/dev/control-plane-bench.sh` or inline
in the task) that:

1. Starts a saturation window: trigger a resume of a known-large thread (or a
   synthetic multi-hundred-MB transcript fixture) via the normal wake path.
2. Concurrently probes `/health`, `GET /api/tasks`, `GET /api/recent-threads`
   in a loop (e.g. 20 rps for 60s) and prints a latency distribution
   (p50/p95/p99/max + timeout count).
3. Records `scheduler_lag_ms` (Batch 2) alongside.

Every batch from 3 onward attaches before/after numbers from this harness to
its review task. Batch 2's lag metric defines "saturation window" objectively
so runs are comparable.

## Rollout Order And Process

1. Batch 1 + 2 (independent, can land together) — immediate user-visible
   relief and the measurement baseline.
2. Batch 3 (two commits) — the main fix; measured with the harness.
3. Batch 4 — hygiene knives; measured.
4. Batch 5 — audit-then-fix; measured with egress-stall repro.
5. Batch 6 — only with explicit sign-off, after 1-5 are measured.

Each batch: implement → focused validation → commit (scoped `git commit --
<paths>`) → codex review task (`--agent codex --notify current-thread`) →
address findings → next batch. Gateway behavior changes require build +
install + managed-gateway restart before manual verification
(`scripts/build-local-cli.sh`, restart with wake).
