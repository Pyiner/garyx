use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use garyx_models::{Principal, TaskExecutor, TaskSource, TaskStatus};
use garyx_router::tasks::{TaskListFilter, TaskSummary};
use rusqlite::types::{Type, Value as SqlValue};
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::task_tree::{RawTaskNode, layout_anchored_task_tree, task_node_id, thread_root_node_id};

use super::{GaryxDbError, GaryxDbResult, GaryxDbService, normalize_optional, normalize_thread_id};

pub const CURRENT_TASK_PROJECTION_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskProjectionDraft {
    pub thread_id: String,
    pub number: u64,
    pub status: String,
    pub title: String,
    pub creator_json: String,
    pub creator_id: String,
    pub assignee_json: Option<String>,
    pub assignee_id: Option<String>,
    pub updated_by_json: String,
    pub executor_json: Option<String>,
    pub source_json: Option<String>,
    pub source_thread_id: Option<String>,
    pub source_task_thread_id: Option<String>,
    pub source_task_id: Option<String>,
    pub parent_task_number: Option<u64>,
    pub source_bot_id: Option<String>,
    pub notification_thread_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub source_updated_at: String,
    pub source_events_len: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum TaskForestNode {
    Thread {
        node_id: String,
        thread_id: String,
        title: String,
        thread_type: String,
        provider_type: Option<String>,
        agent_id: Option<String>,
        message_count: u32,
        last_message_preview: String,
        active_run_id: Option<String>,
        run_state: String,
        updated_at: Option<String>,
        last_active_at: Option<String>,
        /// DFS depth in anchored mode (thread root = 0); absent in console modes.
        #[serde(skip_serializing_if = "Option::is_none")]
        depth: Option<u32>,
    },
    Task {
        node_id: String,
        parent_node_id: Option<String>,
        #[serde(flatten)]
        task: TaskSummary,
        parent_task_number: Option<u64>,
        parent_thread_id: Option<String>,
        active_run_id: Option<String>,
        run_state: String,
        last_active_at: Option<String>,
        /// DFS depth in anchored mode; absent in console modes.
        #[serde(skip_serializing_if = "Option::is_none")]
        depth: Option<u32>,
    },
}

impl TaskForestNode {
    #[cfg(test)]
    fn thread_id(&self) -> &str {
        match self {
            TaskForestNode::Thread { thread_id, .. } => thread_id,
            TaskForestNode::Task { task, .. } => &task.thread_id,
        }
    }

    #[cfg(test)]
    fn parent_thread_id(&self) -> Option<&str> {
        match self {
            TaskForestNode::Task {
                parent_thread_id, ..
            } => parent_thread_id.as_deref(),
            TaskForestNode::Thread { .. } => None,
        }
    }

    #[cfg(test)]
    fn parent_task_number(&self) -> Option<u64> {
        match self {
            TaskForestNode::Task {
                parent_task_number, ..
            } => *parent_task_number,
            TaskForestNode::Thread { .. } => None,
        }
    }

    #[cfg(test)]
    fn parent_node_id(&self) -> Option<&str> {
        match self {
            TaskForestNode::Task { parent_node_id, .. } => parent_node_id.as_deref(),
            TaskForestNode::Thread { .. } => None,
        }
    }

    #[cfg(test)]
    fn active_run_id(&self) -> Option<&str> {
        match self {
            TaskForestNode::Task { active_run_id, .. }
            | TaskForestNode::Thread { active_run_id, .. } => active_run_id.as_deref(),
        }
    }

    #[cfg(test)]
    fn run_state(&self) -> &str {
        match self {
            TaskForestNode::Task { run_state, .. } | TaskForestNode::Thread { run_state, .. } => {
                run_state
            }
        }
    }

    #[cfg(test)]
    fn last_active_at(&self) -> Option<&str> {
        match self {
            TaskForestNode::Task { last_active_at, .. }
            | TaskForestNode::Thread { last_active_at, .. } => last_active_at.as_deref(),
        }
    }

    #[cfg(test)]
    fn depth(&self) -> Option<u32> {
        match self {
            TaskForestNode::Task { depth, .. } | TaskForestNode::Thread { depth, .. } => *depth,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskForestPage {
    pub tasks: Vec<TaskForestNode>,
    pub total: usize,
    pub root_thread_ids: Vec<String>,
    pub skipped_pinned_thread_ids: Vec<String>,
    /// Active badge count (`in_progress` + `in_review`) in anchored mode;
    /// absent in console modes so clients keep their local recount there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_count: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskForestScope {
    Pinned,
    All,
}

#[allow(clippy::derivable_impls)]
impl Default for TaskForestScope {
    fn default() -> Self {
        Self::Pinned
    }
}

#[derive(Debug, Clone)]
struct PinnedTaskForestRow {
    root_thread_id: String,
    thread_id: String,
    number: u64,
    status: TaskStatus,
    parent_task_number: Option<u64>,
    source_task_number: Option<u64>,
    source_task_thread_id: Option<String>,
    node: TaskForestNode,
}

impl PinnedTaskForestRow {
    fn is_retained_status(&self) -> bool {
        matches!(self.status, TaskStatus::InProgress | TaskStatus::InReview)
    }
}

impl GaryxDbService {
    /// Allocate the next task number: bump the single-row counter while
    /// flooring it against the task projection's `MAX(number)`, all in
    /// one transaction. The allocator guarantee is strictly-increasing
    /// output: the returned number is greater than every previously
    /// allocated number and every number currently in the projection.
    ///
    /// This is NOT a database-level uniqueness guarantee on
    /// `task_projection.number` (#TASK-2099 root review finding 5,
    /// REFUTED for a UNIQUE constraint): task identity is `thread_id`,
    /// legacy databases may already hold duplicate numbers from the
    /// retired file-counter era, and a UNIQUE index would make those
    /// existing rows unwritable. `idx_task_projection_number` stays
    /// non-unique on purpose; the read side dedupes by number and logs
    /// when duplicates are observed (see `list_task_summaries`).
    pub fn allocate_task_number(&self) -> GaryxDbResult<u64> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO task_counter (id, last_allocated) VALUES (1, 0)
             ON CONFLICT(id) DO NOTHING",
            [],
        )?;
        let number: i64 = tx.query_row(
            "UPDATE task_counter
             SET last_allocated = MAX(
                     last_allocated,
                     (SELECT COALESCE(MAX(number), 0) FROM task_projection)
                 ) + 1
             WHERE id = 1
             RETURNING last_allocated",
            [],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(number.max(1) as u64)
    }

    /// One-shot migration seed for the task counter (pre-SQLite installs).
    /// When no counter row exists yet, seed `last_allocated` with the
    /// highest of: the caller-provided floor (the legacy file counter)
    /// and the highest task number embedded in any thread record body
    /// (covers archived threads and done tasks whose projections were
    /// removed). Returns whether a row was seeded.
    pub fn seed_task_counter_if_missing(&self, floor: u64) -> GaryxDbResult<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let exists: Option<i64> = tx
            .query_row("SELECT 1 FROM task_counter WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;
        if exists.is_some() {
            tx.commit()?;
            return Ok(false);
        }
        let records_max: i64 = tx.query_row(
            "SELECT COALESCE(
                 MAX(CAST(json_extract(body, '$.task.number') AS INTEGER)), 0
             )
             FROM thread_records",
            [],
            |row| row.get(0),
        )?;
        let seed = floor.max(records_max.max(0) as u64);
        tx.execute(
            "INSERT INTO task_counter (id, last_allocated) VALUES (1, ?1)",
            params![i64::try_from(seed).unwrap_or(i64::MAX)],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Test-fixture seeding only: production task rows derive in the
    /// same transaction as the record write
    /// (`write_thread_record_with_projections`).
    #[cfg(test)]
    pub fn replace_task_projection(&self, draft: TaskProjectionDraft) -> GaryxDbResult<()> {
        let thread_id = normalize_thread_id(&draft.thread_id)?;
        let mut draft = draft;
        draft.thread_id = thread_id;
        let projected_at = super::now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        upsert_task_projection(&tx, &draft, &projected_at)?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_task_projection(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.conn()?;
        let removed = conn.execute(
            "DELETE FROM task_projection WHERE thread_id = ?1",
            params![thread_id],
        )?;
        Ok(removed > 0)
    }

    pub fn thread_id_for_number(&self, number: u64) -> GaryxDbResult<Option<String>> {
        let number = i64::try_from(number).unwrap_or(i64::MAX);
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT thread_id
             FROM task_projection
             WHERE number = ?1 AND projection_version = ?2
             ORDER BY updated_at DESC, thread_id ASC
             LIMIT 1",
            params![number, CURRENT_TASK_PROJECTION_VERSION],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn has_running_subtask_targeting(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.read_conn()?;
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1
                 FROM task_projection
                 WHERE status = 'in_progress'
                   AND notification_thread_id = ?1
                   AND thread_id <> ?1
                   AND projection_version = ?2
                 LIMIT 1",
                params![thread_id, CURRENT_TASK_PROJECTION_VERSION],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn list_task_summaries(
        &self,
        filter: &TaskListFilter,
    ) -> GaryxDbResult<(Vec<TaskSummary>, usize, bool)> {
        let limit = filter.limit.unwrap_or(50).clamp(1, 200);
        let offset = filter.offset.unwrap_or(0);
        let (where_sql, bind_values) = task_projection_filter_sql(filter)?;
        let duplicate_count_sql = format!(
            "SELECT COUNT(*)
             FROM (
                SELECT number
                FROM task_projection task
                WHERE {where_sql}
                GROUP BY number
                HAVING COUNT(*) > 1
             )"
        );
        let count_sql = format!(
            "WITH filtered AS (
                SELECT task.thread_id, task.number, task.updated_at,
                       ROW_NUMBER() OVER (
                           PARTITION BY task.number
                           ORDER BY task.updated_at DESC, task.thread_id ASC
                       ) AS rn
                FROM task_projection task
                WHERE {where_sql}
             )
             SELECT COUNT(*) FROM filtered WHERE rn = 1"
        );
        let list_sql = format!(
            "WITH filtered AS (
                SELECT task.thread_id, task.number, task.status, task.title,
                       task.creator_json, task.assignee_json, task.source_json,
                       task.executor_json, task.updated_at, task.updated_by_json,
                       COALESCE(meta.agent_id, '') AS runtime_agent_id,
                       COALESCE(meta.message_count, 0) AS reply_count,
                       ROW_NUMBER() OVER (
                           PARTITION BY task.number
                           ORDER BY task.updated_at DESC, task.thread_id ASC
                       ) AS rn
                FROM task_projection task
                LEFT JOIN thread_meta meta ON meta.thread_id = task.thread_id
                WHERE {where_sql}
             )
             SELECT thread_id, number, status, title, creator_json, assignee_json,
                    source_json, executor_json, updated_at, updated_by_json,
                    runtime_agent_id, reply_count
             FROM filtered
             WHERE rn = 1
             ORDER BY updated_at DESC, thread_id ASC
             LIMIT ? OFFSET ?"
        );

        let conn = self.read_conn()?;
        let duplicate_count: i64 = conn.query_row(
            &duplicate_count_sql,
            params_from_iter(bind_values.iter()),
            |row| row.get(0),
        )?;
        if duplicate_count > 0 {
            tracing::warn!(
                duplicate_task_numbers = duplicate_count,
                "task projection contains duplicate task numbers; list results are deduped"
            );
        }
        let total: i64 =
            conn.query_row(&count_sql, params_from_iter(bind_values.iter()), |row| {
                row.get(0)
            })?;
        let mut list_bind_values = bind_values;
        list_bind_values.push(SqlValue::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));
        list_bind_values.push(SqlValue::Integer(i64::try_from(offset).unwrap_or(i64::MAX)));
        let mut stmt = conn.prepare(&list_sql)?;
        let rows = stmt.query_map(
            params_from_iter(list_bind_values.iter()),
            task_summary_from_row,
        )?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        let total = usize::try_from(total).unwrap_or(usize::MAX);
        let has_more = offset.saturating_add(tasks.len()) < total;
        Ok((tasks, total, has_more))
    }

    pub fn list_task_forest(
        &self,
        filter: &TaskListFilter,
        scope: TaskForestScope,
    ) -> GaryxDbResult<TaskForestPage> {
        match scope {
            TaskForestScope::Pinned => self.list_pinned_task_forest(filter),
            TaskForestScope::All => {
                let (tasks, total) = self.list_all_task_forest(filter)?;
                Ok(TaskForestPage {
                    tasks,
                    total,
                    root_thread_ids: Vec::new(),
                    skipped_pinned_thread_ids: Vec::new(),
                    active_count: None,
                })
            }
        }
    }

    fn list_all_task_forest(
        &self,
        filter: &TaskListFilter,
    ) -> GaryxDbResult<(Vec<TaskForestNode>, usize)> {
        let (where_sql, bind_values) = task_projection_filter_sql(filter)?;
        let count_sql = format!(
            "WITH filtered AS (
                SELECT task.thread_id, task.number, task.updated_at,
                       ROW_NUMBER() OVER (
                           PARTITION BY task.number
                           ORDER BY task.updated_at DESC, task.thread_id ASC
                       ) AS rn
                FROM task_projection task
                WHERE {where_sql}
             )
             SELECT COUNT(*) FROM filtered WHERE rn = 1"
        );
        let list_sql = format!(
            "WITH filtered AS (
                SELECT task.thread_id, task.number, task.status, task.title,
                       task.creator_json, task.assignee_json, task.source_json,
                       task.executor_json, task.updated_at, task.updated_by_json,
                       COALESCE(meta.agent_id, '') AS runtime_agent_id,
                       COALESCE(meta.message_count, 0) AS reply_count,
                       task.parent_task_number,
                       COALESCE(
                           task.source_task_thread_id,
                           (
                               SELECT parent.thread_id
                               FROM task_projection parent
                               WHERE parent.projection_version = ?
                                 AND (
                                   parent.number = task.parent_task_number
                                   OR task.source_task_id = ('#TASK-' || parent.number) COLLATE NOCASE
                                 )
                               ORDER BY parent.updated_at DESC, parent.thread_id ASC
                               LIMIT 1
                           )
                       ) AS parent_thread_id,
                       recent.active_run_id,
                       recent.run_state,
                       recent.last_active_at,
                       ROW_NUMBER() OVER (
                           PARTITION BY task.number
                           ORDER BY task.updated_at DESC, task.thread_id ASC
                       ) AS rn
                FROM task_projection task
                LEFT JOIN thread_meta meta ON meta.thread_id = task.thread_id
                LEFT JOIN recent_threads recent ON recent.thread_id = task.thread_id
                WHERE {where_sql}
             )
             SELECT thread_id, number, status, title, creator_json, assignee_json,
                    source_json, executor_json, updated_at, updated_by_json,
                    runtime_agent_id, reply_count, parent_task_number,
                    parent_thread_id, active_run_id, COALESCE(run_state, 'idle') AS run_state,
                    last_active_at
             FROM filtered
             WHERE rn = 1
             ORDER BY updated_at DESC, thread_id ASC"
        );

        let conn = self.read_conn()?;
        let total: i64 =
            conn.query_row(&count_sql, params_from_iter(bind_values.iter()), |row| {
                row.get(0)
            })?;
        let mut list_bind_values = vec![SqlValue::Integer(CURRENT_TASK_PROJECTION_VERSION)];
        list_bind_values.extend(bind_values);
        let mut stmt = conn.prepare(&list_sql)?;
        let rows = stmt.query_map(params_from_iter(list_bind_values.iter()), |row| {
            task_forest_task_from_row(row, None)
        })?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok((tasks, usize::try_from(total).unwrap_or(usize::MAX)))
    }

    pub fn list_task_forest_anchored(
        &self,
        anchor_thread_id: &str,
        _filter: &TaskListFilter,
    ) -> GaryxDbResult<TaskForestPage> {
        let anchor_thread_id = normalize_thread_id(anchor_thread_id)?;
        let conn = self.read_conn()?;
        let anchor_is_task = conn.query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM task_projection
                WHERE thread_id = ?1
                  AND projection_version = ?2
             )",
            params![anchor_thread_id, CURRENT_TASK_PROJECTION_VERSION],
            |row| row.get::<_, i64>(0).map(|value| value != 0),
        )?;
        let raw_filter = TaskListFilter {
            include_done: true,
            ..TaskListFilter::default()
        };
        let (where_sql, bind_values) = task_projection_filter_sql(&raw_filter)?;

        if anchor_is_task {
            // Climb to the topmost task first: when its origin conversation is
            // known, the anchor sees the same origin-rooted forest as the
            // conversation anchor (only the client-side highlight differs).
            let Some(root) = anchored_task_root_from_conn(
                &conn,
                &where_sql,
                bind_values.clone(),
                &anchor_thread_id,
            )?
            else {
                return Ok(empty_anchored_task_forest_page());
            };
            if let Some(origin) = root.source_thread_id {
                return origin_rooted_task_forest_from_conn(
                    &conn,
                    &where_sql,
                    bind_values,
                    &origin,
                );
            }
            // Legacy fallback: no origin conversation on record, so the tree
            // stays rooted at the topmost task without a synthetic root.
            let raw = climbed_task_tree_rows_from_conn(
                &conn,
                &where_sql,
                bind_values,
                &anchor_thread_id,
            )?;
            let layout = layout_anchored_task_tree(raw, None);
            let root_thread_ids = if layout.nodes.is_empty() {
                Vec::new()
            } else {
                vec![root.thread_id]
            };
            return Ok(TaskForestPage {
                total: layout.nodes.len(),
                tasks: layout.nodes,
                root_thread_ids,
                skipped_pinned_thread_ids: Vec::new(),
                active_count: Some(layout.active_count),
            });
        }

        origin_rooted_task_forest_from_conn(&conn, &where_sql, bind_values, &anchor_thread_id)
    }

    fn list_pinned_task_forest(&self, filter: &TaskListFilter) -> GaryxDbResult<TaskForestPage> {
        self.task_forest_rooted_or_pinned(filter)
    }

    /// Shared engine for the pinned-roots forest. The chat-page task popover
    /// uses `list_task_forest_anchored`; this path keeps the console contract.
    fn task_forest_rooted_or_pinned(
        &self,
        filter: &TaskListFilter,
    ) -> GaryxDbResult<TaskForestPage> {
        let (where_sql, bind_values) = task_projection_filter_sql(filter)?;
        let pinned_cte = "SELECT thread_id,
                       ROW_NUMBER() OVER (
                           ORDER BY sort_order ASC, pinned_at DESC, thread_id ASC
                       ) AS root_rank
                FROM thread_pins"
            .to_owned();
        let skipped_sql = "WITH pinned AS (
                SELECT thread_id, pinned_at, sort_order
                FROM thread_pins
             ),
             related AS (
                SELECT DISTINCT pinned.thread_id
                FROM pinned
                JOIN task_projection task
                  ON task.projection_version = ?1
                 AND (
                       task.thread_id = pinned.thread_id
                    OR task.source_thread_id = pinned.thread_id
                 )
             )
             SELECT pinned.thread_id
             FROM pinned
             LEFT JOIN related ON related.thread_id = pinned.thread_id
             WHERE related.thread_id IS NULL
             ORDER BY pinned.sort_order ASC, pinned.pinned_at DESC, pinned.thread_id ASC";
        let list_sql = format!(
            "WITH RECURSIVE pinned AS (
                {pinned_cte}
             ),
             filtered AS (
                SELECT task.thread_id, task.number, task.status, task.title,
                       task.creator_json, task.assignee_json, task.source_json,
                       task.executor_json, task.updated_at, task.updated_by_json,
                       COALESCE(meta.agent_id, '') AS runtime_agent_id,
                       COALESCE(meta.message_count, 0) AS reply_count,
                       task.parent_task_number,
                       task.source_thread_id,
                       task.source_task_thread_id,
                       task.source_task_id,
                       recent.active_run_id,
                       recent.run_state,
                       recent.last_active_at,
                       ROW_NUMBER() OVER (
                           PARTITION BY task.number
                           ORDER BY
                             CASE
                               WHEN task.thread_id IN (SELECT thread_id FROM pinned) THEN 0
                               WHEN task.source_thread_id IN (SELECT thread_id FROM pinned)
                                    AND task.parent_task_number IS NULL
                                    AND task.source_task_id IS NULL THEN 1
                               ELSE 2
                             END,
                             task.updated_at DESC,
                             task.thread_id ASC
                       ) AS rn
                FROM task_projection task
                LEFT JOIN thread_meta meta ON meta.thread_id = task.thread_id
                LEFT JOIN recent_threads recent ON recent.thread_id = task.thread_id
                WHERE {where_sql}
             ),
             deduped AS (
                SELECT * FROM filtered WHERE rn = 1
             ),
             seed_candidates AS (
                SELECT pinned.root_rank,
                       pinned.thread_id AS root_thread_id,
                       deduped.thread_id,
                       deduped.number,
                       ROW_NUMBER() OVER (
                           PARTITION BY pinned.thread_id, deduped.thread_id
                           ORDER BY
                             CASE
                               WHEN deduped.thread_id = pinned.thread_id THEN 0
                               ELSE 1
                             END,
                             deduped.updated_at DESC,
                             deduped.thread_id ASC
                       ) AS seed_rn
                FROM pinned
                JOIN deduped
                  ON deduped.thread_id = pinned.thread_id
                  OR (
                       deduped.source_thread_id = pinned.thread_id
                   AND deduped.parent_task_number IS NULL
                   AND deduped.source_task_id IS NULL
                  )
             ),
             seeds AS (
                SELECT root_rank, root_thread_id, thread_id, number
                FROM seed_candidates
                WHERE seed_rn = 1
             ),
             task_tree(root_rank, root_thread_id, depth, thread_id, number, path) AS (
                SELECT root_rank,
                       root_thread_id,
                       1,
                       thread_id,
                       number,
                       ',' || root_thread_id || ',' || thread_id || ','
                FROM seeds
                UNION ALL
                SELECT task_tree.root_rank,
                       task_tree.root_thread_id,
                       task_tree.depth + 1,
                       child.thread_id,
                       child.number,
                       task_tree.path || child.thread_id || ','
                FROM task_tree
                JOIN deduped child
                  ON child.source_task_thread_id = task_tree.thread_id
                  OR child.parent_task_number = task_tree.number
                  OR child.source_task_id = ('#TASK-' || task_tree.number) COLLATE NOCASE
                WHERE task_tree.depth < 64
                  AND instr(task_tree.path, ',' || child.thread_id || ',') = 0
             ),
             reached AS (
                SELECT root_rank, root_thread_id, depth, thread_id,
                       ROW_NUMBER() OVER (
                           PARTITION BY thread_id
                           ORDER BY root_rank ASC, depth ASC, thread_id ASC
                       ) AS reach_rn
                FROM task_tree
             )
             SELECT deduped.thread_id, deduped.number, deduped.status, deduped.title,
                    deduped.creator_json, deduped.assignee_json,
                    deduped.source_json, deduped.executor_json,
                    deduped.updated_at, deduped.updated_by_json,
                    deduped.runtime_agent_id, deduped.reply_count,
                    deduped.parent_task_number,
                    CASE
                        WHEN reached.depth = 1 THEN reached.root_thread_id
                        ELSE COALESCE(
                            deduped.source_task_thread_id,
                            (
                                SELECT parent.thread_id
                                FROM deduped parent
                                WHERE parent.number = deduped.parent_task_number
                                   OR deduped.source_task_id = ('#TASK-' || parent.number) COLLATE NOCASE
                                ORDER BY parent.updated_at DESC, parent.thread_id ASC
                                LIMIT 1
                            )
                        )
                    END
                    AS parent_thread_id,
                    deduped.active_run_id,
                    COALESCE(deduped.run_state, 'idle') AS run_state,
                    deduped.last_active_at,
                    reached.root_rank,
                    reached.root_thread_id,
                    reached.depth,
                    deduped.source_task_id,
                    deduped.source_task_thread_id
                FROM deduped
                JOIN reached ON reached.thread_id = deduped.thread_id
             WHERE reached.reach_rn = 1
             ORDER BY reached.root_rank ASC, reached.depth ASC, deduped.number ASC, deduped.thread_id ASC"
        );

        let conn = self.read_conn()?;
        let mut skipped_pinned_thread_ids = Vec::new();
        let mut skipped_stmt = conn.prepare(skipped_sql)?;
        let skipped_rows = skipped_stmt
            .query_map(params![CURRENT_TASK_PROJECTION_VERSION], |row| {
                row.get::<_, String>(0)
            })?;
        for row in skipped_rows {
            skipped_pinned_thread_ids.push(row?);
        }

        let list_bind_values = bind_values;

        let mut stmt = conn.prepare(&list_sql)?;
        let rows = stmt.query_map(params_from_iter(list_bind_values.iter()), |row| {
            let root_thread_id = row.get::<_, String>(18)?;
            let source_task_id = row.get::<_, Option<String>>(20)?;
            let source_task_number = source_task_id.as_deref().and_then(task_number_from_task_id);
            let source_task_thread_id = row.get::<_, Option<String>>(21)?;
            let node = task_forest_task_from_row(row, None)?;
            let TaskForestNode::Task {
                task,
                parent_task_number,
                ..
            } = &node
            else {
                unreachable!("task forest SQL rows are task nodes");
            };
            Ok(PinnedTaskForestRow {
                root_thread_id,
                thread_id: task.thread_id.clone(),
                number: task.number,
                status: task.status,
                parent_task_number: *parent_task_number,
                source_task_number,
                source_task_thread_id,
                node,
            })
        })?;
        let mut reached_rows = Vec::new();
        let mut root_thread_ids = Vec::new();
        for row in rows {
            let row = row?;
            let root_thread_id = row.root_thread_id.clone();
            if !root_thread_ids.contains(&root_thread_id) {
                root_thread_ids.push(root_thread_id.clone());
            }
            reached_rows.push(row);
        }
        let mut tasks = Vec::with_capacity(root_thread_ids.len() + reached_rows.len());
        for root_thread_id in &root_thread_ids {
            tasks.push(task_forest_thread_root_from_conn(&conn, root_thread_id)?);
            let root_row_indices = reached_rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| (row.root_thread_id == *root_thread_id).then_some(index))
                .collect::<Vec<_>>();
            let mut by_number = HashMap::new();
            let mut by_thread = HashMap::new();
            for index in &root_row_indices {
                let row = &reached_rows[*index];
                by_number.entry(row.number).or_insert(*index);
                by_thread.entry(row.thread_id.clone()).or_insert(*index);
            }
            for index in root_row_indices {
                let row = &reached_rows[index];
                if !row.is_retained_status() {
                    continue;
                }
                let mut node = row.node.clone();
                let (parent_task_number, parent_thread_id, parent_node_id) =
                    if let Some(parent_index) = task_forest_nearest_retained_parent_index(
                        &reached_rows,
                        index,
                        &by_number,
                        &by_thread,
                    ) {
                        let parent = &reached_rows[parent_index];
                        (
                            Some(parent.number),
                            Some(parent.thread_id.clone()),
                            Some(task_node_id(&parent.thread_id)),
                        )
                    } else {
                        (
                            None,
                            Some(root_thread_id.clone()),
                            Some(thread_root_node_id(root_thread_id)),
                        )
                    };
                if let TaskForestNode::Task {
                    parent_node_id: task_parent_node_id,
                    parent_task_number: task_parent_task_number,
                    parent_thread_id: task_parent_thread_id,
                    ..
                } = &mut node
                {
                    *task_parent_node_id = parent_node_id;
                    *task_parent_task_number = parent_task_number;
                    *task_parent_thread_id = parent_thread_id;
                }
                tasks.push(node);
            }
        }
        let total = tasks.len();
        Ok(TaskForestPage {
            tasks,
            total,
            root_thread_ids,
            skipped_pinned_thread_ids,
            active_count: None,
        })
    }

    pub fn task_subtree_summaries(&self, root_thread_id: &str) -> GaryxDbResult<Vec<TaskSummary>> {
        let root_thread_id = normalize_thread_id(root_thread_id)?;
        self.task_recursive_summaries(
            "WITH RECURSIVE task_tree(depth, thread_id, path) AS (
                SELECT 0, task.thread_id, ',' || task.thread_id || ','
                FROM task_projection task
                WHERE task.thread_id = ?1
                  AND task.projection_version = ?2
                UNION ALL
                SELECT task_tree.depth + 1, child.thread_id,
                       task_tree.path || child.thread_id || ','
                FROM task_tree
                JOIN task_projection parent ON parent.thread_id = task_tree.thread_id
                JOIN task_projection child
                  ON child.source_task_thread_id = task_tree.thread_id
                  OR child.source_task_id = ('#TASK-' || parent.number) COLLATE NOCASE
                WHERE child.projection_version = ?2
                  AND task_tree.depth < 64
                  AND instr(task_tree.path, ',' || child.thread_id || ',') = 0
             )
             SELECT task.thread_id, task.number, task.status, task.title,
                    task.creator_json, task.assignee_json, task.source_json,
                    task.executor_json, task.updated_at, task.updated_by_json,
                    COALESCE(meta.agent_id, '') AS runtime_agent_id,
                    COALESCE(meta.message_count, 0) AS reply_count
             FROM task_tree
             JOIN task_projection task ON task.thread_id = task_tree.thread_id
             LEFT JOIN thread_meta meta ON meta.thread_id = task.thread_id
             ORDER BY task_tree.depth ASC, task.number ASC, task.thread_id ASC",
            &root_thread_id,
        )
    }

    pub fn task_ancestor_summaries(&self, leaf_thread_id: &str) -> GaryxDbResult<Vec<TaskSummary>> {
        let leaf_thread_id = normalize_thread_id(leaf_thread_id)?;
        self.task_recursive_summaries(
            "WITH RECURSIVE ancestors(depth, thread_id, path) AS (
                SELECT 0, task.thread_id, ',' || task.thread_id || ','
                FROM task_projection task
                WHERE task.thread_id = ?1
                  AND task.projection_version = ?2
                UNION ALL
                SELECT ancestors.depth + 1, parent.thread_id,
                       ancestors.path || parent.thread_id || ','
                FROM ancestors
                JOIN task_projection child ON child.thread_id = ancestors.thread_id
                JOIN task_projection parent
                  ON parent.thread_id = child.source_task_thread_id
                  OR parent.number = child.parent_task_number
                  OR child.source_task_id = ('#TASK-' || parent.number) COLLATE NOCASE
                WHERE parent.projection_version = ?2
                  AND ancestors.depth < 64
                  AND instr(ancestors.path, ',' || parent.thread_id || ',') = 0
             )
             SELECT task.thread_id, task.number, task.status, task.title,
                    task.creator_json, task.assignee_json, task.source_json,
                    task.executor_json, task.updated_at, task.updated_by_json,
                    COALESCE(meta.agent_id, '') AS runtime_agent_id,
                    COALESCE(meta.message_count, 0) AS reply_count
             FROM ancestors
             JOIN task_projection task ON task.thread_id = ancestors.thread_id
             LEFT JOIN thread_meta meta ON meta.thread_id = task.thread_id
             ORDER BY ancestors.depth DESC, task.number ASC, task.thread_id ASC",
            &leaf_thread_id,
        )
    }

    fn task_recursive_summaries(
        &self,
        sql: &str,
        thread_id: &str,
    ) -> GaryxDbResult<Vec<TaskSummary>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(
            params![thread_id, CURRENT_TASK_PROJECTION_VERSION],
            task_summary_from_row,
        )?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }
}

pub(super) fn upsert_task_projection(
    tx: &Transaction<'_>,
    draft: &TaskProjectionDraft,
    projected_at: &str,
) -> GaryxDbResult<()> {
    let number = i64::try_from(draft.number).unwrap_or(i64::MAX);
    let parent_task_number = draft
        .parent_task_number
        .map(|number| i64::try_from(number).unwrap_or(i64::MAX));
    let source_events_len = i64::try_from(draft.source_events_len).unwrap_or(i64::MAX);
    tx.execute(
        "INSERT INTO task_projection (
            thread_id, number, status, title, creator_json, creator_id,
            assignee_json, assignee_id, updated_by_json, executor_json,
            source_json, source_thread_id, source_task_thread_id, source_task_id,
            parent_task_number, source_bot_id, notification_thread_id,
            created_at, updated_at, source_updated_at, source_events_len,
            projection_version, projected_at
         )
         VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
         )
         ON CONFLICT(thread_id) DO UPDATE SET
            number = excluded.number,
            status = excluded.status,
            title = excluded.title,
            creator_json = excluded.creator_json,
            creator_id = excluded.creator_id,
            assignee_json = excluded.assignee_json,
            assignee_id = excluded.assignee_id,
            updated_by_json = excluded.updated_by_json,
            executor_json = excluded.executor_json,
            source_json = excluded.source_json,
            source_thread_id = excluded.source_thread_id,
            source_task_thread_id = excluded.source_task_thread_id,
            source_task_id = excluded.source_task_id,
            parent_task_number = excluded.parent_task_number,
            source_bot_id = excluded.source_bot_id,
            notification_thread_id = excluded.notification_thread_id,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            source_updated_at = excluded.source_updated_at,
            source_events_len = excluded.source_events_len,
            projection_version = excluded.projection_version,
            projected_at = excluded.projected_at
         WHERE task_projection.projection_version <> excluded.projection_version
            OR (excluded.source_events_len, excluded.source_updated_at)
             > (task_projection.source_events_len, task_projection.source_updated_at)",
        params![
            draft.thread_id,
            number,
            draft.status,
            draft.title,
            draft.creator_json,
            draft.creator_id,
            draft.assignee_json,
            draft.assignee_id,
            draft.updated_by_json,
            draft.executor_json,
            draft.source_json,
            draft.source_thread_id,
            draft.source_task_thread_id,
            draft.source_task_id,
            parent_task_number,
            draft.source_bot_id,
            draft.notification_thread_id,
            draft.created_at,
            draft.updated_at,
            draft.source_updated_at,
            source_events_len,
            CURRENT_TASK_PROJECTION_VERSION,
            projected_at,
        ],
    )?;
    Ok(())
}

fn task_projection_filter_sql(filter: &TaskListFilter) -> GaryxDbResult<(String, Vec<SqlValue>)> {
    let mut clauses = vec!["task.projection_version = ?".to_owned()];
    let mut bind_values = vec![SqlValue::Integer(CURRENT_TASK_PROJECTION_VERSION)];

    if !filter.include_done {
        clauses.push("task.status <> 'done'".to_owned());
    }
    if let Some(status) = filter.status {
        clauses.push("task.status = ?".to_owned());
        bind_values.push(SqlValue::Text(status.as_str().to_owned()));
    }
    if let Some(assignee) = &filter.assignee {
        clauses.push("task.assignee_json = ?".to_owned());
        bind_values.push(SqlValue::Text(canonical_task_json("assignee", assignee)?));
    }
    if let Some(creator) = &filter.creator {
        clauses.push("task.creator_json = ?".to_owned());
        bind_values.push(SqlValue::Text(canonical_task_json("creator", creator)?));
    }
    if let Some(source_thread_id) = normalize_optional(filter.source_thread_id.as_deref()) {
        clauses.push("(task.source_thread_id = ? OR task.source_task_thread_id = ?)".to_owned());
        bind_values.push(SqlValue::Text(source_thread_id.clone()));
        bind_values.push(SqlValue::Text(source_thread_id));
    }
    if let Some(source_task_id) = normalize_optional(filter.source_task_id.as_deref()) {
        clauses.push("task.source_task_id = ? COLLATE NOCASE".to_owned());
        bind_values.push(SqlValue::Text(source_task_id));
    }
    if let Some(source_bot_id) = normalize_optional(filter.source_bot_id.as_deref()) {
        clauses.push("task.source_bot_id = ?".to_owned());
        bind_values.push(SqlValue::Text(source_bot_id));
    }

    Ok((clauses.join(" AND "), bind_values))
}

fn canonical_task_json<T: Serialize>(field: &str, value: &T) -> GaryxDbResult<String> {
    serde_json::to_string(value).map_err(|error| {
        GaryxDbError::BadRequest(format!("failed to serialize task {field} filter: {error}"))
    })
}

fn task_summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskSummary> {
    let thread_id = row.get::<_, String>(0)?;
    let number = row.get::<_, i64>(1)?.max(0) as u64;
    let status = task_status_from_row(2, row.get::<_, String>(2)?)?;
    let title = row.get::<_, String>(3)?;
    let creator = json_from_row::<Principal>(4, row.get::<_, String>(4)?)?;
    let assignee = optional_json_from_row::<Principal>(5, row.get::<_, Option<String>>(5)?)?;
    let source = optional_json_from_row::<TaskSource>(6, row.get::<_, Option<String>>(6)?)?;
    let executor = optional_json_from_row::<TaskExecutor>(7, row.get::<_, Option<String>>(7)?)?;
    let updated_at = timestamp_from_row(8, row.get::<_, String>(8)?)?;
    let updated_by = json_from_row::<Principal>(9, row.get::<_, String>(9)?)?;
    let runtime_agent_id = row.get::<_, String>(10)?;
    let reply_count = row.get::<_, i64>(11)?.clamp(0, i64::from(u32::MAX)) as u32;
    Ok(TaskSummary {
        thread_id,
        task_id: format!("#TASK-{number}"),
        number,
        title,
        status,
        creator,
        assignee,
        source,
        executor,
        updated_at,
        updated_by,
        runtime_agent_id,
        reply_count,
    })
}

fn task_number_from_task_id(task_id: &str) -> Option<u64> {
    let trimmed = task_id.trim();
    let candidate = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let (prefix, number) = candidate.split_once('-')?;
    if !prefix.eq_ignore_ascii_case("TASK") {
        return None;
    }
    number.parse::<u64>().ok().filter(|number| *number > 0)
}

fn task_forest_immediate_parent_index(
    row: &PinnedTaskForestRow,
    by_number: &HashMap<u64, usize>,
    by_thread: &HashMap<String, usize>,
) -> Option<usize> {
    row.parent_task_number
        .or(row.source_task_number)
        .and_then(|number| by_number.get(&number).copied())
        .or_else(|| {
            row.source_task_thread_id
                .as_ref()
                .and_then(|thread_id| by_thread.get(thread_id).copied())
        })
}

fn task_forest_nearest_retained_parent_index(
    rows: &[PinnedTaskForestRow],
    current_index: usize,
    by_number: &HashMap<u64, usize>,
    by_thread: &HashMap<String, usize>,
) -> Option<usize> {
    let mut seen = BTreeSet::new();
    let mut parent_index =
        task_forest_immediate_parent_index(&rows[current_index], by_number, by_thread);
    while let Some(index) = parent_index {
        if index == current_index || !seen.insert(index) {
            break;
        }
        let parent = &rows[index];
        if parent.is_retained_status() {
            return Some(index);
        }
        parent_index = task_forest_immediate_parent_index(parent, by_number, by_thread);
    }
    None
}

#[derive(Debug, Clone)]
struct AnchoredTaskRoot {
    thread_id: String,
    source_thread_id: Option<String>,
}

fn empty_anchored_task_forest_page() -> TaskForestPage {
    TaskForestPage {
        tasks: Vec::new(),
        total: 0,
        root_thread_ids: Vec::new(),
        skipped_pinned_thread_ids: Vec::new(),
        active_count: Some(0),
    }
}

/// Shared CTE prefix for anchored forest queries: the deduped task projection
/// (one row per task number, newest wins) plus resolved parent edges.
///
/// Performance shape (#TASK-1956): the desktop task-tree panel polls the
/// anchored forest every 5s, so this prefix must stay cheap on stores with
/// thousands of tasks. Three structural choices keep it O(N log N):
/// `deduped`/`parent_edges` are MATERIALIZED (the recursive walk re-joins
/// them per step, and inlining re-ran the whole projection scan each step);
/// parent resolution uses equi-joins instead of a correlated OR subquery
/// (`deduped` is unique per task number, so no LIMIT 1 ranking is needed);
/// and the `'#TASK-' || number` fallback match is anchored on an indexable
/// `CAST(substr(...))` key with the original string equality re-checked, so
/// the join never degenerates into an N^2 stringify-and-compare scan.
/// Presentation joins (thread_meta/recent_threads) live in the final
/// per-node SELECT, not here: they only decorate emitted rows.
fn anchored_forest_cte_prefix(where_sql: &str) -> String {
    format!(
        "WITH RECURSIVE filtered AS (
            SELECT task.thread_id, task.number, task.status, task.title,
                   task.creator_json, task.assignee_json, task.source_json,
                   task.executor_json, task.updated_at, task.updated_by_json,
                   task.parent_task_number,
                   task.source_thread_id,
                   task.source_task_thread_id,
                   task.source_task_id,
                   ROW_NUMBER() OVER (
                       PARTITION BY task.number
                       ORDER BY task.updated_at DESC, task.thread_id ASC
                   ) AS rn
            FROM task_projection task
            WHERE {where_sql}
         ),
         deduped AS MATERIALIZED (
            SELECT * FROM filtered WHERE rn = 1
         ),
         parent_edges AS MATERIALIZED (
            SELECT child.thread_id,
                   COALESCE(
                       p1.thread_id,
                       p2.thread_id,
                       child.source_task_thread_id
                   ) AS parent_thread_id
            FROM deduped child
            LEFT JOIN deduped p1
              ON child.parent_task_number IS NOT NULL
             AND p1.number = child.parent_task_number
            LEFT JOIN deduped p2
              ON child.parent_task_number IS NULL
             AND p2.number = CAST(substr(child.source_task_id, 7) AS INTEGER)
             AND child.source_task_id = ('#TASK-' || p2.number) COLLATE NOCASE
         )"
    )
}

const ANCHORED_FOREST_NODE_COLUMNS: &str =
    "SELECT deduped.thread_id, deduped.number, deduped.status, deduped.title,
                deduped.creator_json, deduped.assignee_json,
                deduped.source_json, deduped.executor_json,
                deduped.updated_at, deduped.updated_by_json,
                COALESCE(meta.agent_id, '') AS runtime_agent_id,
                COALESCE(meta.message_count, 0) AS reply_count,
                deduped.parent_task_number,
                edge.parent_thread_id,
                recent.active_run_id,
                COALESCE(recent.run_state, 'idle') AS run_state,
                recent.last_active_at,
                deduped.source_task_id,
                deduped.source_task_thread_id";

/// Climb from a task anchor to its topmost task and report that root's
/// origin conversation (`source_thread_id`), if any.
fn anchored_task_root_from_conn(
    conn: &Connection,
    where_sql: &str,
    mut bind_values: Vec<SqlValue>,
    anchor_thread_id: &str,
) -> GaryxDbResult<Option<AnchoredTaskRoot>> {
    let sql = format!(
        "{prefix},
         up(thread_id, number, depth, path) AS (
            SELECT d.thread_id, d.number, 0, ',' || d.thread_id || ','
            FROM deduped d
            WHERE d.thread_id = ?
            UNION ALL
            SELECT parent.thread_id, parent.number, up.depth + 1,
                   up.path || parent.thread_id || ','
            FROM up
            JOIN parent_edges edge ON edge.thread_id = up.thread_id
            JOIN deduped parent ON parent.thread_id = edge.parent_thread_id
            WHERE up.depth < 64
              AND instr(up.path, ',' || parent.thread_id || ',') = 0
         ),
         root AS (
            SELECT thread_id
            FROM up
            ORDER BY depth DESC, thread_id ASC
            LIMIT 1
         )
         SELECT deduped.thread_id, deduped.source_thread_id
         FROM root
         JOIN deduped ON deduped.thread_id = root.thread_id",
        prefix = anchored_forest_cte_prefix(where_sql)
    );
    bind_values.push(SqlValue::Text(anchor_thread_id.to_owned()));
    let mut stmt = conn.prepare(&sql)?;
    let root = stmt
        .query_row(params_from_iter(bind_values.iter()), |row| {
            Ok(AnchoredTaskRoot {
                thread_id: row.get(0)?,
                source_thread_id: row.get(1)?,
            })
        })
        .optional()?;
    Ok(root)
}

/// Origin-rooted forest shared by conversation anchors and task anchors whose
/// root task records its source conversation: every task seeded from the
/// origin plus all descendants, laid out in DFS order behind one hydrated
/// `kind:"thread"` root node.
fn origin_rooted_task_forest_from_conn(
    conn: &Connection,
    where_sql: &str,
    bind_values: Vec<SqlValue>,
    origin_thread_id: &str,
) -> GaryxDbResult<TaskForestPage> {
    let raw = origin_seeded_task_rows_from_conn(conn, where_sql, bind_values, origin_thread_id)?;
    let layout = layout_anchored_task_tree(raw, Some(origin_thread_id));
    if layout.nodes.is_empty() {
        return Ok(empty_anchored_task_forest_page());
    }
    let mut thread_root = task_forest_thread_root_from_conn(conn, origin_thread_id)?;
    if let TaskForestNode::Thread { depth, .. } = &mut thread_root {
        *depth = Some(0);
    }
    let mut tasks = layout.nodes;
    tasks.insert(0, thread_root);
    Ok(TaskForestPage {
        total: tasks.len(),
        tasks,
        root_thread_ids: vec![origin_thread_id.to_owned()],
        skipped_pinned_thread_ids: Vec::new(),
        active_count: Some(layout.active_count),
    })
}

fn origin_seeded_task_rows_from_conn(
    conn: &Connection,
    where_sql: &str,
    mut bind_values: Vec<SqlValue>,
    origin_thread_id: &str,
) -> GaryxDbResult<Vec<RawTaskNode>> {
    let sql = format!(
        "{prefix},
         seeds AS (
            SELECT thread_id, number
            FROM deduped
            WHERE source_thread_id = ?
              AND parent_task_number IS NULL
              AND source_task_id IS NULL
         ),
         down(thread_id, number, depth, path) AS (
            SELECT s.thread_id, s.number, 0, ',' || s.thread_id || ','
            FROM seeds s
            UNION ALL
            SELECT child.thread_id, child.number, down.depth + 1,
                   down.path || child.thread_id || ','
            FROM down
            JOIN parent_edges edge ON edge.parent_thread_id = down.thread_id
            JOIN deduped child ON child.thread_id = edge.thread_id
            WHERE down.depth < 64
              AND instr(down.path, ',' || child.thread_id || ',') = 0
         )
         {columns}
         FROM down
         JOIN deduped ON deduped.thread_id = down.thread_id
         LEFT JOIN parent_edges edge ON edge.thread_id = deduped.thread_id
         LEFT JOIN thread_meta meta ON meta.thread_id = deduped.thread_id
         LEFT JOIN recent_threads recent ON recent.thread_id = deduped.thread_id
         ORDER BY down.depth ASC, deduped.number ASC, deduped.thread_id ASC",
        prefix = anchored_forest_cte_prefix(where_sql),
        columns = ANCHORED_FOREST_NODE_COLUMNS
    );
    bind_values.push(SqlValue::Text(origin_thread_id.to_owned()));
    collect_anchored_raw_task_nodes(conn, &sql, bind_values)
}

/// Legacy origin-less task anchor: expand the whole tree under the climbed
/// topmost task.
fn climbed_task_tree_rows_from_conn(
    conn: &Connection,
    where_sql: &str,
    mut bind_values: Vec<SqlValue>,
    anchor_thread_id: &str,
) -> GaryxDbResult<Vec<RawTaskNode>> {
    let sql = format!(
        "{prefix},
         up(thread_id, number, depth, path) AS (
            SELECT d.thread_id, d.number, 0, ',' || d.thread_id || ','
            FROM deduped d
            WHERE d.thread_id = ?
            UNION ALL
            SELECT parent.thread_id, parent.number, up.depth + 1,
                   up.path || parent.thread_id || ','
            FROM up
            JOIN parent_edges edge ON edge.thread_id = up.thread_id
            JOIN deduped parent ON parent.thread_id = edge.parent_thread_id
            WHERE up.depth < 64
              AND instr(up.path, ',' || parent.thread_id || ',') = 0
         ),
         root AS (
            SELECT thread_id
            FROM up
            ORDER BY depth DESC, thread_id ASC
            LIMIT 1
         ),
         down(thread_id, number, depth, path) AS (
            SELECT root.thread_id, d.number, 0, ',' || root.thread_id || ','
            FROM root
            JOIN deduped d ON d.thread_id = root.thread_id
            UNION ALL
            SELECT child.thread_id, child.number, down.depth + 1,
                   down.path || child.thread_id || ','
            FROM down
            JOIN parent_edges edge ON edge.parent_thread_id = down.thread_id
            JOIN deduped child ON child.thread_id = edge.thread_id
            WHERE down.depth < 64
              AND instr(down.path, ',' || child.thread_id || ',') = 0
         )
         {columns}
         FROM down
         JOIN deduped ON deduped.thread_id = down.thread_id
         LEFT JOIN parent_edges edge ON edge.thread_id = deduped.thread_id
         LEFT JOIN thread_meta meta ON meta.thread_id = deduped.thread_id
         LEFT JOIN recent_threads recent ON recent.thread_id = deduped.thread_id
         ORDER BY down.depth ASC, deduped.number ASC, deduped.thread_id ASC",
        prefix = anchored_forest_cte_prefix(where_sql),
        columns = ANCHORED_FOREST_NODE_COLUMNS
    );
    bind_values.push(SqlValue::Text(anchor_thread_id.to_owned()));
    collect_anchored_raw_task_nodes(conn, &sql, bind_values)
}

fn collect_anchored_raw_task_nodes(
    conn: &Connection,
    sql: &str,
    bind_values: Vec<SqlValue>,
) -> GaryxDbResult<Vec<RawTaskNode>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        params_from_iter(bind_values.iter()),
        anchored_raw_task_node_from_row,
    )?;
    let mut raw = Vec::new();
    for row in rows {
        raw.push(row?);
    }
    Ok(raw)
}

fn anchored_raw_task_node_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawTaskNode> {
    let source_task_id = row.get::<_, Option<String>>(17)?;
    let source_task_number = source_task_id.as_deref().and_then(task_number_from_task_id);
    let source_task_thread_id = row.get::<_, Option<String>>(18)?;
    let node = task_forest_task_from_row(row, None)?;
    let TaskForestNode::Task {
        task,
        parent_task_number,
        ..
    } = &node
    else {
        unreachable!("anchored forest SQL rows are task nodes");
    };
    Ok(RawTaskNode {
        thread_id: task.thread_id.clone(),
        number: task.number,
        status: task.status,
        parent_task_number: *parent_task_number,
        source_task_number,
        source_task_thread_id,
        node,
    })
}

fn task_forest_thread_root_from_conn(
    conn: &Connection,
    thread_id: &str,
) -> rusqlite::Result<TaskForestNode> {
    let row = conn
        .query_row(
            "SELECT base.thread_id,
                    COALESCE(NULLIF(recent.title, ''), NULLIF(meta.thread_label, ''), base.thread_id) AS title,
                    COALESCE(NULLIF(recent.thread_type, ''), NULLIF(meta.thread_type, ''), 'chat') AS thread_type,
                    COALESCE(recent.provider_type, meta.provider_type) AS provider_type,
                    COALESCE(recent.agent_id, meta.agent_id) AS agent_id,
                    COALESCE(recent.message_count, meta.message_count, 0) AS message_count,
                    COALESCE(
                        NULLIF(recent.last_message_preview, ''),
                        meta.last_message_preview,
                        meta.last_assistant_message,
                        meta.last_user_message,
                        ''
                    ) AS last_message_preview,
                    COALESCE(recent.active_run_id, meta.active_run_id) AS active_run_id,
                    COALESCE(
                        recent.run_state,
                        CASE WHEN meta.active_run_id IS NULL THEN 'idle' ELSE 'running' END
                    ) AS run_state,
                    COALESCE(recent.updated_at, meta.updated_at) AS updated_at,
                    COALESCE(recent.last_active_at, meta.updated_at, meta.projected_at) AS last_active_at
             FROM (SELECT ?1 AS thread_id) base
             LEFT JOIN recent_threads recent ON recent.thread_id = base.thread_id
             LEFT JOIN thread_meta meta ON meta.thread_id = base.thread_id
             WHERE base.thread_id = ?1",
            params![thread_id],
            |row| {
                Ok(TaskForestNode::Thread {
                    node_id: thread_root_node_id(&row.get::<_, String>(0)?),
                    thread_id: row.get(0)?,
                    title: row.get(1)?,
                    thread_type: row.get(2)?,
                    provider_type: row.get(3)?,
                    agent_id: row.get(4)?,
                    message_count: row
                        .get::<_, i64>(5)?
                        .clamp(0, i64::from(u32::MAX)) as u32,
                    last_message_preview: row.get(6)?,
                    active_run_id: row.get(7)?,
                    run_state: row.get(8)?,
                    updated_at: row.get(9)?,
                    last_active_at: row.get(10)?,
                    depth: None,
                })
            },
        )
        .optional()?;
    Ok(row.unwrap_or_else(|| TaskForestNode::Thread {
        node_id: thread_root_node_id(thread_id),
        thread_id: thread_id.to_owned(),
        title: thread_id.to_owned(),
        thread_type: "chat".to_owned(),
        provider_type: None,
        agent_id: None,
        message_count: 0,
        last_message_preview: String::new(),
        active_run_id: None,
        run_state: "idle".to_owned(),
        updated_at: None,
        last_active_at: None,
        depth: None,
    }))
}

fn task_forest_task_from_row(
    row: &rusqlite::Row<'_>,
    root_parent_thread_id: Option<&str>,
) -> rusqlite::Result<TaskForestNode> {
    let task = task_summary_from_row(row)?;
    let parent_task_number = row
        .get::<_, Option<i64>>(12)?
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0);
    let parent_thread_id = row.get::<_, Option<String>>(13)?;
    let parent_node_id = root_parent_thread_id
        .map(thread_root_node_id)
        .or_else(|| parent_thread_id.as_deref().map(task_node_id));
    Ok(TaskForestNode::Task {
        node_id: task_node_id(&task.thread_id),
        parent_node_id,
        task,
        parent_task_number,
        parent_thread_id,
        active_run_id: row.get(14)?,
        run_state: row.get(15)?,
        last_active_at: row.get(16)?,
        depth: None,
    })
}

fn task_status_from_row(index: usize, value: String) -> rusqlite::Result<TaskStatus> {
    match value.as_str() {
        "todo" => Ok(TaskStatus::Todo),
        "in_progress" => Ok(TaskStatus::InProgress),
        "in_review" => Ok(TaskStatus::InReview),
        "done" => Ok(TaskStatus::Done),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Text,
            format!("unknown task status: {value}").into(),
        )),
    }
}

fn json_from_row<T: DeserializeOwned>(index: usize, value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

fn optional_json_from_row<T: DeserializeOwned>(
    index: usize,
    value: Option<String>,
) -> rusqlite::Result<Option<T>> {
    value.map(|value| json_from_row(index, value)).transpose()
}

fn timestamp_from_row(index: usize, value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
        })
}

#[cfg(test)]
mod tests;
