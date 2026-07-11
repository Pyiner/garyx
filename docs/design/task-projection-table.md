# Task Projection Table Design

> **Superseded (2026-07, #TASK-2099).** Historical design record. The
> shipped architecture differs: `thread_records` in SQLite is the truth
> source (#TASK-1864), `task_projection` derives in the same transaction
> as every record write (no rebuild/reconcile layer), the file-based
> task counter was replaced by the transactional SQLite `task_counter`
> row, and the in-memory TASK_INDEX plus registry-based readers were
> removed in favor of the store-owned `TaskProjectionReader` seam. See
> `docs/agents/repository-contracts.md` for the current contracts.

Status: approved synthesis for implementation. The thread file record remains
the only source of truth for task data; `task_projection` is a read-only derived
SQLite projection.

## Goal

Garyx tasks are stored as a `task` overlay on normal thread records. Today,
task listing, number lookup, source/status/assignee filtering, and running
subtask checks depend on the process-local `TASK_INDEX`. After a gateway
restart, that index is rebuilt by scanning all thread files.

Add an independent `task_projection` table that stores queryable task business
fields and allows:

- SQL-backed list, status, assignee, creator, and source filtering.
- SQL-backed recursive parent/child traversal.
- Cold-start `TASK_INDEX` bootstrap from SQLite instead of a full file scan.
- Running-subtask checks that work immediately after restart.

The projection must never become canonical task storage. If projection writes
fail, the file write still succeeds and backfill/reconcile repairs the derived
table later.

## Decisions

Use an independent `task_projection` table rather than adding sparse task-only
columns to `thread_meta`.

Use the existing `RecentThreadProjectingStore` write path. TaskService CRUD and
bridge task-status transitions already write through the same gateway
`ThreadStore`; projection writes belong in `project_thread()` and `delete()`,
not in a new wrapper store or router-to-gateway dependency.

Use versioned backfill plus realtime single-row upsert. Backfill is the only
production path allowed to scan all thread files, and it runs from gateway
warmup and reader `is_current` gating.

Keep `TASK_INDEX` for now as a process-local cache and number-allocation guard,
but bootstrap it from the projection when current. Non-gateway tests and stores
without a reader continue to use the existing file-scan fallback.

Use router-owned reader traits and a gateway implementation. `garyx-router`
must not depend on gateway SQLite code.

## Schema

The table keeps flattened filter columns plus canonical JSON strings for
structured task values that must be reconstructed without loss.

```sql
CREATE TABLE IF NOT EXISTS task_projection (
    thread_id              TEXT PRIMARY KEY,
    number                 INTEGER NOT NULL CHECK (number > 0),
    status                 TEXT NOT NULL CHECK (
        status IN ('todo', 'in_progress', 'in_review', 'done')
    ),
    title                  TEXT NOT NULL,

    creator_json           TEXT NOT NULL,
    creator_id             TEXT NOT NULL,
    assignee_json          TEXT,
    assignee_id            TEXT,
    updated_by_json        TEXT NOT NULL,
    executor_json          TEXT,

    source_json            TEXT,
    source_thread_id       TEXT,
    source_task_thread_id  TEXT,
    source_task_id         TEXT COLLATE NOCASE,
    parent_task_number     INTEGER CHECK (
        parent_task_number IS NULL OR parent_task_number > 0
    ),
    source_bot_id          TEXT,

    notification_thread_id TEXT,

    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL,
    source_updated_at      TEXT NOT NULL,
    source_events_len      INTEGER NOT NULL CHECK (source_events_len >= 0),

    projection_version     INTEGER NOT NULL DEFAULT 1,
    projected_at           TEXT NOT NULL
) STRICT;
```

Indexes:

```sql
CREATE INDEX IF NOT EXISTS idx_task_projection_number
    ON task_projection(number);

CREATE INDEX IF NOT EXISTS idx_task_projection_updated
    ON task_projection(updated_at DESC, thread_id ASC);

CREATE INDEX IF NOT EXISTS idx_task_projection_open_updated
    ON task_projection(updated_at DESC, thread_id ASC)
    WHERE status <> 'done';

CREATE INDEX IF NOT EXISTS idx_task_projection_status_updated
    ON task_projection(status, updated_at DESC, thread_id ASC);

CREATE INDEX IF NOT EXISTS idx_task_projection_assignee_status_updated
    ON task_projection(assignee_id, status, updated_at DESC, thread_id ASC);

CREATE INDEX IF NOT EXISTS idx_task_projection_creator_status_updated
    ON task_projection(creator_id, status, updated_at DESC, thread_id ASC);

CREATE INDEX IF NOT EXISTS idx_task_projection_source_thread_updated
    ON task_projection(source_thread_id, updated_at DESC, thread_id ASC);

CREATE INDEX IF NOT EXISTS idx_task_projection_source_task_thread_updated
    ON task_projection(source_task_thread_id, updated_at DESC, thread_id ASC);

CREATE INDEX IF NOT EXISTS idx_task_projection_source_task_updated
    ON task_projection(source_task_id, updated_at DESC, thread_id ASC);

CREATE INDEX IF NOT EXISTS idx_task_projection_source_bot_updated
    ON task_projection(source_bot_id, updated_at DESC, thread_id ASC);

CREATE INDEX IF NOT EXISTS idx_task_projection_notification_thread_status
    ON task_projection(notification_thread_id, status, updated_at DESC)
    WHERE status = 'in_progress';

CREATE INDEX IF NOT EXISTS idx_task_projection_parent_thread_updated
    ON task_projection(source_task_thread_id, updated_at DESC, thread_id ASC);

CREATE INDEX IF NOT EXISTS idx_task_projection_parent_number_updated
    ON task_projection(parent_task_number, updated_at DESC, thread_id ASC);
```

`number` is intentionally not unique. The projection is derived data and must
not let historical drift or concurrent repair prevent `ON CONFLICT(thread_id)`
upserts. Readers dedupe by `number` and warn on duplicates.

## Draft Construction

Create `garyx-gateway/src/task_projection.rs` with:

```rust
task_projection_draft_from_thread_data(thread_id, data) -> Option<TaskProjectionDraft>
backfill_task_projection_if_incomplete(thread_store, garyx_db) -> usize
```

Draft construction must parse the record through the typed `ThreadTask` path and
then serialize structured fields with `serde_json::to_string`. Do not store raw
`serde_json::Value` subtrees, because legacy aliases and map ordering would
break canonical equality.

Field mapping:

- `thread_id`: canonical thread key.
- `number`, `status`, `title`, `created_at`, `updated_at`: from `ThreadTask`.
- `creator_json`, `assignee_json`, `updated_by_json`, `executor_json`,
  `source_json`: canonical typed JSON.
- `creator_id`, `assignee_id`: principal id helper columns.
- `source_thread_id`, `source_task_thread_id`, `source_task_id`: from
  `TaskSource`.
- `parent_task_number`: parse `source.task_id` with the existing task-id parser.
- `source_bot_id`: `source.bot_id`, or the existing `channel:account` fallback.
- `notification_thread_id`: populated only for thread notification targets.
- `source_updated_at`: task `updated_at`.
- `source_events_len`: `task.events.len()`.

## Write Semantics

`GaryxDbService` owns:

- `CURRENT_TASK_PROJECTION_VERSION = 1`.
- `TASK_PROJECTION_NAME = "task_projection"`.
- `TaskProjectionDraft`.
- `replace_task_projection`.
- `remove_task_projection`.
- `sync_task_projection_snapshot`.
- `task_projection_needs_backfill`.
- `count_task_projection`.
- SQL read methods for index rows, number lookup, list, running-subtask checks,
  subtree, and ancestor traversal.

All upserts, from realtime writes, backfill snapshots, and reconcile, must use
the same revision guard:

```sql
ON CONFLICT(thread_id) DO UPDATE SET ...
WHERE (excluded.source_events_len, excluded.source_updated_at)
    > (task_projection.source_events_len, task_projection.source_updated_at)
```

`source_events_len` is the primary monotonic revision. `updated_at` is only a
tie-break. All mutations that can change a projected field must append a task
event, including create, assign, unassign, update status, stop, set title,
mark-in-review, and mark-in-progress-on-wake. Deletion/removal uses tombstone
compensation and does not rely on events length.

Projection writes run after the file write and must warn, not fail the business
operation, if SQLite projection fails.

`RecentThreadProjectingStore` must:

- Upsert task projection when `project_thread()` sees a task.
- Remove task projection when `project_thread()` sees no task.
- Remove task projection from `delete()`.
- Remove task projection from archived/deleted projection cleanup helpers.

## Backfill And Reconcile

Backfill state uses `projection_states`:

- Missing state means backfill is required.
- Version mismatch means backfill is required.
- Previous `source_row_count > 0` and current table count `0` means backfill is
  required.
- A zero-task install records `(version=1, source_row_count=0)` and must not
  rescan on every startup.

`source_row_count` records the number of task rows written by the task
projection backfill, not the number of thread files.

Backfill must be single-flight. The background warmup path and the first request
that observes `reader.is_current() == false` share the same lock, recheck
`needs_backfill`, and ensure only one scan runs.

Snapshot writes must not clear the table. They upsert each scanned draft with
the revision guard and clean only rows whose `projection_version` is not current.
This prevents a snapshot from deleting current realtime writes.

Realtime deletes during backfill must record tombstones in process memory.
After the snapshot writes, backfill applies tombstone compensation deletes so
that a scanned stale row is not resurrected after the file truth removed it.

Task projection reconcile is mandatory:

- It rides the existing `reconcile_active_recent_thread_projection` periodic
  loop and does not introduce a separate cadence.
- It removes stale rows whose file truth no longer contains a task.
- It fills missing rows and updates rows whose source revision is behind.
- It runs immediately once after successful backfill.

## Router Reader

Define in `garyx-router`:

```rust
#[async_trait]
pub trait TaskProjectionReader: Send + Sync {
    async fn is_current(&self) -> bool;
    async fn task_index_rows(&self) -> Vec<(u64, String)>;
    async fn thread_id_for_number(&self, number: u64) -> Option<String>;
    async fn has_running_subtask_targeting(&self, thread_id: &str) -> bool;
    async fn list_task_summaries(
        &self,
        filter: &TaskListFilter,
    ) -> Option<(Vec<TaskSummary>, usize, bool)>;
    async fn max_number(&self) -> Option<u64>;
}
```

Reader registration is store-scoped, not process-global:

```rust
static TASK_PROJECTION_READERS:
    OnceLock<StdMutex<HashMap<usize, Arc<dyn TaskProjectionReader>>>>;
```

The key is the same `store_id` used by `TASK_INDEX` (`Arc::as_ptr` partitioning).
This keeps multiple AppState instances, memory databases, and tests isolated,
and lets free functions find the reader for their current `ThreadStore`.

`TaskService` keeps a `with_projection_reader` builder for explicit injection,
but free functions and service methods also look up the store-scoped registry.
If no reader exists, behavior falls back to the existing file-backed path.

`ensure_task_index` rules:

- Reader exists and `is_current()` is true: load `task_index_rows()` into the
  memory index and mark bootstrapped.
- Reader exists and is not current: synchronously trigger single-flight
  backfill, then load from projection only if it succeeds and is current.
- Backfill failure must not mark the memory index bootstrapped.
- No reader: use the existing full-file scan fallback.

`find_task_by_number` may use `thread_id_for_number`, but it must still read the
file truth and verify the task number. If the file is stale or mismatched, remove
the projection row and fall back according to projection state.

`thread_task_has_running_subtasks` must use SQL when a reader exists:

```sql
status = 'in_progress'
AND notification_thread_id = ?
AND thread_id <> ?
LIMIT 1
```

This is required for bridge parent-task gating after restart.

## SQL List Semantics

`list_task_summaries` pushes down all existing filters:

- `status`.
- `include_done=false` as `status <> 'done'`.
- `assignee` by canonical `assignee_json`.
- `creator` by canonical `creator_json`.
- `source_thread_id` matching `source_thread_id OR source_task_thread_id`.
- `source_task_id` with `COLLATE NOCASE`.
- `source_bot_id`, including the existing `channel:account` fallback.

List must dedupe duplicate task numbers:

```sql
ROW_NUMBER() OVER (
    PARTITION BY number
    ORDER BY updated_at DESC, thread_id ASC
) = 1
```

`total` and `has_more` are computed after dedupe. Duplicate numbers should log a
warning. List is intentionally zero-file-read and therefore eventually
consistent: stale rows may be briefly visible until delete projection,
single-read cleanup, or periodic reconcile removes them.

`TaskSummary.runtime_agent_id` and `reply_count` come from a `LEFT JOIN` to
`thread_meta(agent_id, message_count)` and are not duplicated into
`task_projection`.

## Parent And Ancestor Traversal

Recursive CTEs use `thread_id` as the recursive identity. `source_task_thread_id`
is the preferred parent pointer; `source_task_id` and `parent_task_number` are
fallback filters only. CTEs must include depth or visited-path guards, with a
maximum depth of 64, so malformed self-references cannot loop forever.

Subtree traversal starts at the root task thread id, then follows children whose
`source_task_thread_id` is the current node thread id, or whose
`source_task_id` matches the current node task id case-insensitively.

Ancestor traversal starts at the leaf task thread id, then follows
`source_task_thread_id` or `source_task_id` back to the parent projection row.

## Gateway Integration

Gateway registers a store-scoped reader for each AppState/ThreadStore using the
same store identity as `TASK_INDEX`.

Reader coverage must include:

- `TaskService` used by task routes.
- Automation task service creation.
- Bridge `run_management` free functions.

`app_state.rs` must run task projection backfill in sync warmup and run one
task projection reconcile pass immediately after successful backfill. The
periodic active recent-thread reconcile loop also runs task projection
reconcile.

## Validation Requirements

Required tests:

- Gateway memory DB DDL, upsert, remove, count, and version backfill predicate.
- Conditional upsert read-then-write interleaving where a stale snapshot cannot
  overwrite a newer realtime write.
- Tombstone compensation where a backfill snapshot cannot resurrect a deleted
  task row.
- SQL list filter pushdown, duplicate-number dedupe, total/has_more after
  dedupe, and stale-row cleanup.
- Recursive subtree and ancestor CTEs with thread-id identity and cycle defense.
- Router tests with no reader still pass through the existing file-backed path.
- First request before warmup triggers single-flight backfill and does not mark
  `TASK_INDEX` bootstrapped on failure.
- Warmup/write race.
- Headless cold-start list uses SQL and does not scan all task files after
  clearing the in-memory index.
- Bridge running-subtask gating works after restart without a warmed
  `TASK_INDEX`.
- R4 invariant: every mutation that changes projected task fields increments
  `events.len()`.

Before handoff, run focused tests for the touched crates plus `cargo build` and
`cargo clippy`.
