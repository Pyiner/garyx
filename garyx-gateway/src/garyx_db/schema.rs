//! Connection setup, base schema creation, and column/index evolution.

use super::*;

/// Durability/concurrency settings for the on-disk database: WAL journal
/// (persistent, readers never block the single writer), NORMAL fsync
/// (sub-ms commits, still crash-safe under WAL), and a busy timeout so
/// cross-process contention retries instead of failing fast.
pub(super) fn configure_file_connection(conn: &Connection) -> GaryxDbResult<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let journal_mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(GaryxDbError::Configuration(format!(
            "failed to enable WAL journal mode: got {journal_mode}"
        )));
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

pub(super) fn initialize_connection(conn: &Connection) -> GaryxDbResult<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS thread_pins (
            thread_id TEXT PRIMARY KEY,
            pinned_at TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0
        ) STRICT;

        CREATE TABLE IF NOT EXISTS thread_pins_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            pins_revision INTEGER NOT NULL DEFAULT 0 CHECK (pins_revision >= 0)
        ) STRICT;

        CREATE TABLE IF NOT EXISTS thread_favorites (
            thread_id TEXT PRIMARY KEY,
            favorited_at TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS thread_favorites_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            favorites_revision INTEGER NOT NULL DEFAULT 0 CHECK (favorites_revision >= 0)
        ) STRICT;

        CREATE TABLE IF NOT EXISTS garyx_store_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            store_incarnation_id TEXT NOT NULL
        ) STRICT;

        -- Thread-record truth source (#TASK-1864 batch 2): canonical record
        -- bodies for thread::*/meta::*/cron::*/tool::* keys. Bodies never
        -- contain the retired `messages` snapshot; projections derive from
        -- this table inside the same write transaction.
        CREATE TABLE IF NOT EXISTS thread_records (
            key         TEXT PRIMARY KEY,
            body        TEXT NOT NULL,
            updated_at  TEXT,
            recorded_at TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS archived_threads (
            thread_id TEXT PRIMARY KEY,
            archived_at TEXT NOT NULL,
            kind TEXT NOT NULL DEFAULT 'archived'
                CHECK (kind IN ('archived', 'deleted'))
        ) STRICT;

        CREATE TABLE IF NOT EXISTS lifecycle_operations (
            store_incarnation TEXT NOT NULL,
            operation_id TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('archive', 'delete')),
            thread_id TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            outcome TEXT NOT NULL CHECK (outcome IN (
                'applied_changed', 'applied_noop',
                'rejected_conflict', 'rejected_not_found'
            )),
            result_payload TEXT,
            detail TEXT,
            completed_at TEXT NOT NULL,
            PRIMARY KEY (store_incarnation, operation_id)
        ) STRICT;

        CREATE TABLE IF NOT EXISTS cleanup_outbox (
            job_id INTEGER PRIMARY KEY AUTOINCREMENT,
            thread_id TEXT NOT NULL,
            step TEXT NOT NULL CHECK (step IN (
                'endpoint_runtime_invalidate', 'runtime_teardown',
                'transcript_remove', 'thread_log_remove',
                'prompt_attachments_remove'
            )),
            payload TEXT,
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'done')),
            attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
            next_attempt_at TEXT,
            created_at TEXT NOT NULL,
            settled_at TEXT
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_cleanup_outbox_pending
            ON cleanup_outbox(status, next_attempt_at)
            WHERE status = 'pending';

        CREATE TABLE IF NOT EXISTS recent_threads (
            thread_id TEXT PRIMARY KEY,
            title TEXT NOT NULL DEFAULT '',
            workspace_dir TEXT,
            thread_type TEXT NOT NULL DEFAULT 'chat',
            provider_type TEXT,
            agent_id TEXT,
            message_count INTEGER NOT NULL DEFAULT 0,
            last_message_preview TEXT NOT NULL DEFAULT '',
            recent_run_id TEXT,
            active_run_id TEXT,
            run_state TEXT NOT NULL DEFAULT 'idle',
            updated_at TEXT,
            last_active_at TEXT NOT NULL,
            activity_seq INTEGER NOT NULL DEFAULT 0 CHECK (
                activity_seq >= 0
                AND activity_seq < 9007199254740991
            ),
            recorded_at TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS recent_threads_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            activity_seq INTEGER NOT NULL CHECK (
                activity_seq >= 0
                AND activity_seq < 9007199254740991
            )
        ) STRICT;

        CREATE TABLE IF NOT EXISTS projection_states (
            projection_name TEXT PRIMARY KEY,
            projection_version INTEGER NOT NULL,
            source_row_count INTEGER NOT NULL,
            projected_at TEXT NOT NULL,
            based_on_import_generation INTEGER
        ) STRICT;

        -- Task-number allocator (single row). Allocation happens in one
        -- transaction that also floors the counter against the task
        -- projection's MAX(number), so numbers are strictly increasing
        -- and never reused even if this row lags or is reset.
        CREATE TABLE IF NOT EXISTS task_counter (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            last_allocated INTEGER NOT NULL CHECK (last_allocated >= 0)
        ) STRICT;

        CREATE TABLE IF NOT EXISTS task_projection (
            thread_id TEXT PRIMARY KEY,
            number INTEGER NOT NULL CHECK (number > 0),
            status TEXT NOT NULL CHECK (
                status IN ('todo', 'in_progress', 'in_review', 'done')
            ),
            title TEXT NOT NULL,
            creator_json TEXT NOT NULL,
            creator_id TEXT NOT NULL,
            assignee_json TEXT,
            assignee_id TEXT,
            updated_by_json TEXT NOT NULL,
            executor_json TEXT,
            source_json TEXT,
            source_thread_id TEXT,
            source_task_thread_id TEXT,
            source_task_id TEXT COLLATE NOCASE,
            parent_task_number INTEGER CHECK (
                parent_task_number IS NULL OR parent_task_number > 0
            ),
            source_bot_id TEXT,
            notification_thread_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            source_updated_at TEXT NOT NULL,
            source_events_len INTEGER NOT NULL CHECK (source_events_len >= 0),
            projection_version INTEGER NOT NULL DEFAULT 1,
            projected_at TEXT NOT NULL
        ) STRICT;

        -- Intentionally NON-unique: task identity is thread_id, and legacy
        -- databases can hold duplicate numbers from the retired file-counter
        -- era. The allocator only guarantees strictly-increasing output; the
        -- read side dedupes by number (see task_forest.rs).
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

        CREATE TABLE IF NOT EXISTS thread_meta (
            thread_id TEXT PRIMARY KEY,
            workspace_dir TEXT,
            thread_type TEXT NOT NULL DEFAULT 'chat',
            thread_label TEXT,
            agent_id TEXT,
            provider_type TEXT,
            created_at TEXT,
            updated_at TEXT,
            message_count INTEGER NOT NULL DEFAULT 0,
            last_user_message TEXT,
            last_assistant_message TEXT,
            last_message_preview TEXT,
            recent_run_id TEXT,
            active_run_id TEXT,
            worktree_json TEXT,
            last_delivery_context_json TEXT,
            last_delivery_updated_at TEXT,
            default_list_hidden INTEGER NOT NULL DEFAULT 0,
            sort_updated_at_us INTEGER NOT NULL DEFAULT 0,
            search_text TEXT NOT NULL DEFAULT '',
            provider_key TEXT,
            selected_model TEXT,
            selected_model_reasoning_effort TEXT,
            selected_model_service_tier TEXT,
            sdk_session_id TEXT,
            projection_version INTEGER NOT NULL DEFAULT 6,
            projected_at TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS thread_channel_endpoints (
            endpoint_key TEXT PRIMARY KEY,
            channel TEXT NOT NULL,
            account_id TEXT NOT NULL,
            binding_key TEXT NOT NULL,
            chat_id TEXT NOT NULL DEFAULT '',
            delivery_target_type TEXT NOT NULL DEFAULT 'chat_id',
            delivery_target_id TEXT NOT NULL DEFAULT '',
            display_label TEXT NOT NULL DEFAULT '',
            thread_id TEXT,
            thread_label TEXT,
            workspace_dir TEXT,
            thread_updated_at TEXT,
            last_inbound_at TEXT,
            last_delivery_at TEXT,
            projected_at TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS automation_thread_runs (
            automation_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            workspace_dir TEXT,
            agent_id TEXT,
            automation_label_snapshot TEXT,
            mode TEXT NOT NULL CHECK (mode IN ('generated_thread', 'target_thread')),
            status TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            recorded_at TEXT NOT NULL,
            PRIMARY KEY (automation_id, run_id)
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_automation_thread_runs_automation
            ON automation_thread_runs(automation_id, recorded_at DESC);

        CREATE INDEX IF NOT EXISTS idx_automation_thread_runs_thread
            ON automation_thread_runs(thread_id);

        CREATE UNIQUE INDEX IF NOT EXISTS idx_automation_thread_runs_generated_thread
            ON automation_thread_runs(thread_id)
            WHERE mode = 'generated_thread';

        CREATE TABLE IF NOT EXISTS workspaces (
            path TEXT PRIMARY KEY,
            name TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted_at TEXT,
            pinned_at TEXT
        ) STRICT;

        CREATE TABLE IF NOT EXISTS capsules (
            id            TEXT PRIMARY KEY,
            title         TEXT NOT NULL DEFAULT '',
            description   TEXT NOT NULL DEFAULT '',
            thread_id     TEXT,
            run_id        TEXT,
            agent_id      TEXT,
            provider_type TEXT,
            html_sha256   TEXT NOT NULL,
            byte_size     INTEGER NOT NULL DEFAULT 0,
            revision      INTEGER NOT NULL DEFAULT 1,
            created_at    TEXT NOT NULL,
            updated_at    TEXT NOT NULL,
            favorited_at  TEXT
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_capsules_updated
            ON capsules(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_capsules_thread
            ON capsules(thread_id);

        "#,
    )?;
    meetings::migrate_meetings_pull_era_schema(conn)?;
    conn.execute_batch(meetings::MEETINGS_DDL)?;
    ensure_recent_threads_activity_seq_column(conn)?;
    ensure_recent_threads_meta_row(conn)?;
    ensure_thread_pins_sort_order_column(conn)?;
    ensure_thread_pins_meta_row(conn)?;
    ensure_thread_favorites_meta_row(conn)?;
    ensure_store_incarnation_row(conn)?;
    ensure_archived_threads_kind_column(conn)?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_thread_pins_sort_order
             ON thread_pins(sort_order ASC, pinned_at DESC, thread_id ASC);",
    )?;
    ensure_capsules_favorited_at_column(conn)?;
    ensure_projection_state_import_generation_column(conn)?;
    ensure_thread_meta_projection_columns(conn)?;
    ensure_thread_channel_endpoint_columns(conn)?;
    ensure_thread_channel_endpoint_single_holder_schema(conn)?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_thread_channel_endpoints_thread
            ON thread_channel_endpoints(thread_id);

        CREATE INDEX IF NOT EXISTS idx_thread_channel_endpoints_channel_account
            ON thread_channel_endpoints(channel, account_id);
        "#,
    )?;
    ensure_thread_meta_membership_columns(conn)?;
    ensure_thread_meta_indexes(conn)?;
    ensure_workspaces_deleted_at_column(conn)?;
    ensure_workspaces_pinned_at_column(conn)?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_workspaces_active_name_path
            ON workspaces(deleted_at, lower(COALESCE(NULLIF(name, ''), path)), lower(path));
        "#,
    )?;
    Ok(())
}

/// Destructive upgrade cleanup for the removed Workflow product. Old runs,
/// task-backed runs, and child threads are deleted rather than decoded or
/// adapted; no compatibility representation survives normal startup.
pub(super) fn purge_retired_workflow_state(conn: &Connection) -> GaryxDbResult<()> {
    let tx = conn.unchecked_transaction()?;
    let mut retired_thread_ids = BTreeSet::new();
    let mut removed_any_pin = false;
    let mut removed_any_favorite = false;

    if sqlite_table_exists(&tx, "workflow_runs")? {
        // `task_thread_id` was added after the first Workflow schema. Read
        // only the original primary key here; task-backed threads are also
        // discovered authoritatively from their record/projection executor.
        let mut stmt = tx.prepare("SELECT workflow_id FROM workflow_runs")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            insert_thread_id(&mut retired_thread_ids, &row?);
        }
    }

    if sqlite_table_exists(&tx, "workflow_child_runs")? {
        let mut stmt = tx.prepare("SELECT thread_id FROM workflow_child_runs")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            insert_thread_id(&mut retired_thread_ids, &row?);
        }
    }

    {
        let mut stmt = tx.prepare(
            "SELECT thread_id, executor_json
             FROM task_projection
             WHERE executor_json IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (thread_id, executor_json) = row?;
            if is_retired_workflow_executor_json(&executor_json) {
                insert_thread_id(&mut retired_thread_ids, &thread_id);
            }
        }
    }

    {
        let mut stmt =
            tx.prepare("SELECT thread_id FROM thread_meta WHERE thread_type = 'workflow_run'")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            insert_thread_id(&mut retired_thread_ids, &row?);
        }
    }

    {
        let mut stmt = tx.prepare(
            "SELECT key, body
             FROM thread_records
             WHERE instr(lower(body), 'workflow') > 0",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (key, body) = row?;
            if serde_json::from_str::<Value>(&body)
                .ok()
                .as_ref()
                .is_some_and(is_retired_workflow_thread_record)
            {
                insert_thread_id(&mut retired_thread_ids, &key);
            }
        }
    }

    for thread_id in &retired_thread_ids {
        tx.execute(
            "DELETE FROM thread_records WHERE key = ?1",
            params![thread_id],
        )?;
        remove_thread_meta_projection_tx(&tx, thread_id)?;
        remove_task_projection_tx(&tx, thread_id)?;
        remove_recent_thread_tx(&tx, thread_id)?;
        removed_any_pin |= tx.execute(
            "DELETE FROM thread_pins WHERE thread_id = ?1",
            params![thread_id],
        )? > 0;
        removed_any_favorite |= tx.execute(
            "DELETE FROM thread_favorites WHERE thread_id = ?1",
            params![thread_id],
        )? > 0;
        tx.execute(
            "DELETE FROM archived_threads WHERE thread_id = ?1",
            params![thread_id],
        )?;
        tx.execute(
            "DELETE FROM automation_thread_runs WHERE thread_id = ?1",
            params![thread_id],
        )?;
        tx.execute(
            "UPDATE capsules SET thread_id = NULL WHERE thread_id = ?1",
            params![thread_id],
        )?;
    }

    tx.execute_batch(
        r#"
        DROP TABLE IF EXISTS workflow_events;
        DROP TABLE IF EXISTS workflow_child_runs;
        DROP TABLE IF EXISTS workflow_runs;
        "#,
    )?;
    bump_thread_pins_revision_if_changed_tx(&tx, removed_any_pin)?;
    // This runs under the process-lifetime data-dir lock and before listener
    // bind, so there can be no in-flight HTTP writer to fence when no row was
    // removed. Preserve the collection revision on a no-op purge.
    bump_thread_favorites_revision_if_changed_tx(&tx, removed_any_favorite)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn sqlite_table_exists(conn: &Connection, table_name: &str) -> GaryxDbResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
        params![table_name],
        |row| row.get(0),
    )?)
}

pub(super) fn is_retired_workflow_executor_json(raw: &str) -> bool {
    serde_json::from_str::<Value>(raw)
        .ok()
        .is_some_and(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|executor_type| executor_type.eq_ignore_ascii_case("workflow"))
        })
}

pub(crate) fn is_retired_workflow_thread_record(data: &Value) -> bool {
    let Some(record) = data.as_object() else {
        return false;
    };
    let task_uses_workflow = record
        .get("task")
        .and_then(Value::as_object)
        .and_then(|task| task.get("executor"))
        .and_then(Value::as_object)
        .and_then(|executor| executor.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|executor_type| executor_type.eq_ignore_ascii_case("workflow"));
    task_uses_workflow
        || object_marks_retired_workflow(record)
        || record
            .get("metadata")
            .and_then(Value::as_object)
            .is_some_and(object_marks_retired_workflow)
}

pub(super) fn object_marks_retired_workflow(object: &serde_json::Map<String, Value>) -> bool {
    ["thread_kind", "thread_type"]
        .into_iter()
        .filter_map(|key| object.get(key).and_then(Value::as_str))
        .any(|value| value.eq_ignore_ascii_case("workflow_run"))
        || object
            .get("source")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("workflow"))
        || object
            .get("workflow_thread")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || [
            "workflow_run_id",
            "workflowRunId",
            "workflow_child_run_id",
            "workflowChildRunId",
        ]
        .into_iter()
        .filter_map(|key| object.get(key).and_then(Value::as_str))
        .any(|value| !value.trim().is_empty())
}

pub(super) fn ensure_thread_pins_sort_order_column(conn: &Connection) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(thread_pins)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<BTreeSet<_>, _>>()?;
    if columns.contains("sort_order") {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE thread_pins
             ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
        [],
    )?;
    Ok(())
}

pub(super) fn ensure_recent_threads_activity_seq_column(conn: &Connection) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(recent_threads)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<BTreeSet<_>, _>>()?;
    if columns.contains("activity_seq") {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE recent_threads
             ADD COLUMN activity_seq INTEGER NOT NULL DEFAULT 0 CHECK (
                 activity_seq >= 0
                 AND activity_seq < 9007199254740991
             )",
        [],
    )?;
    Ok(())
}

pub(super) fn ensure_recent_threads_meta_row(conn: &Connection) -> GaryxDbResult<()> {
    conn.execute(
        "INSERT INTO recent_threads_meta (id, activity_seq)
         VALUES (1, 0)
         ON CONFLICT(id) DO NOTHING",
        [],
    )?;
    Ok(())
}

pub(super) fn ensure_thread_pins_meta_row(conn: &Connection) -> GaryxDbResult<()> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM thread_pins_meta WHERE id = 1",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        conn.execute(
            "INSERT INTO thread_pins_meta (id, pins_revision) VALUES (1, 0)",
            [],
        )?;
    }
    Ok(())
}

pub(super) fn ensure_thread_favorites_meta_row(conn: &Connection) -> GaryxDbResult<()> {
    conn.execute(
        "INSERT INTO thread_favorites_meta (id, favorites_revision)
         VALUES (1, 0)
         ON CONFLICT(id) DO NOTHING",
        [],
    )?;
    Ok(())
}

pub(super) fn ensure_archived_threads_kind_column(conn: &Connection) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(archived_threads)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == "kind" {
            return Ok(());
        }
    }
    // Every pre-cutover row was produced by archive, so the only correct
    // migration value is `archived`.
    conn.execute(
        "ALTER TABLE archived_threads
         ADD COLUMN kind TEXT NOT NULL DEFAULT 'archived'
         CHECK (kind IN ('archived', 'deleted'))",
        [],
    )?;
    Ok(())
}

pub(super) fn ensure_workspaces_deleted_at_column(conn: &Connection) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(workspaces)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == "deleted_at" {
            return Ok(());
        }
    }
    conn.execute("ALTER TABLE workspaces ADD COLUMN deleted_at TEXT", [])?;
    Ok(())
}

pub(super) fn ensure_workspaces_pinned_at_column(conn: &Connection) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(workspaces)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == "pinned_at" {
            return Ok(());
        }
    }
    conn.execute("ALTER TABLE workspaces ADD COLUMN pinned_at TEXT", [])?;
    Ok(())
}

/// `root_workspace_path` and `workspace_origin` are plain projected columns
/// written by the same-transaction projection derivation. The single source
/// of truth for both derivations is Rust
/// (`workspace_mode::thread_workspace_origin` /
/// `thread_root_workspace_path`), so there is no SQL twin to drift — the
/// legacy VIRTUAL generated column (whose `REPLACE(thread_id, ':', '-')`
/// could disagree with the full sanitizer on unusual thread ids) is dropped
/// on sight. Historical rows are populated by the versioned
/// `thread_meta_workspace_membership_v1` cutover.
pub(super) fn ensure_thread_meta_membership_columns(conn: &Connection) -> GaryxDbResult<()> {
    // A worktree that ran an earlier revision may still carry the retired
    // VIRTUAL generated column; table_xinfo sees it, table_info does not.
    let mut stmt = conn.prepare("PRAGMA table_xinfo(thread_meta)")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(6)?))
    })?;
    let mut has_root = false;
    let mut root_is_generated = false;
    let mut has_origin = false;
    for row in rows {
        let (name, hidden) = row?;
        if name == "root_workspace_path" {
            has_root = true;
            root_is_generated = hidden != 0;
        } else if name == "workspace_origin" {
            has_origin = true;
        }
    }
    drop(stmt);
    if root_is_generated {
        // The previous revision indexed the generated column; SQLite refuses
        // to drop a column an index depends on. The plain-column index is
        // recreated by ensure_thread_meta_indexes right after.
        conn.execute("DROP INDEX IF EXISTS idx_thread_meta_root_workspace", [])?;
        conn.execute("ALTER TABLE thread_meta DROP COLUMN root_workspace_path", [])?;
        has_root = false;
    }
    if !has_root {
        conn.execute(
            "ALTER TABLE thread_meta ADD COLUMN root_workspace_path TEXT",
            [],
        )?;
    }
    if !has_origin {
        conn.execute(
            "ALTER TABLE thread_meta ADD COLUMN workspace_origin TEXT",
            [],
        )?;
    }
    Ok(())
}

pub(super) fn ensure_capsules_favorited_at_column(conn: &Connection) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(capsules)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == "favorited_at" {
            return Ok(());
        }
    }
    conn.execute("ALTER TABLE capsules ADD COLUMN favorited_at TEXT", [])?;
    Ok(())
}

pub(super) fn ensure_projection_state_import_generation_column(
    conn: &Connection,
) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(projection_states)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == "based_on_import_generation" {
            return Ok(());
        }
    }
    conn.execute(
        "ALTER TABLE projection_states ADD COLUMN based_on_import_generation INTEGER",
        [],
    )?;
    Ok(())
}

pub(super) fn thread_meta_column_names(conn: &Connection) -> GaryxDbResult<Vec<String>> {
    let mut stmt = conn.prepare("PRAGMA table_info(thread_meta)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub(super) fn ensure_thread_meta_indexes(conn: &Connection) -> GaryxDbResult<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_thread_meta_root_workspace
             ON thread_meta(root_workspace_path, sort_updated_at_us DESC)
             WHERE root_workspace_path IS NOT NULL;
         CREATE INDEX IF NOT EXISTS idx_thread_meta_workspace
             ON thread_meta(workspace_dir);
         CREATE INDEX IF NOT EXISTS idx_thread_meta_type_updated
             ON thread_meta(thread_type, updated_at DESC);
         CREATE INDEX IF NOT EXISTS idx_thread_meta_last_delivery
             ON thread_meta(last_delivery_updated_at DESC)
             WHERE last_delivery_context_json IS NOT NULL;
         CREATE INDEX IF NOT EXISTS idx_thread_meta_visible_updated
             ON thread_meta(default_list_hidden, updated_at DESC, projected_at DESC);
         CREATE INDEX IF NOT EXISTS idx_thread_meta_summary_visible
             ON thread_meta(sort_updated_at_us DESC, thread_id DESC)
             WHERE default_list_hidden = 0;
         CREATE INDEX IF NOT EXISTS idx_thread_meta_summary_task
             ON thread_meta(sort_updated_at_us DESC, thread_id DESC)
             WHERE default_list_hidden = 0 AND thread_type = 'task';
         CREATE INDEX IF NOT EXISTS idx_thread_meta_summary_non_task
             ON thread_meta(sort_updated_at_us DESC, thread_id DESC)
             WHERE default_list_hidden = 0 AND thread_type <> 'task';
         CREATE INDEX IF NOT EXISTS idx_thread_meta_summary_workspace_visible
             ON thread_meta(workspace_dir, sort_updated_at_us DESC, thread_id DESC)
             WHERE default_list_hidden = 0;
         CREATE INDEX IF NOT EXISTS idx_thread_meta_summary_workspace_task
             ON thread_meta(workspace_dir, sort_updated_at_us DESC, thread_id DESC)
             WHERE default_list_hidden = 0 AND thread_type = 'task';
         CREATE INDEX IF NOT EXISTS idx_thread_meta_summary_workspace_non_task
             ON thread_meta(workspace_dir, sort_updated_at_us DESC, thread_id DESC)
             WHERE default_list_hidden = 0 AND thread_type <> 'task';",
    )?;
    Ok(())
}

pub(super) fn ensure_thread_meta_projection_columns(conn: &Connection) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(thread_meta)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = BTreeSet::new();
    for row in rows {
        columns.insert(row?);
    }
    if !columns.contains("default_list_hidden") {
        conn.execute(
            "ALTER TABLE thread_meta
             ADD COLUMN default_list_hidden INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    for name in [
        "created_at",
        "last_user_message",
        "last_assistant_message",
        "last_message_preview",
        "recent_run_id",
        "active_run_id",
        "worktree_json",
        "provider_key",
        "selected_model",
        "selected_model_reasoning_effort",
        "selected_model_service_tier",
        "sdk_session_id",
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE thread_meta ADD COLUMN {name} TEXT"),
                [],
            )?;
        }
    }
    if !columns.contains("message_count") {
        conn.execute(
            "ALTER TABLE thread_meta
             ADD COLUMN message_count INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !columns.contains("sort_updated_at_us") {
        conn.execute(
            "ALTER TABLE thread_meta
             ADD COLUMN sort_updated_at_us INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !columns.contains("search_text") {
        conn.execute(
            "ALTER TABLE thread_meta
             ADD COLUMN search_text TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !columns.contains("projection_version") {
        conn.execute(
            "ALTER TABLE thread_meta
             ADD COLUMN projection_version INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

pub(super) fn ensure_thread_channel_endpoint_columns(conn: &Connection) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(thread_channel_endpoints)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = BTreeSet::new();
    for row in rows {
        columns.insert(row?);
    }
    for (name, sql_type) in [
        ("chat_id", "TEXT NOT NULL DEFAULT ''"),
        ("delivery_target_type", "TEXT NOT NULL DEFAULT 'chat_id'"),
        ("delivery_target_id", "TEXT NOT NULL DEFAULT ''"),
        ("display_label", "TEXT NOT NULL DEFAULT ''"),
        ("thread_id", "TEXT"),
        ("thread_label", "TEXT"),
        ("workspace_dir", "TEXT"),
        ("thread_updated_at", "TEXT"),
        ("last_inbound_at", "TEXT"),
        ("last_delivery_at", "TEXT"),
        ("projected_at", "TEXT NOT NULL DEFAULT ''"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE thread_channel_endpoints ADD COLUMN {name} {sql_type}"),
                [],
            )?;
        }
    }
    Ok(())
}

/// Restore the single-owner endpoint schema after versions that stored one
/// row per `(endpoint_key, thread_id)`. `CREATE TABLE IF NOT EXISTS` cannot
/// change that composite primary key, so current `ON CONFLICT(endpoint_key)`
/// writes otherwise fail at prepare time. The endpoint table is derived state:
/// rebuild it atomically and clear the holder-dedup marker so the existing
/// post-import startup migration repopulates it from canonical thread records.
pub(super) fn ensure_thread_channel_endpoint_single_holder_schema(
    conn: &Connection,
) -> GaryxDbResult<()> {
    let primary_key_columns = {
        let mut stmt = conn.prepare(
            "SELECT name
               FROM pragma_table_info('thread_channel_endpoints')
              WHERE pk > 0
              ORDER BY pk",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut columns = Vec::new();
        for row in rows {
            columns.push(row?);
        }
        columns
    };
    if primary_key_columns == ["endpoint_key"] {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute("DROP TABLE thread_channel_endpoints", [])?;
    tx.execute_batch(
        r#"
        CREATE TABLE thread_channel_endpoints (
            endpoint_key TEXT PRIMARY KEY,
            channel TEXT NOT NULL,
            account_id TEXT NOT NULL,
            binding_key TEXT NOT NULL,
            chat_id TEXT NOT NULL DEFAULT '',
            delivery_target_type TEXT NOT NULL DEFAULT 'chat_id',
            delivery_target_id TEXT NOT NULL DEFAULT '',
            display_label TEXT NOT NULL DEFAULT '',
            thread_id TEXT,
            thread_label TEXT,
            workspace_dir TEXT,
            thread_updated_at TEXT,
            last_inbound_at TEXT,
            last_delivery_at TEXT,
            projected_at TEXT NOT NULL
        ) STRICT;
        "#,
    )?;
    tx.execute(
        "DELETE FROM projection_states WHERE projection_name = ?1",
        params![ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME],
    )?;
    tx.commit()?;
    tracing::info!(
        ?primary_key_columns,
        "rebuilt legacy thread endpoint projection for single-holder ownership"
    );
    Ok(())
}
