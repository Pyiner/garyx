use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use garyx_models::{Principal, TaskExecutor, TaskSource, TaskStatus};
use garyx_router::tasks::{TaskListFilter, TaskSummary};
use rusqlite::types::{Type, Value as SqlValue};
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::task_tree::{RawTaskNode, prune_anchored_task_tree};

use super::{
    GaryxDbError, GaryxDbResult, GaryxDbService, normalize_optional, normalize_thread_id,
    now_string,
};

pub const CURRENT_TASK_PROJECTION_VERSION: i64 = 1;
pub const TASK_PROJECTION_NAME: &str = "task_projection";

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
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskForestPage {
    pub tasks: Vec<TaskForestNode>,
    pub total: usize,
    pub root_thread_ids: Vec<String>,
    pub skipped_pinned_thread_ids: Vec<String>,
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

pub(crate) struct TaskProjectionBackfillActivity<'a> {
    db: &'a GaryxDbService,
}

impl Drop for TaskProjectionBackfillActivity<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.db.task_projection_backfill_active.lock() {
            *active = false;
        }
    }
}
impl GaryxDbService {
    pub(crate) async fn lock_task_projection_backfill(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.task_projection_backfill_lock.lock().await
    }

    pub(crate) fn mark_task_projection_backfill_active(
        &self,
    ) -> GaryxDbResult<TaskProjectionBackfillActivity<'_>> {
        let mut active = self
            .task_projection_backfill_active
            .lock()
            .map_err(|_| GaryxDbError::LockPoisoned)?;
        *active = true;
        Ok(TaskProjectionBackfillActivity { db: self })
    }

    pub fn count_task_projection(&self) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM task_projection", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn task_projection_needs_backfill(&self) -> GaryxDbResult<bool> {
        let conn = self.conn()?;
        let state = conn
            .query_row(
                "SELECT projection_version, source_row_count
                 FROM projection_states
                 WHERE projection_name = ?1",
                params![TASK_PROJECTION_NAME],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((version, source_row_count)) = state else {
            return Ok(true);
        };
        if version != CURRENT_TASK_PROJECTION_VERSION {
            return Ok(true);
        }
        let current_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM task_projection", [], |row| row.get(0))?;
        Ok(source_row_count > 0 && current_count == 0)
    }

    pub fn task_projection_is_current(&self) -> GaryxDbResult<bool> {
        Ok(!self.task_projection_needs_backfill()?)
    }

    pub fn replace_task_projection(&self, draft: TaskProjectionDraft) -> GaryxDbResult<()> {
        let thread_id = normalize_thread_id(&draft.thread_id)?;
        let mut draft = draft;
        draft.thread_id = thread_id;
        let projected_at = now_string();
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
        let backfill_active = *self
            .task_projection_backfill_active
            .lock()
            .map_err(|_| GaryxDbError::LockPoisoned)?;
        if backfill_active {
            let mut tombstones = self
                .task_projection_tombstones
                .lock()
                .map_err(|_| GaryxDbError::LockPoisoned)?;
            tombstones.insert(thread_id.clone());
        }
        Ok(removed > 0)
    }

    pub fn sync_task_projection_snapshot(
        &self,
        drafts: Vec<TaskProjectionDraft>,
    ) -> GaryxDbResult<()> {
        let projected_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        for mut draft in drafts {
            draft.thread_id = normalize_thread_id(&draft.thread_id)?;
            upsert_task_projection(&tx, &draft, &projected_at)?;
        }
        tx.execute(
            "DELETE FROM task_projection WHERE projection_version <> ?1",
            params![CURRENT_TASK_PROJECTION_VERSION],
        )?;
        let tombstones = {
            let mut tombstones = self
                .task_projection_tombstones
                .lock()
                .map_err(|_| GaryxDbError::LockPoisoned)?;
            let values = tombstones.iter().cloned().collect::<Vec<_>>();
            tombstones.clear();
            values
        };
        for thread_id in tombstones {
            tx.execute(
                "DELETE FROM task_projection WHERE thread_id = ?1",
                params![thread_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn task_index_rows(&self) -> GaryxDbResult<Vec<(u64, String)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "WITH ranked AS (
                SELECT number, thread_id,
                       ROW_NUMBER() OVER (
                           PARTITION BY number
                           ORDER BY updated_at DESC, thread_id ASC
                       ) AS rn
                FROM task_projection
                WHERE projection_version = ?1
             )
             SELECT number, thread_id
             FROM ranked
             WHERE rn = 1
             ORDER BY number ASC",
        )?;
        let rows = stmt.query_map(params![CURRENT_TASK_PROJECTION_VERSION], |row| {
            let number = row.get::<_, i64>(0)?;
            Ok((number.max(0) as u64, row.get::<_, String>(1)?))
        })?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row?);
        }
        Ok(values)
    }

    pub fn thread_id_for_number(&self, number: u64) -> GaryxDbResult<Option<String>> {
        let number = i64::try_from(number).unwrap_or(i64::MAX);
        let conn = self.conn()?;
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
        let conn = self.conn()?;
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

    pub fn max_task_projection_number(&self) -> GaryxDbResult<Option<u64>> {
        let conn = self.conn()?;
        let value: Option<i64> = conn.query_row(
            "SELECT MAX(number)
             FROM task_projection
             WHERE projection_version = ?1",
            params![CURRENT_TASK_PROJECTION_VERSION],
            |row| row.get(0),
        )?;
        Ok(value.map(|number| number.max(0) as u64))
    }

    pub fn list_task_projection_thread_ids(&self) -> GaryxDbResult<Vec<String>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT thread_id FROM task_projection ORDER BY thread_id ASC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut thread_ids = Vec::new();
        for row in rows {
            thread_ids.push(row?);
        }
        Ok(thread_ids)
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

        let conn = self.conn()?;
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

        let conn = self.conn()?;
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
        let raw_filter = TaskListFilter {
            include_done: true,
            ..TaskListFilter::default()
        };
        let (where_sql, bind_values) = task_projection_filter_sql(&raw_filter)?;
        let list_sql = format!(
            "WITH RECURSIVE filtered AS (
                SELECT task.thread_id, task.number, task.status, task.title,
                       task.creator_json, task.assignee_json, task.source_json,
                       task.executor_json, task.updated_at, task.updated_by_json,
                       COALESCE(meta.agent_id, '') AS runtime_agent_id,
                       COALESCE(meta.message_count, 0) AS reply_count,
                       task.parent_task_number,
                       task.source_task_thread_id,
                       task.source_task_id,
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
             ),
             deduped AS (
                SELECT * FROM filtered WHERE rn = 1
             ),
             parent_edges AS (
                SELECT child.thread_id,
                       COALESCE(
                           (
                               SELECT parent.thread_id
                               FROM deduped parent
                               WHERE (
                                     child.parent_task_number IS NOT NULL
                                 AND parent.number = child.parent_task_number
                               )
                                  OR (
                                     child.parent_task_number IS NULL
                                 AND child.source_task_id = ('#TASK-' || parent.number) COLLATE NOCASE
                                  )
                               ORDER BY parent.updated_at DESC, parent.thread_id ASC
                               LIMIT 1
                           ),
                           child.source_task_thread_id
                       ) AS parent_thread_id
                FROM deduped child
             ),
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
             SELECT deduped.thread_id, deduped.number, deduped.status, deduped.title,
                    deduped.creator_json, deduped.assignee_json,
                    deduped.source_json, deduped.executor_json,
                    deduped.updated_at, deduped.updated_by_json,
                    deduped.runtime_agent_id, deduped.reply_count,
                    deduped.parent_task_number,
                    edge.parent_thread_id,
                    deduped.active_run_id,
                    COALESCE(deduped.run_state, 'idle') AS run_state,
                    deduped.last_active_at,
                    deduped.source_task_id,
                    deduped.source_task_thread_id,
                    (SELECT thread_id FROM root) AS root_thread_id
             FROM down
             JOIN deduped ON deduped.thread_id = down.thread_id
             LEFT JOIN parent_edges edge ON edge.thread_id = deduped.thread_id
             ORDER BY down.depth ASC, deduped.number ASC, deduped.thread_id ASC"
        );

        let conn = self.conn()?;
        let mut list_bind_values = bind_values;
        list_bind_values.push(SqlValue::Text(anchor_thread_id.clone()));
        let mut stmt = conn.prepare(&list_sql)?;
        let rows = stmt.query_map(params_from_iter(list_bind_values.iter()), |row| {
            let source_task_id = row.get::<_, Option<String>>(17)?;
            let source_task_number = source_task_id.as_deref().and_then(task_number_from_task_id);
            let source_task_thread_id = row.get::<_, Option<String>>(18)?;
            let root_thread_id = row.get::<_, String>(19)?;
            let node = task_forest_task_from_row(row, None)?;
            let TaskForestNode::Task {
                task,
                parent_task_number,
                ..
            } = &node
            else {
                unreachable!("anchored task tree SQL rows are task nodes");
            };
            Ok((
                root_thread_id,
                RawTaskNode {
                    thread_id: task.thread_id.clone(),
                    number: task.number,
                    status: task.status,
                    parent_task_number: *parent_task_number,
                    source_task_number,
                    source_task_thread_id,
                    node,
                },
            ))
        })?;

        let mut raw = Vec::new();
        let mut root_thread_ids = Vec::new();
        for row in rows {
            let (root_thread_id, raw_node) = row?;
            if !root_thread_ids.contains(&root_thread_id) {
                root_thread_ids.push(root_thread_id);
            }
            raw.push(raw_node);
        }

        let tasks = prune_anchored_task_tree(raw, &anchor_thread_id);
        Ok(TaskForestPage {
            total: tasks.len(),
            tasks,
            root_thread_ids,
            skipped_pinned_thread_ids: Vec::new(),
        })
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
                       ROW_NUMBER() OVER (ORDER BY pinned_at DESC, thread_id ASC) AS root_rank
                FROM thread_pins"
            .to_owned();
        let skipped_sql = "WITH pinned AS (
                SELECT thread_id, pinned_at
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
             ORDER BY pinned.pinned_at DESC, pinned.thread_id ASC";
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

        let conn = self.conn()?;
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
                            Some(task_forest_task_node_id(&parent.thread_id)),
                        )
                    } else {
                        (
                            None,
                            Some(root_thread_id.clone()),
                            Some(task_forest_thread_root_node_id(root_thread_id)),
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
        let conn = self.conn()?;
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

fn upsert_task_projection(
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

fn task_forest_thread_root_node_id(thread_id: &str) -> String {
    format!("thread-root:{thread_id}")
}

fn task_forest_task_node_id(thread_id: &str) -> String {
    format!("task:{thread_id}")
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

fn task_forest_thread_root_from_conn(
    conn: &Connection,
    thread_id: &str,
) -> rusqlite::Result<TaskForestNode> {
    let row = conn
        .query_row(
            "SELECT pinned.thread_id,
                    COALESCE(NULLIF(recent.title, ''), NULLIF(meta.thread_label, ''), pinned.thread_id) AS title,
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
             FROM thread_pins pinned
             LEFT JOIN recent_threads recent ON recent.thread_id = pinned.thread_id
             LEFT JOIN thread_meta meta ON meta.thread_id = pinned.thread_id
             WHERE pinned.thread_id = ?1",
            params![thread_id],
            |row| {
                Ok(TaskForestNode::Thread {
                    node_id: task_forest_thread_root_node_id(&row.get::<_, String>(0)?),
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
                })
            },
        )
        .optional()?;
    Ok(row.unwrap_or_else(|| TaskForestNode::Thread {
        node_id: task_forest_thread_root_node_id(thread_id),
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
        .map(task_forest_thread_root_node_id)
        .or_else(|| parent_thread_id.as_deref().map(task_forest_task_node_id));
    Ok(TaskForestNode::Task {
        node_id: task_forest_task_node_id(&task.thread_id),
        parent_node_id,
        task,
        parent_task_number,
        parent_thread_id,
        active_run_id: row.get(14)?,
        run_state: row.get(15)?,
        last_active_at: row.get(16)?,
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
mod tests {
    use super::super::RecentThreadDraft;
    use super::*;

    fn task_projection_draft(
        thread_id: &str,
        number: u64,
        status: TaskStatus,
        updated_at: &str,
        source: Option<TaskSource>,
        source_events_len: usize,
    ) -> TaskProjectionDraft {
        let creator = Principal::Agent {
            agent_id: "test-agent".to_owned(),
        };
        let assignee = Principal::Human {
            user_id: "1000000001".to_owned(),
        };
        let updated_by = creator.clone();
        let parent_task_number = source
            .as_ref()
            .and_then(|source| source.task_id.as_deref())
            .and_then(|task_id| task_id.strip_prefix("#TASK-"))
            .and_then(|number| number.parse::<u64>().ok());
        let source_bot_id = source
            .as_ref()
            .and_then(|source| source.bot_id.clone())
            .or_else(|| {
                source.as_ref().and_then(|source| {
                    Some(format!(
                        "{}:{}",
                        source.channel.as_ref()?,
                        source.account_id.as_ref()?
                    ))
                })
            });
        TaskProjectionDraft {
            thread_id: thread_id.to_owned(),
            number,
            status: status.as_str().to_owned(),
            title: format!("Task {number}"),
            creator_json: serde_json::to_string(&creator).expect("creator json"),
            creator_id: creator.id().to_owned(),
            assignee_json: Some(serde_json::to_string(&assignee).expect("assignee json")),
            assignee_id: Some(assignee.id().to_owned()),
            updated_by_json: serde_json::to_string(&updated_by).expect("updated_by json"),
            executor_json: None,
            source_json: source
                .as_ref()
                .map(|source| serde_json::to_string(source).expect("source json")),
            source_thread_id: source.as_ref().and_then(|source| source.thread_id.clone()),
            source_task_thread_id: source
                .as_ref()
                .and_then(|source| source.task_thread_id.clone()),
            source_task_id: source.as_ref().and_then(|source| source.task_id.clone()),
            parent_task_number,
            source_bot_id,
            notification_thread_id: None,
            created_at: "2026-01-01T00:00:00.000Z".to_owned(),
            updated_at: updated_at.to_owned(),
            source_updated_at: updated_at.to_owned(),
            source_events_len,
        }
    }

    fn thread_source(thread_id: &str, task_id: &str) -> TaskSource {
        TaskSource {
            thread_id: Some(thread_id.to_owned()),
            task_id: Some(task_id.to_owned()),
            task_thread_id: Some(thread_id.to_owned()),
            bot_id: None,
            channel: None,
            account_id: None,
        }
    }

    fn chat_source(thread_id: &str) -> TaskSource {
        TaskSource {
            thread_id: Some(thread_id.to_owned()),
            task_id: None,
            task_thread_id: None,
            bot_id: None,
            channel: None,
            account_id: None,
        }
    }

    fn bot_thread_source(thread_id: &str, task_id: &str, bot_id: &str) -> TaskSource {
        TaskSource {
            thread_id: Some(thread_id.to_owned()),
            task_id: Some(task_id.to_owned()),
            task_thread_id: Some(thread_id.to_owned()),
            bot_id: Some(bot_id.to_owned()),
            channel: None,
            account_id: None,
        }
    }

    fn with_creator(mut draft: TaskProjectionDraft, creator: &Principal) -> TaskProjectionDraft {
        draft.creator_json = serde_json::to_string(creator).expect("creator json");
        draft.creator_id = creator.id().to_owned();
        draft
    }

    fn with_assignee(mut draft: TaskProjectionDraft, assignee: &Principal) -> TaskProjectionDraft {
        draft.assignee_json = Some(serde_json::to_string(assignee).expect("assignee json"));
        draft.assignee_id = Some(assignee.id().to_owned());
        draft
    }

    #[test]
    fn task_projection_zero_task_state_does_not_repeat_backfill() {
        let db = GaryxDbService::memory().expect("db opens");
        assert!(
            db.task_projection_needs_backfill()
                .expect("new projection needs initial backfill")
        );

        db.sync_task_projection_snapshot(Vec::new())
            .expect("sync empty snapshot");
        db.record_projection_state(TASK_PROJECTION_NAME, CURRENT_TASK_PROJECTION_VERSION, 0)
            .expect("record empty projection state");
        assert!(
            !db.task_projection_needs_backfill()
                .expect("zero-task state is current")
        );

        db.replace_task_projection(task_projection_draft(
            "thread::task-1",
            1,
            TaskStatus::Todo,
            "2026-01-01T00:00:01.000Z",
            None,
            1,
        ))
        .expect("insert task projection");
        db.record_projection_state(TASK_PROJECTION_NAME, CURRENT_TASK_PROJECTION_VERSION, 1)
            .expect("record non-empty state");
        db.remove_task_projection("thread::task-1")
            .expect("remove projection");
        assert!(
            db.task_projection_needs_backfill()
                .expect("unexpected empty after non-empty state needs backfill")
        );
    }

    #[test]
    fn task_projection_snapshot_cannot_overwrite_newer_revision() {
        let db = GaryxDbService::memory().expect("db opens");
        let stale = task_projection_draft(
            "thread::task-1",
            1,
            TaskStatus::InProgress,
            "2026-01-01T00:00:01.000Z",
            None,
            1,
        );
        let fresh = task_projection_draft(
            "thread::task-1",
            1,
            TaskStatus::Done,
            "2026-01-01T00:00:02.000Z",
            None,
            2,
        );

        db.replace_task_projection(fresh)
            .expect("insert fresh realtime row");
        db.sync_task_projection_snapshot(vec![stale])
            .expect("sync stale snapshot row");

        let (tasks, total, _) = db
            .list_task_summaries(&TaskListFilter {
                include_done: true,
                ..Default::default()
            })
            .expect("list task projection");
        assert_eq!(total, 1);
        assert_eq!(tasks[0].status, TaskStatus::Done);
    }

    #[test]
    fn task_projection_snapshot_applies_delete_tombstones() {
        let db = GaryxDbService::memory().expect("db opens");
        let _active = db
            .mark_task_projection_backfill_active()
            .expect("mark active backfill");
        db.remove_task_projection("thread::deleted")
            .expect("record delete tombstone");
        db.sync_task_projection_snapshot(vec![task_projection_draft(
            "thread::deleted",
            7,
            TaskStatus::InProgress,
            "2026-01-01T00:00:01.000Z",
            None,
            1,
        )])
        .expect("sync snapshot with deleted row");
        assert_eq!(
            db.thread_id_for_number(7).expect("lookup deleted row"),
            None
        );
    }

    #[test]
    fn task_projection_list_filters_and_dedupes_duplicate_numbers() {
        let db = GaryxDbService::memory().expect("db opens");
        let source = TaskSource {
            thread_id: Some("thread::origin".to_owned()),
            task_id: Some("#TASK-1".to_owned()),
            task_thread_id: Some("thread::parent".to_owned()),
            bot_id: None,
            channel: Some("api".to_owned()),
            account_id: Some("main".to_owned()),
        };
        db.replace_task_projection(task_projection_draft(
            "thread::older",
            42,
            TaskStatus::InProgress,
            "2026-01-01T00:00:01.000Z",
            Some(source.clone()),
            1,
        ))
        .expect("insert older duplicate");
        db.replace_task_projection(task_projection_draft(
            "thread::newer",
            42,
            TaskStatus::InReview,
            "2026-01-01T00:00:02.000Z",
            Some(source),
            2,
        ))
        .expect("insert newer duplicate");
        db.replace_task_projection(task_projection_draft(
            "thread::done",
            43,
            TaskStatus::Done,
            "2026-01-01T00:00:03.000Z",
            None,
            1,
        ))
        .expect("insert done row");

        let (tasks, total, has_more) = db
            .list_task_summaries(&TaskListFilter {
                source_thread_id: Some("thread::parent".to_owned()),
                source_task_id: Some("#task-1".to_owned()),
                source_bot_id: Some("api:main".to_owned()),
                include_done: false,
                limit: Some(10),
                offset: Some(0),
                ..Default::default()
            })
            .expect("list filtered task projection");

        assert_eq!(total, 1);
        assert!(!has_more);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].thread_id, "thread::newer");
        assert_eq!(tasks[0].number, 42);
        assert_eq!(tasks[0].status, TaskStatus::InReview);
    }

    #[test]
    fn task_projection_recursive_ctes_use_thread_identity_and_guard_cycles() {
        let db = GaryxDbService::memory().expect("db opens");
        db.replace_task_projection(task_projection_draft(
            "thread::parent",
            1,
            TaskStatus::InProgress,
            "2026-01-01T00:00:01.000Z",
            None,
            1,
        ))
        .expect("insert parent");
        db.replace_task_projection(task_projection_draft(
            "thread::child",
            2,
            TaskStatus::InProgress,
            "2026-01-01T00:00:02.000Z",
            Some(thread_source("thread::parent", "#TASK-1")),
            1,
        ))
        .expect("insert child");
        db.replace_task_projection(task_projection_draft(
            "thread::grandchild",
            3,
            TaskStatus::Todo,
            "2026-01-01T00:00:03.000Z",
            Some(thread_source("thread::child", "#TASK-2")),
            1,
        ))
        .expect("insert grandchild");
        db.replace_task_projection(task_projection_draft(
            "thread::cycle",
            4,
            TaskStatus::Todo,
            "2026-01-01T00:00:04.000Z",
            Some(thread_source("thread::cycle", "#TASK-4")),
            1,
        ))
        .expect("insert self cycle");

        let subtree = db
            .task_subtree_summaries("thread::parent")
            .expect("subtree");
        assert_eq!(
            subtree
                .iter()
                .map(|task| task.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::parent", "thread::child", "thread::grandchild"]
        );

        let ancestors = db
            .task_ancestor_summaries("thread::grandchild")
            .expect("ancestors");
        assert_eq!(
            ancestors
                .iter()
                .map(|task| task.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::parent", "thread::child", "thread::grandchild"]
        );

        let cycle = db.task_subtree_summaries("thread::cycle").expect("cycle");
        assert_eq!(cycle.len(), 1);
    }

    #[test]
    fn task_forest_includes_parent_and_run_state_fields() {
        let db = GaryxDbService::memory().expect("db opens");
        db.replace_task_projection(task_projection_draft(
            "thread::parent",
            1,
            TaskStatus::InProgress,
            "2026-01-01T00:00:01.000Z",
            None,
            1,
        ))
        .expect("insert parent");
        db.replace_task_projection(task_projection_draft(
            "thread::child",
            2,
            TaskStatus::Todo,
            "2026-01-01T00:00:02.000Z",
            Some(thread_source("thread::parent", "#TASK-1")),
            1,
        ))
        .expect("insert child");
        db.replace_task_projection(task_projection_draft(
            "thread::legacy-child",
            3,
            TaskStatus::Todo,
            "2026-01-01T00:00:03.000Z",
            Some(TaskSource {
                thread_id: Some("thread::origin".to_owned()),
                task_id: Some("#TASK-1".to_owned()),
                task_thread_id: None,
                bot_id: None,
                channel: None,
                account_id: None,
            }),
            1,
        ))
        .expect("insert legacy child");
        db.upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::child".to_owned(),
            title: "Child".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: Some("claude_code".to_owned()),
            agent_id: Some("claude".to_owned()),
            message_count: 4,
            last_message_preview: "Working".to_owned(),
            recent_run_id: Some("run::recent".to_owned()),
            active_run_id: Some("run::active".to_owned()),
            run_state: "running".to_owned(),
            updated_at: Some("2026-01-01T00:00:03.000Z".to_owned()),
            last_active_at: "2026-01-01T00:00:04.000Z".to_owned(),
        })
        .expect("insert recent thread");

        let page = db
            .list_task_forest(
                &TaskListFilter {
                    include_done: true,
                    ..Default::default()
                },
                TaskForestScope::All,
            )
            .expect("list forest");

        assert_eq!(page.total, 3);
        assert!(page.root_thread_ids.is_empty());
        assert!(page.skipped_pinned_thread_ids.is_empty());
        let child = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::child")
            .expect("child node");
        assert_eq!(child.parent_task_number(), Some(1));
        assert_eq!(child.parent_thread_id(), Some("thread::parent"));
        assert_eq!(child.active_run_id(), Some("run::active"));
        assert_eq!(child.run_state(), "running");
        assert_eq!(child.last_active_at(), Some("2026-01-01T00:00:04.000Z"));
        let legacy_child = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::legacy-child")
            .expect("legacy child node");
        assert_eq!(legacy_child.parent_task_number(), Some(1));
        assert_eq!(legacy_child.parent_thread_id(), Some("thread::parent"));
        let parent = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::parent")
            .expect("parent node");
        assert_eq!(parent.parent_task_number(), None);
        assert_eq!(parent.run_state(), "idle");
    }

    #[test]
    fn pinned_task_forest_returns_pinned_roots_and_descendants() {
        let db = GaryxDbService::memory().expect("db opens");
        db.upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::chat-a".to_owned(),
            title: "Chat A".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: Some("codex".to_owned()),
            agent_id: Some("codex".to_owned()),
            message_count: 7,
            last_message_preview: "Coordinate A".to_owned(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-01-01T00:00:01.500Z".to_owned()),
            last_active_at: "2026-01-01T00:00:01.500Z".to_owned(),
        })
        .expect("insert chat a");
        db.upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::chat-b".to_owned(),
            title: "Chat B".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: Some("claude_code".to_owned()),
            agent_id: Some("claude".to_owned()),
            message_count: 3,
            last_message_preview: "Coordinate B".to_owned(),
            recent_run_id: None,
            active_run_id: Some("run::chat-b".to_owned()),
            run_state: "running".to_owned(),
            updated_at: Some("2026-01-01T00:00:03.500Z".to_owned()),
            last_active_at: "2026-01-01T00:00:03.500Z".to_owned(),
        })
        .expect("insert chat b");
        db.replace_task_projection(task_projection_draft(
            "thread::child-a",
            11,
            TaskStatus::InProgress,
            "2026-01-01T00:00:02.000Z",
            Some(chat_source("thread::chat-a")),
            1,
        ))
        .expect("insert child a");
        db.replace_task_projection(task_projection_draft(
            "thread::grandchild-a",
            12,
            TaskStatus::InReview,
            "2026-01-01T00:00:03.000Z",
            Some(thread_source("thread::child-a", "#TASK-11")),
            1,
        ))
        .expect("insert grandchild a");
        db.replace_task_projection(task_projection_draft(
            "thread::child-b",
            21,
            TaskStatus::InProgress,
            "2026-01-01T00:00:05.000Z",
            Some(chat_source("thread::chat-b")),
            1,
        ))
        .expect("insert child b");
        db.replace_task_projection(task_projection_draft(
            "thread::unrelated",
            99,
            TaskStatus::Todo,
            "2026-01-01T00:00:06.000Z",
            None,
            1,
        ))
        .expect("insert unrelated");
        db.conn()
            .expect("db connection")
            .execute_batch(
                "INSERT INTO thread_pins (thread_id, pinned_at)
                 VALUES
                   ('thread::chat-a', '2026-01-01T00:00:01.000Z'),
                   ('thread::chat', '2026-01-01T00:00:02.000Z'),
                   ('thread::chat-b', '2026-01-01T00:00:03.000Z')",
            )
            .expect("insert pins");

        let page = db
            .list_task_forest(
                &TaskListFilter {
                    include_done: true,
                    ..Default::default()
                },
                TaskForestScope::Pinned,
            )
            .expect("list pinned forest");

        assert_eq!(
            page.tasks
                .iter()
                .map(|node| node.thread_id())
                .collect::<Vec<_>>(),
            vec![
                "thread::chat-b",
                "thread::child-b",
                "thread::chat-a",
                "thread::child-a",
                "thread::grandchild-a"
            ]
        );
        assert_eq!(page.total, 5);
        assert_eq!(
            page.root_thread_ids,
            vec!["thread::chat-b".to_owned(), "thread::chat-a".to_owned()]
        );
        assert_eq!(page.skipped_pinned_thread_ids, vec!["thread::chat"]);
        let root_b = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::chat-b")
            .expect("chat b root");
        match root_b {
            TaskForestNode::Thread {
                title,
                active_run_id,
                ..
            } => {
                assert_eq!(title, "Chat B");
                assert_eq!(active_run_id.as_deref(), Some("run::chat-b"));
            }
            TaskForestNode::Task { .. } => panic!("chat root should be a thread node"),
        }
        let child_a = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::child-a")
            .expect("child a");
        assert_eq!(child_a.parent_thread_id(), Some("thread::chat-a"));
        assert_eq!(child_a.parent_node_id(), Some("thread-root:thread::chat-a"));
    }

    #[test]
    fn pinned_task_forest_filters_inactive_tasks_and_reparents_active_descendants() {
        let db = GaryxDbService::memory().expect("db opens");
        db.replace_task_projection(task_projection_draft(
            "thread::done-parent",
            31,
            TaskStatus::Done,
            "2026-01-01T00:00:01.000Z",
            Some(chat_source("thread::chat-active")),
            1,
        ))
        .expect("insert done parent");
        db.replace_task_projection(task_projection_draft(
            "thread::todo-middle",
            32,
            TaskStatus::Todo,
            "2026-01-01T00:00:02.000Z",
            Some(thread_source("thread::done-parent", "#TASK-31")),
            1,
        ))
        .expect("insert todo middle");
        db.replace_task_projection(task_projection_draft(
            "thread::active-leaf",
            33,
            TaskStatus::InProgress,
            "2026-01-01T00:00:03.000Z",
            Some(thread_source("thread::todo-middle", "#TASK-32")),
            1,
        ))
        .expect("insert active leaf");
        db.replace_task_projection(task_projection_draft(
            "thread::review-child",
            34,
            TaskStatus::InReview,
            "2026-01-01T00:00:04.000Z",
            Some(thread_source("thread::active-leaf", "#TASK-33")),
            1,
        ))
        .expect("insert review child");
        db.replace_task_projection(task_projection_draft(
            "thread::done-child",
            35,
            TaskStatus::Done,
            "2026-01-01T00:00:05.000Z",
            Some(thread_source("thread::active-leaf", "#TASK-33")),
            1,
        ))
        .expect("insert done child");
        db.replace_task_projection(task_projection_draft(
            "thread::inactive-only",
            41,
            TaskStatus::Done,
            "2026-01-01T00:00:06.000Z",
            Some(chat_source("thread::chat-inactive")),
            1,
        ))
        .expect("insert inactive-only child");
        db.conn()
            .expect("db connection")
            .execute_batch(
                "INSERT INTO thread_pins (thread_id, pinned_at)
                 VALUES
                   ('thread::chat-active', '2026-01-01T00:00:01.000Z'),
                   ('thread::chat-inactive', '2026-01-01T00:00:02.000Z')",
            )
            .expect("insert pins");

        let page = db
            .list_task_forest(
                &TaskListFilter {
                    include_done: true,
                    ..Default::default()
                },
                TaskForestScope::Pinned,
            )
            .expect("list pinned forest");

        assert_eq!(
            page.tasks
                .iter()
                .map(|node| node.thread_id())
                .collect::<Vec<_>>(),
            vec![
                "thread::chat-inactive",
                "thread::chat-active",
                "thread::active-leaf",
                "thread::review-child"
            ]
        );
        assert_eq!(
            page.root_thread_ids,
            vec![
                "thread::chat-inactive".to_owned(),
                "thread::chat-active".to_owned()
            ]
        );
        assert!(page.skipped_pinned_thread_ids.is_empty());
        for node in &page.tasks {
            if let TaskForestNode::Task { task, .. } = node {
                assert!(
                    matches!(task.status, TaskStatus::InProgress | TaskStatus::InReview),
                    "inactive task leaked into pinned forest: {:?}",
                    task.status
                );
            }
        }
        let active_leaf = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::active-leaf")
            .expect("active leaf");
        assert_eq!(active_leaf.parent_task_number(), None);
        assert_eq!(active_leaf.parent_thread_id(), Some("thread::chat-active"));
        assert_eq!(
            active_leaf.parent_node_id(),
            Some("thread-root:thread::chat-active")
        );
        let review_child = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::review-child")
            .expect("review child");
        assert_eq!(review_child.parent_task_number(), Some(33));
        assert_eq!(review_child.parent_thread_id(), Some("thread::active-leaf"));
        assert_eq!(
            review_child.parent_node_id(),
            Some("task:thread::active-leaf")
        );
    }

    #[test]
    fn anchored_task_forest_climbs_to_root_and_keeps_done_ancestors() {
        let db = GaryxDbService::memory().expect("db opens");
        db.replace_task_projection(task_projection_draft(
            "thread::1254",
            1254,
            TaskStatus::InProgress,
            "2026-01-01T00:00:01.000Z",
            None,
            1,
        ))
        .expect("insert root");
        db.replace_task_projection(task_projection_draft(
            "thread::1261",
            1261,
            TaskStatus::InReview,
            "2026-01-01T00:00:02.000Z",
            Some(thread_source("thread::1254", "#TASK-1254")),
            1,
        ))
        .expect("insert active child");
        db.replace_task_projection(task_projection_draft(
            "thread::1262",
            1262,
            TaskStatus::Done,
            "2026-01-01T00:00:03.000Z",
            Some(thread_source("thread::1254", "#TASK-1254")),
            1,
        ))
        .expect("insert done structural child");
        db.replace_task_projection(task_projection_draft(
            "thread::1263",
            1263,
            TaskStatus::Done,
            "2026-01-01T00:00:04.000Z",
            Some(thread_source("thread::1254", "#TASK-1254")),
            1,
        ))
        .expect("insert done branch");
        db.replace_task_projection(task_projection_draft(
            "thread::1270",
            1270,
            TaskStatus::InProgress,
            "2026-01-01T00:00:05.000Z",
            Some(thread_source("thread::1262", "#TASK-1262")),
            1,
        ))
        .expect("insert active grandchild");
        db.replace_task_projection(task_projection_draft(
            "thread::1271",
            1271,
            TaskStatus::Done,
            "2026-01-01T00:00:06.000Z",
            Some(thread_source("thread::1263", "#TASK-1263")),
            1,
        ))
        .expect("insert done dead leaf");

        let page = db
            .list_task_forest_anchored("thread::1270", &TaskListFilter::default())
            .expect("anchored forest");

        assert_eq!(page.root_thread_ids, vec!["thread::1254"]);
        assert_eq!(page.skipped_pinned_thread_ids, Vec::<String>::new());
        assert_eq!(page.total, 4);
        assert_eq!(
            page.tasks
                .iter()
                .map(|node| node.thread_id())
                .collect::<Vec<_>>(),
            vec![
                "thread::1254",
                "thread::1261",
                "thread::1262",
                "thread::1270"
            ]
        );
        assert!(
            page.tasks
                .iter()
                .all(|node| matches!(node, TaskForestNode::Task { .. })),
            "anchored tree must not synthesize thread roots"
        );
        let active_grandchild = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::1270")
            .expect("active grandchild");
        assert_eq!(active_grandchild.parent_task_number(), Some(1262));
        assert_eq!(active_grandchild.parent_thread_id(), Some("thread::1262"));
        assert_eq!(
            active_grandchild.parent_node_id(),
            Some("task:thread::1262")
        );
    }

    #[test]
    fn anchored_task_forest_plan_b_keeps_current_dead_branch_path() {
        let db = GaryxDbService::memory().expect("db opens");
        for (thread_id, number, status, source) in [
            ("thread::1254", 1254, TaskStatus::InProgress, None),
            (
                "thread::1261",
                1261,
                TaskStatus::InReview,
                Some(thread_source("thread::1254", "#TASK-1254")),
            ),
            (
                "thread::1262",
                1262,
                TaskStatus::Done,
                Some(thread_source("thread::1254", "#TASK-1254")),
            ),
            (
                "thread::1263",
                1263,
                TaskStatus::Done,
                Some(thread_source("thread::1254", "#TASK-1254")),
            ),
            (
                "thread::1270",
                1270,
                TaskStatus::InProgress,
                Some(thread_source("thread::1262", "#TASK-1262")),
            ),
            (
                "thread::1271",
                1271,
                TaskStatus::Done,
                Some(thread_source("thread::1263", "#TASK-1263")),
            ),
        ] {
            db.replace_task_projection(task_projection_draft(
                thread_id,
                number,
                status,
                &format!("2026-01-01T00:00:{:02}.000Z", number - 1250),
                source,
                1,
            ))
            .expect("insert task");
        }

        let page = db
            .list_task_forest_anchored("thread::1271", &TaskListFilter::default())
            .expect("anchored forest");

        assert_eq!(page.total, 6);
        assert_eq!(
            page.tasks
                .iter()
                .map(|node| node.thread_id())
                .collect::<Vec<_>>(),
            vec![
                "thread::1254",
                "thread::1261",
                "thread::1262",
                "thread::1263",
                "thread::1270",
                "thread::1271"
            ]
        );
        let current_dead_leaf = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::1271")
            .expect("current dead leaf");
        assert_eq!(current_dead_leaf.parent_thread_id(), Some("thread::1263"));
        assert_eq!(
            page.tasks
                .iter()
                .filter(|node| matches!(
                    node,
                    TaskForestNode::Task { task, .. }
                        if matches!(task.status, TaskStatus::InProgress | TaskStatus::InReview)
                ))
                .count(),
            3
        );
    }

    #[test]
    fn anchored_task_forest_hides_all_done_and_bare_threads() {
        let db = GaryxDbService::memory().expect("db opens");
        db.replace_task_projection(task_projection_draft(
            "thread::done-root",
            200,
            TaskStatus::Done,
            "2026-01-01T00:00:01.000Z",
            None,
            1,
        ))
        .expect("insert done root");
        db.replace_task_projection(task_projection_draft(
            "thread::done-child",
            201,
            TaskStatus::Done,
            "2026-01-01T00:00:02.000Z",
            Some(thread_source("thread::done-root", "#TASK-200")),
            1,
        ))
        .expect("insert done child");

        let all_done = db
            .list_task_forest_anchored("thread::done-child", &TaskListFilter::default())
            .expect("all done anchored forest");
        assert!(all_done.tasks.is_empty());
        assert_eq!(all_done.total, 0);
        assert_eq!(all_done.root_thread_ids, vec!["thread::done-root"]);

        let bare = db
            .list_task_forest_anchored("thread::bare", &TaskListFilter::default())
            .expect("bare anchored forest");
        assert!(bare.tasks.is_empty());
        assert!(bare.root_thread_ids.is_empty());
    }

    #[test]
    fn anchored_task_forest_ignores_caller_filters_for_raw_tree() {
        let db = GaryxDbService::memory().expect("db opens");
        let target_creator = Principal::Agent {
            agent_id: "target-creator".to_owned(),
        };
        let target_assignee = Principal::Human {
            user_id: "1000000002".to_owned(),
        };

        db.replace_task_projection(task_projection_draft(
            "thread::filter-root",
            300,
            TaskStatus::Done,
            "2026-01-01T00:00:01.000Z",
            None,
            1,
        ))
        .expect("insert root");
        db.replace_task_projection(task_projection_draft(
            "thread::filter-sibling",
            301,
            TaskStatus::InReview,
            "2026-01-01T00:00:02.000Z",
            Some(thread_source("thread::filter-root", "#TASK-300")),
            1,
        ))
        .expect("insert sibling");
        db.replace_task_projection(task_projection_draft(
            "thread::filter-child",
            302,
            TaskStatus::InProgress,
            "2026-01-01T00:00:03.000Z",
            Some(thread_source("thread::filter-root", "#TASK-300")),
            1,
        ))
        .expect("insert child");
        db.replace_task_projection(with_assignee(
            with_creator(
                task_projection_draft(
                    "thread::filter-leaf",
                    303,
                    TaskStatus::InProgress,
                    "2026-01-01T00:00:04.000Z",
                    Some(bot_thread_source(
                        "thread::filter-child",
                        "#TASK-302",
                        "api:target",
                    )),
                    1,
                ),
                &target_creator,
            ),
            &target_assignee,
        ))
        .expect("insert leaf");

        let page = db
            .list_task_forest_anchored(
                "thread::filter-leaf",
                &TaskListFilter {
                    status: Some(TaskStatus::Done),
                    assignee: Some(target_assignee),
                    creator: Some(target_creator),
                    source_thread_id: Some("thread::filter-child".to_owned()),
                    source_task_id: Some("#TASK-302".to_owned()),
                    source_bot_id: Some("api:target".to_owned()),
                    include_done: false,
                    limit: Some(1),
                    offset: Some(99),
                },
            )
            .expect("anchored forest");

        assert_eq!(page.root_thread_ids, vec!["thread::filter-root"]);
        assert_eq!(page.total, 4);
        assert_eq!(
            page.tasks
                .iter()
                .map(|node| node.thread_id())
                .collect::<Vec<_>>(),
            vec![
                "thread::filter-root",
                "thread::filter-sibling",
                "thread::filter-child",
                "thread::filter-leaf",
            ]
        );
        let leaf = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::filter-leaf")
            .expect("leaf");
        assert_eq!(leaf.parent_task_number(), Some(302));
        assert_eq!(leaf.parent_thread_id(), Some("thread::filter-child"));
    }

    #[test]
    fn pinned_task_forest_prefers_pinned_seed_over_newer_duplicate_number() {
        let db = GaryxDbService::memory().expect("db opens");
        db.replace_task_projection(task_projection_draft(
            "thread::pinned-direct",
            1,
            TaskStatus::InProgress,
            "2026-01-01T00:00:01.000Z",
            Some(chat_source("thread::pinned-chat")),
            1,
        ))
        .expect("insert pinned direct duplicate");
        db.replace_task_projection(task_projection_draft(
            "thread::newer-duplicate",
            1,
            TaskStatus::InReview,
            "2026-01-01T00:00:02.000Z",
            None,
            2,
        ))
        .expect("insert newer duplicate");
        db.replace_task_projection(task_projection_draft(
            "thread::child",
            2,
            TaskStatus::Todo,
            "2026-01-01T00:00:03.000Z",
            Some(thread_source("thread::pinned-direct", "#TASK-1")),
            1,
        ))
        .expect("insert child");
        db.pin_thread("thread::pinned-chat").expect("pin chat");

        let page = db
            .list_task_forest(
                &TaskListFilter {
                    include_done: true,
                    ..Default::default()
                },
                TaskForestScope::Pinned,
            )
            .expect("list pinned forest");

        assert_eq!(
            page.tasks
                .iter()
                .map(|node| node.thread_id())
                .collect::<Vec<_>>(),
            vec!["thread::pinned-chat", "thread::pinned-direct"]
        );
        assert_eq!(page.root_thread_ids, vec!["thread::pinned-chat"]);
        assert!(
            !page
                .tasks
                .iter()
                .any(|node| node.thread_id() == "thread::newer-duplicate")
        );
    }

    #[test]
    fn pinned_task_forest_skips_only_pins_without_any_projection() {
        let db = GaryxDbService::memory().expect("db opens");
        db.replace_task_projection(task_projection_draft(
            "thread::other-bot-direct",
            1,
            TaskStatus::InProgress,
            "2026-01-01T00:00:01.000Z",
            Some(TaskSource {
                thread_id: Some("thread::other-bot-chat".to_owned()),
                task_id: None,
                task_thread_id: None,
                bot_id: Some("api:other".to_owned()),
                channel: None,
                account_id: None,
            }),
            1,
        ))
        .expect("insert other bot root");
        db.replace_task_projection(task_projection_draft(
            "thread::main-child",
            2,
            TaskStatus::InProgress,
            "2026-01-01T00:00:02.000Z",
            Some(TaskSource {
                thread_id: Some("thread::other-bot-chat".to_owned()),
                task_id: Some("#TASK-1".to_owned()),
                task_thread_id: Some("thread::other-bot-direct".to_owned()),
                bot_id: Some("api:main".to_owned()),
                channel: None,
                account_id: None,
            }),
            1,
        ))
        .expect("insert child");
        db.pin_thread("thread::other-bot-chat")
            .expect("pin filtered chat");
        db.pin_thread("thread::chat").expect("pin chat");

        let page = db
            .list_task_forest(
                &TaskListFilter {
                    include_done: true,
                    source_bot_id: Some("api:main".to_owned()),
                    ..Default::default()
                },
                TaskForestScope::Pinned,
            )
            .expect("list filtered pinned forest");

        assert!(page.tasks.is_empty());
        assert!(page.root_thread_ids.is_empty());
        assert_eq!(page.skipped_pinned_thread_ids, vec!["thread::chat"]);
    }
}
