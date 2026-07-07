use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use chrono::{SecondsFormat, Utc};
use garyx_router::KnownChannelEndpoint;
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

mod task_forest;

pub use task_forest::{
    CURRENT_TASK_PROJECTION_VERSION, TASK_PROJECTION_NAME, TaskForestNode, TaskForestPage,
    TaskForestScope, TaskProjectionDraft,
};

const CURRENT_THREAD_META_PROJECTION_VERSION: i64 = 4;

#[derive(Debug, thiserror::Error)]
pub enum GaryxDbError {
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("database lock poisoned")]
    LockPoisoned,
    #[error("blocking database task failed: {0}")]
    Join(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub type GaryxDbResult<T> = Result<T, GaryxDbError>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinnedThreadRecord {
    pub thread_id: String,
    pub pinned_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecentThreadRecord {
    pub thread_id: String,
    pub title: String,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub provider_type: Option<String>,
    pub agent_id: Option<String>,
    pub message_count: u32,
    pub last_message_preview: String,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub run_state: String,
    pub updated_at: Option<String>,
    pub last_active_at: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentThreadDraft {
    pub thread_id: String,
    pub title: String,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub provider_type: Option<String>,
    pub agent_id: Option<String>,
    pub message_count: u32,
    pub last_message_preview: String,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub run_state: String,
    pub updated_at: Option<String>,
    pub last_active_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMessageRouteRecord {
    pub thread_id: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    pub thread_binding_key: Option<String>,
    pub message_id: String,
    pub projected_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMessageRouteDraft {
    pub thread_id: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    pub thread_binding_key: Option<String>,
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMetaRecord {
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub thread_label: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub provider_key: Option<String>,
    pub selected_model: Option<String>,
    pub selected_model_reasoning_effort: Option<String>,
    pub selected_model_service_tier: Option<String>,
    pub sdk_session_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub message_count: u32,
    pub last_user_message: Option<String>,
    pub last_assistant_message: Option<String>,
    pub last_message_preview: Option<String>,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub worktree_json: Option<String>,
    pub last_delivery_context_json: Option<String>,
    pub last_delivery_updated_at: Option<String>,
    pub default_list_hidden: bool,
    pub projection_version: i64,
    pub projected_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadMetaDraft {
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub thread_label: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub provider_key: Option<String>,
    pub selected_model: Option<String>,
    pub selected_model_reasoning_effort: Option<String>,
    pub selected_model_service_tier: Option<String>,
    pub sdk_session_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub message_count: u32,
    pub last_user_message: Option<String>,
    pub last_assistant_message: Option<String>,
    pub last_message_preview: Option<String>,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub worktree_json: Option<String>,
    pub last_delivery_context_json: Option<String>,
    pub last_delivery_updated_at: Option<String>,
    pub default_list_hidden: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadMetaProjectionSnapshot {
    pub thread_meta: Vec<ThreadMetaDraft>,
    pub channel_endpoints: Vec<KnownChannelEndpoint>,
    pub message_routes: Vec<ThreadMessageRouteDraft>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadMetaProjectionDraft {
    pub thread_id: String,
    pub thread_meta: ThreadMetaDraft,
    pub channel_endpoints: Vec<KnownChannelEndpoint>,
    pub message_routes: Vec<ThreadMessageRouteDraft>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AutomationThreadRunRecord {
    pub automation_id: String,
    pub run_id: String,
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub agent_id: Option<String>,
    pub automation_label_snapshot: Option<String>,
    pub mode: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationThreadRunDraft {
    pub automation_id: String,
    pub run_id: String,
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub agent_id: Option<String>,
    pub automation_label_snapshot: Option<String>,
    pub mode: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowRunRecord {
    pub workflow_id: String,
    pub task_id: Option<String>,
    pub task_thread_id: Option<String>,
    pub workflow_definition_id: Option<String>,
    pub workflow_definition_version: Option<u64>,
    pub workflow_definition_snapshot_json: Option<String>,
    pub input_json: Option<String>,
    pub parent_thread_id: String,
    pub parent_run_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub current_phase_index: Option<i64>,
    pub script_text: String,
    pub meta_json: String,
    pub result_json: Option<String>,
    pub output_text: Option<String>,
    pub error: Option<String>,
    pub workspace_dir: Option<String>,
    pub created_by: Option<String>,
    pub total_children: u32,
    pub completed_children: u32,
    pub failed_children: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tool_calls: u64,
    pub total_cost_usd: f64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRunDraft {
    pub workflow_id: Option<String>,
    pub task_id: Option<String>,
    pub task_thread_id: Option<String>,
    pub workflow_definition_id: Option<String>,
    pub workflow_definition_version: Option<u64>,
    pub workflow_definition_snapshot_json: Option<String>,
    pub input_json: Option<String>,
    pub parent_thread_id: String,
    pub parent_run_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub current_phase_index: Option<i64>,
    pub script_text: String,
    pub meta_json: String,
    pub result_json: Option<String>,
    pub output_text: Option<String>,
    pub error: Option<String>,
    pub workspace_dir: Option<String>,
    pub created_by: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptedWorkflowTaskReference {
    pub workflow_id: String,
    pub task_thread_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowChildRunRecord {
    pub workflow_id: String,
    pub workflow_child_run_id: String,
    pub thread_id: String,
    pub phase_index: i64,
    pub phase_title: String,
    pub label: String,
    pub agent_id: Option<String>,
    pub status: String,
    pub prompt: String,
    pub result_mode: String,
    pub schema_json: Option<String>,
    pub result_text: Option<String>,
    pub result_json: Option<String>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: u64,
    pub cost_usd: f64,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowChildRunDraft {
    pub workflow_id: String,
    pub workflow_child_run_id: Option<String>,
    pub thread_id: String,
    pub phase_index: i64,
    pub phase_title: String,
    pub label: String,
    pub agent_id: Option<String>,
    pub status: String,
    pub prompt: String,
    pub result_mode: String,
    pub schema_json: Option<String>,
    pub result_text: Option<String>,
    pub result_json: Option<String>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: u64,
    pub cost_usd: f64,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkflowChildRunUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorkflowEventRecord {
    pub event_seq: u64,
    pub event_id: String,
    pub workflow_id: String,
    pub workflow_child_run_id: Option<String>,
    pub thread_id: Option<String>,
    pub event_type: String,
    pub payload_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowEventDraft {
    pub event_id: Option<String>,
    pub workflow_id: String,
    pub workflow_child_run_id: Option<String>,
    pub thread_id: Option<String>,
    pub event_type: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRunDrilldownSnapshot {
    pub workflow: WorkflowRunRecord,
    pub children: Vec<WorkflowChildRunRecord>,
    pub events: Vec<WorkflowEventRecord>,
    pub latest_event_seq: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub name: Option<String>,
    pub path: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDraft {
    pub name: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CapsuleRecord {
    pub id: String,
    pub title: String,
    pub description: String,
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub html_sha256: String,
    pub byte_size: i64,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsuleCreateDraft {
    pub id: String,
    pub title: String,
    pub description: String,
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub html_sha256: String,
    pub byte_size: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapsuleUpdateDraft {
    pub title: Option<String>,
    pub description: Option<String>,
    pub html_sha256: Option<String>,
    pub byte_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DreamSpanRecord {
    pub span_id: String,
    pub dream_id: String,
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub start_seq: u64,
    pub end_seq: u64,
    pub start_at: String,
    pub end_at: String,
    pub excerpt: String,
    pub message_count: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DreamTopicRecord {
    pub dream_id: String,
    pub title: String,
    pub summary: String,
    pub first_message_at: String,
    pub last_message_at: String,
    pub updated_at: String,
    pub source: String,
    pub confidence: f64,
    pub message_count: u32,
    pub span_count: u32,
    pub spans: Vec<DreamSpanRecord>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DreamScanRunRecord {
    pub run_id: String,
    pub scanned_from: String,
    pub scanned_to: String,
    pub created_at: String,
    pub source: String,
    pub status: String,
    pub topics_count: u32,
    pub spans_count: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DreamTopicDraft {
    pub dream_id: String,
    pub title: String,
    pub summary: String,
    pub first_message_at: String,
    pub last_message_at: String,
    pub source: String,
    pub confidence: f64,
    pub message_count: u32,
    pub spans: Vec<DreamSpanDraft>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DreamSpanDraft {
    pub span_id: String,
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub start_seq: u64,
    pub end_seq: u64,
    pub start_at: String,
    pub end_at: String,
    pub excerpt: String,
    pub message_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DreamIdResolution {
    dream_id: String,
    duplicate_dream_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DreamOverlapCandidate {
    overlap_score: u64,
    overlap_count: u32,
    last_message_at: String,
    exact_span_keys: BTreeSet<(String, u64, u64)>,
    span_count: u32,
}

pub struct GaryxDbService {
    conn: Mutex<Connection>,
    task_projection_tombstones: Mutex<BTreeSet<String>>,
    task_projection_backfill_lock: tokio::sync::Mutex<()>,
    task_projection_backfill_active: Mutex<bool>,
}

const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(5_000);

/// Durability/concurrency settings for the on-disk database: WAL journal
/// (persistent, readers never block the single writer), NORMAL fsync
/// (sub-ms commits, still crash-safe under WAL), and a busy timeout so
/// cross-process contention retries instead of failing fast.
fn configure_file_connection(conn: &Connection) -> GaryxDbResult<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let journal_mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(GaryxDbError::BadRequest(format!(
            "failed to enable WAL journal mode: got {journal_mode}"
        )));
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

impl GaryxDbService {
    pub fn open(path: impl AsRef<Path>) -> GaryxDbResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        configure_file_connection(&conn)?;
        initialize_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            task_projection_tombstones: Mutex::new(BTreeSet::new()),
            task_projection_backfill_lock: tokio::sync::Mutex::new(()),
            task_projection_backfill_active: Mutex::new(false),
        })
    }

    pub fn memory() -> GaryxDbResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        initialize_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            task_projection_tombstones: Mutex::new(BTreeSet::new()),
            task_projection_backfill_lock: tokio::sync::Mutex::new(()),
            task_projection_backfill_active: Mutex::new(false),
        })
    }

    fn conn(&self) -> GaryxDbResult<MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| GaryxDbError::LockPoisoned)
    }

    /// Run `f` against this service on the blocking thread pool.
    ///
    /// New async call sites (the SQLite thread-store surgery, #TASK-1864)
    /// must use this entry point so database IO never occupies a runtime
    /// worker. Existing synchronous call sites migrate separately
    /// (#TASK-1829).
    pub async fn run_blocking<T, F>(self: &std::sync::Arc<Self>, f: F) -> GaryxDbResult<T>
    where
        F: FnOnce(&GaryxDbService) -> GaryxDbResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let db = std::sync::Arc::clone(self);
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|err| GaryxDbError::Join(err.to_string()))?
    }

    pub fn list_pinned_threads(&self) -> GaryxDbResult<Vec<PinnedThreadRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT thread_id, pinned_at FROM thread_pins ORDER BY pinned_at DESC, thread_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PinnedThreadRecord {
                thread_id: row.get(0)?,
                pinned_at: row.get(1)?,
            })
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn pin_thread(&self, thread_id: &str) -> GaryxDbResult<PinnedThreadRecord> {
        let thread_id = normalize_thread_id(thread_id)?;
        let pinned_at = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO thread_pins (thread_id, pinned_at)
             VALUES (?1, ?2)
             ON CONFLICT(thread_id) DO UPDATE SET pinned_at = excluded.pinned_at",
            params![thread_id, pinned_at],
        )?;
        Ok(PinnedThreadRecord {
            thread_id,
            pinned_at,
        })
    }

    pub fn unpin_thread(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.conn()?;
        let removed = conn.execute(
            "DELETE FROM thread_pins WHERE thread_id = ?1",
            params![thread_id],
        )?;
        Ok(removed > 0)
    }

    pub fn mark_thread_archived(&self, thread_id: &str) -> GaryxDbResult<String> {
        let thread_id = normalize_thread_id(thread_id)?;
        let archived_at = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO archived_threads (thread_id, archived_at)
             VALUES (?1, ?2)
             ON CONFLICT(thread_id) DO UPDATE SET archived_at = excluded.archived_at",
            params![thread_id, archived_at],
        )?;
        Ok(archived_at)
    }

    pub fn is_thread_archived(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.conn()?;
        let archived: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM archived_threads WHERE thread_id = ?1",
                params![thread_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(archived.is_some())
    }

    pub fn list_workspaces(&self) -> GaryxDbResult<Vec<WorkspaceRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT name, path, created_at, updated_at
             FROM workspaces
             WHERE deleted_at IS NULL
             ORDER BY lower(COALESCE(NULLIF(name, ''), path)) ASC, lower(path) ASC",
        )?;
        let rows = stmt.query_map([], workspace_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn count_workspace_rows(&self) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn upsert_workspace(&self, draft: WorkspaceDraft) -> GaryxDbResult<WorkspaceRecord> {
        let path = normalize_workspace_path(&draft.path)?;
        let name = normalize_optional(draft.name.as_deref());
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO workspaces (path, name, created_at, updated_at, deleted_at)
             VALUES (?1, ?2, ?3, ?3, NULL)
             ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                updated_at = excluded.updated_at,
                deleted_at = NULL",
            params![path, name, now],
        )?;
        workspace_by_path(&conn, &path)?
            .ok_or_else(|| GaryxDbError::BadRequest("workspace was not saved".to_owned()))
    }

    pub fn delete_workspace(&self, path: &str) -> GaryxDbResult<bool> {
        let path = normalize_workspace_path(path)?;
        let now = now_string();
        let conn = self.conn()?;
        let removed = conn.execute(
            "UPDATE workspaces
             SET updated_at = ?2, deleted_at = ?2
             WHERE path = ?1 AND deleted_at IS NULL",
            params![path, now],
        )?;
        if removed == 0 {
            conn.execute(
                "INSERT INTO workspaces (path, name, created_at, updated_at, deleted_at)
                 VALUES (?1, NULL, ?2, ?2, ?2)
                 ON CONFLICT(path) DO NOTHING",
                params![path, now],
            )?;
        }
        Ok(removed > 0)
    }

    pub fn seed_workspaces_if_empty(&self, drafts: Vec<WorkspaceDraft>) -> GaryxDbResult<bool> {
        let mut normalized = Vec::new();
        let mut seen = BTreeSet::new();
        for draft in drafts {
            let path = normalize_workspace_path(&draft.path)?;
            if !seen.insert(path.clone()) {
                continue;
            }
            normalized.push(WorkspaceDraft {
                name: normalize_optional(draft.name.as_deref()),
                path,
            });
        }
        if normalized.is_empty() {
            return Ok(false);
        }

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))?;
        if count > 0 {
            tx.commit()?;
            return Ok(false);
        }

        let now = now_string();
        for draft in normalized {
            tx.execute(
                "INSERT INTO workspaces (path, name, created_at, updated_at, deleted_at)
                 VALUES (?1, ?2, ?3, ?3, NULL)",
                params![draft.path, draft.name, now],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn create_capsule(&self, draft: CapsuleCreateDraft) -> GaryxDbResult<CapsuleRecord> {
        let id = normalize_capsule_id(&draft.id)?;
        let title = normalize_capsule_text(&draft.title);
        let description = normalize_capsule_text(&draft.description);
        let thread_id = normalize_optional(draft.thread_id.as_deref());
        let run_id = normalize_optional(draft.run_id.as_deref());
        let agent_id = normalize_optional(draft.agent_id.as_deref());
        let provider_type = normalize_optional(draft.provider_type.as_deref());
        let html_sha256 = normalize_capsule_sha256(&draft.html_sha256)?;
        let byte_size = normalize_capsule_byte_size(draft.byte_size)?;
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO capsules (
                id, title, description, thread_id, run_id, agent_id, provider_type,
                html_sha256, byte_size, revision, created_at, updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10)",
            params![
                id,
                title,
                description,
                thread_id,
                run_id,
                agent_id,
                provider_type,
                html_sha256,
                byte_size,
                now,
            ],
        )?;
        capsule_by_id(&conn, &id)?
            .ok_or_else(|| GaryxDbError::BadRequest("capsule was not saved".to_owned()))
    }

    pub fn update_capsule(
        &self,
        id: &str,
        draft: CapsuleUpdateDraft,
    ) -> GaryxDbResult<Option<CapsuleRecord>> {
        let id = normalize_capsule_id(id)?;
        let title = draft.title.as_deref().map(normalize_capsule_text);
        let description = draft.description.as_deref().map(normalize_capsule_text);
        let html_sha256 = draft
            .html_sha256
            .as_deref()
            .map(normalize_capsule_sha256)
            .transpose()?;
        let byte_size = draft
            .byte_size
            .map(normalize_capsule_byte_size)
            .transpose()?;
        let now = now_string();
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE capsules
             SET title = COALESCE(?2, title),
                 description = COALESCE(?3, description),
                 html_sha256 = COALESCE(?4, html_sha256),
                 byte_size = COALESCE(?5, byte_size),
                 revision = revision + 1,
                 updated_at = ?6
             WHERE id = ?1",
            params![id, title, description, html_sha256, byte_size, now],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        capsule_by_id(&conn, &id)
    }

    pub fn get_capsule(&self, id: &str) -> GaryxDbResult<Option<CapsuleRecord>> {
        let id = normalize_capsule_id(id)?;
        let conn = self.conn()?;
        capsule_by_id(&conn, &id)
    }

    pub fn list_capsules(&self) -> GaryxDbResult<Vec<CapsuleRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, description, thread_id, run_id, agent_id, provider_type,
                    html_sha256, byte_size, revision, created_at, updated_at
             FROM capsules
             ORDER BY updated_at DESC, id ASC",
        )?;
        let rows = stmt.query_map([], capsule_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn list_capsules_for_thread(&self, thread_id: &str) -> GaryxDbResult<Vec<CapsuleRecord>> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, description, thread_id, run_id, agent_id, provider_type,
                    html_sha256, byte_size, revision, created_at, updated_at
             FROM capsules
             WHERE thread_id = ?1
             ORDER BY updated_at DESC, id ASC",
        )?;
        let rows = stmt.query_map(params![thread_id], capsule_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn delete_capsule(&self, id: &str) -> GaryxDbResult<bool> {
        let id = normalize_capsule_id(id)?;
        let conn = self.conn()?;
        let removed = conn.execute("DELETE FROM capsules WHERE id = ?1", params![id])?;
        Ok(removed > 0)
    }

    pub fn list_recent_threads(
        &self,
        limit: usize,
        offset: usize,
    ) -> GaryxDbResult<Vec<RecentThreadRecord>> {
        let conn = self.conn()?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let mut stmt = conn.prepare(
            "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                    message_count, last_message_preview, recent_run_id, active_run_id, run_state,
                    updated_at, last_active_at, recorded_at
             FROM recent_threads
             ORDER BY last_active_at DESC, thread_id ASC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(RecentThreadRecord {
                thread_id: row.get(0)?,
                title: row.get(1)?,
                workspace_dir: row.get(2)?,
                thread_type: row.get(3)?,
                provider_type: row.get(4)?,
                agent_id: row.get(5)?,
                message_count: row.get(6)?,
                last_message_preview: row.get(7)?,
                recent_run_id: row.get(8)?,
                active_run_id: row.get(9)?,
                run_state: row.get(10)?,
                updated_at: row.get(11)?,
                last_active_at: row.get(12)?,
                recorded_at: row.get(13)?,
            })
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn count_recent_threads(&self) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM recent_threads", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn projection_state_matches(
        &self,
        projection_name: &str,
        projection_version: i64,
        source_row_count: usize,
    ) -> GaryxDbResult<bool> {
        let projection_name = normalize_required("projection_name", projection_name)?;
        let source_row_count = i64::try_from(source_row_count).unwrap_or(i64::MAX);
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT projection_version, source_row_count
                 FROM projection_states
                 WHERE projection_name = ?1",
                params![projection_name],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        Ok(row.is_some_and(|(version, count)| {
            version == projection_version && count == source_row_count
        }))
    }

    pub fn record_projection_state(
        &self,
        projection_name: &str,
        projection_version: i64,
        source_row_count: usize,
    ) -> GaryxDbResult<()> {
        let projection_name = normalize_required("projection_name", projection_name)?;
        let source_row_count = i64::try_from(source_row_count).unwrap_or(i64::MAX);
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO projection_states (
                projection_name, projection_version, source_row_count, projected_at
             )
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(projection_name) DO UPDATE SET
                projection_version = excluded.projection_version,
                source_row_count = excluded.source_row_count,
                projected_at = excluded.projected_at",
            params![
                projection_name,
                projection_version,
                source_row_count,
                now_string(),
            ],
        )?;
        Ok(())
    }

    pub fn sync_recent_threads_snapshot(
        &self,
        drafts: Vec<RecentThreadDraft>,
        max_records: usize,
    ) -> GaryxDbResult<()> {
        struct NormalizedRecentThreadDraft {
            thread_id: String,
            title: String,
            workspace_dir: Option<String>,
            thread_type: String,
            provider_type: Option<String>,
            agent_id: Option<String>,
            message_count: u32,
            last_message_preview: String,
            recent_run_id: Option<String>,
            active_run_id: Option<String>,
            run_state: String,
            updated_at: Option<String>,
            last_active_at: String,
            recorded_at: String,
        }

        let mut rows = Vec::new();
        for draft in drafts {
            let Ok(thread_id) = normalize_thread_id(&draft.thread_id) else {
                continue;
            };
            let Ok(thread_type) = normalize_required("thread_type", &draft.thread_type) else {
                continue;
            };
            let Ok(run_state) = normalize_required("run_state", &draft.run_state) else {
                continue;
            };
            let Ok(last_active_at) = normalize_required("last_active_at", &draft.last_active_at)
            else {
                continue;
            };
            rows.push(NormalizedRecentThreadDraft {
                thread_id,
                title: draft.title.trim().to_owned(),
                workspace_dir: normalize_optional(draft.workspace_dir.as_deref()),
                thread_type,
                provider_type: normalize_optional(draft.provider_type.as_deref()),
                agent_id: normalize_optional(draft.agent_id.as_deref()),
                message_count: draft.message_count,
                last_message_preview: draft.last_message_preview.trim().to_owned(),
                recent_run_id: normalize_optional(draft.recent_run_id.as_deref()),
                active_run_id: normalize_optional(draft.active_run_id.as_deref()),
                run_state,
                updated_at: normalize_optional(draft.updated_at.as_deref()),
                last_active_at,
                recorded_at: now_string(),
            });
        }

        let retained_thread_ids = rows
            .iter()
            .map(|row| row.thread_id.clone())
            .collect::<Vec<_>>();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        for row in rows {
            tx.execute(
                "INSERT INTO recent_threads (
                    thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                    message_count, last_message_preview, recent_run_id, active_run_id, run_state,
                    updated_at, last_active_at, recorded_at
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(thread_id) DO UPDATE SET
                    title = excluded.title,
                    workspace_dir = excluded.workspace_dir,
                    thread_type = excluded.thread_type,
                    provider_type = excluded.provider_type,
                    agent_id = excluded.agent_id,
                    message_count = excluded.message_count,
                    last_message_preview = excluded.last_message_preview,
                    recent_run_id = excluded.recent_run_id,
                    active_run_id = excluded.active_run_id,
                    run_state = excluded.run_state,
                    updated_at = excluded.updated_at,
                    last_active_at = excluded.last_active_at,
                    recorded_at = excluded.recorded_at",
                params![
                    row.thread_id,
                    row.title,
                    row.workspace_dir,
                    row.thread_type,
                    row.provider_type,
                    row.agent_id,
                    row.message_count,
                    row.last_message_preview,
                    row.recent_run_id,
                    row.active_run_id,
                    row.run_state,
                    row.updated_at,
                    row.last_active_at,
                    row.recorded_at,
                ],
            )?;
        }

        if retained_thread_ids.is_empty() {
            tx.execute("DELETE FROM recent_threads", [])?;
        } else {
            tx.execute(
                "CREATE TEMP TABLE IF NOT EXISTS recent_thread_sync_ids (
                    thread_id TEXT PRIMARY KEY
                 )",
                [],
            )?;
            tx.execute("DELETE FROM recent_thread_sync_ids", [])?;
            for thread_id in &retained_thread_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO recent_thread_sync_ids (thread_id) VALUES (?1)",
                    params![thread_id],
                )?;
            }
            tx.execute(
                "DELETE FROM recent_threads
                  WHERE thread_id NOT IN (
                    SELECT thread_id FROM recent_thread_sync_ids
                  )",
                [],
            )?;
            tx.execute("DELETE FROM recent_thread_sync_ids", [])?;
        }

        if max_records == 0 {
            tx.execute("DELETE FROM recent_threads", [])?;
        } else {
            let max_records = i64::try_from(max_records).unwrap_or(i64::MAX);
            tx.execute(
                "DELETE FROM recent_threads
                 WHERE thread_id IN (
                    SELECT thread_id
                      FROM recent_threads
                     ORDER BY last_active_at DESC, thread_id ASC
                     LIMIT -1 OFFSET ?1
                 )",
                params![max_records],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn upsert_recent_thread(
        &self,
        draft: RecentThreadDraft,
    ) -> GaryxDbResult<RecentThreadRecord> {
        let thread_id = normalize_thread_id(&draft.thread_id)?;
        let thread_type = normalize_required("thread_type", &draft.thread_type)?;
        let run_state = normalize_required("run_state", &draft.run_state)?;
        let last_active_at = normalize_required("last_active_at", &draft.last_active_at)?;
        let title = draft.title.trim().to_owned();
        let workspace_dir = normalize_optional(draft.workspace_dir.as_deref());
        let provider_type = normalize_optional(draft.provider_type.as_deref());
        let agent_id = normalize_optional(draft.agent_id.as_deref());
        let last_message_preview = draft.last_message_preview.trim().to_owned();
        let recent_run_id = normalize_optional(draft.recent_run_id.as_deref());
        let active_run_id = normalize_optional(draft.active_run_id.as_deref());
        let updated_at = normalize_optional(draft.updated_at.as_deref());
        let recorded_at = now_string();

        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO recent_threads (
                thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                message_count, last_message_preview, recent_run_id, active_run_id, run_state,
                updated_at, last_active_at, recorded_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(thread_id) DO UPDATE SET
                title = excluded.title,
                workspace_dir = excluded.workspace_dir,
                thread_type = excluded.thread_type,
                provider_type = excluded.provider_type,
                agent_id = excluded.agent_id,
                message_count = excluded.message_count,
                last_message_preview = excluded.last_message_preview,
                recent_run_id = excluded.recent_run_id,
                active_run_id = excluded.active_run_id,
                run_state = excluded.run_state,
                updated_at = excluded.updated_at,
                last_active_at = excluded.last_active_at,
                recorded_at = excluded.recorded_at",
            params![
                thread_id,
                title,
                workspace_dir,
                thread_type,
                provider_type,
                agent_id,
                draft.message_count,
                last_message_preview,
                recent_run_id,
                active_run_id,
                run_state,
                updated_at,
                last_active_at,
                recorded_at,
            ],
        )?;

        Ok(RecentThreadRecord {
            thread_id,
            title,
            workspace_dir,
            thread_type,
            provider_type,
            agent_id,
            message_count: draft.message_count,
            last_message_preview,
            recent_run_id,
            active_run_id,
            run_state,
            updated_at,
            last_active_at,
            recorded_at,
        })
    }

    pub fn remove_recent_thread(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.conn()?;
        let removed = conn.execute(
            "DELETE FROM recent_threads WHERE thread_id = ?1",
            params![thread_id],
        )?;
        Ok(removed > 0)
    }

    pub fn count_thread_meta_projection_rows(&self) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        let count: i64 = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM thread_meta) +
                (SELECT COUNT(*) FROM thread_channel_endpoints) +
                (SELECT COUNT(*) FROM thread_message_routes)",
            [],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn count_thread_meta_rows(&self) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM thread_meta", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn thread_meta_projection_needs_backfill(&self) -> GaryxDbResult<bool> {
        let conn = self.conn()?;
        let (total, current): (i64, i64) = conn.query_row(
            "SELECT COUNT(*),
                    SUM(CASE WHEN projection_version = ?1 THEN 1 ELSE 0 END)
             FROM thread_meta",
            params![CURRENT_THREAD_META_PROJECTION_VERSION],
            |row| Ok((row.get(0)?, row.get::<_, Option<i64>>(1)?.unwrap_or(0))),
        )?;
        Ok(total == 0 || current != total)
    }

    pub fn count_thread_channel_endpoints(&self) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM thread_channel_endpoints", [], |row| {
                row.get(0)
            })?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn list_thread_channel_endpoints(&self) -> GaryxDbResult<Vec<KnownChannelEndpoint>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT endpoint_key, channel, account_id, binding_key, chat_id,
                    delivery_target_type, delivery_target_id, display_label,
                    thread_id, thread_label, workspace_dir, thread_updated_at,
                    last_inbound_at, last_delivery_at
             FROM thread_channel_endpoints
             ORDER BY endpoint_key ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(KnownChannelEndpoint {
                endpoint_key: row.get(0)?,
                channel: row.get(1)?,
                account_id: row.get(2)?,
                binding_key: row.get(3)?,
                chat_id: row.get(4)?,
                delivery_target_type: row.get(5)?,
                delivery_target_id: row.get(6)?,
                display_label: row.get(7)?,
                thread_id: row.get(8)?,
                thread_label: row.get(9)?,
                workspace_dir: row.get(10)?,
                thread_updated_at: row.get(11)?,
                last_inbound_at: row.get(12)?,
                last_delivery_at: row.get(13)?,
            })
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn list_thread_message_routes(&self) -> GaryxDbResult<Vec<ThreadMessageRouteRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT thread_id, channel, account_id, chat_id, thread_binding_key,
                    message_id, projected_at
             FROM thread_message_routes
             ORDER BY projected_at ASC, thread_id ASC, message_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let thread_binding_key: String = row.get(4)?;
            Ok(ThreadMessageRouteRecord {
                thread_id: row.get(0)?,
                channel: row.get(1)?,
                account_id: row.get(2)?,
                chat_id: row.get(3)?,
                thread_binding_key: optional_from_stored_string(&thread_binding_key),
                message_id: row.get(5)?,
                projected_at: row.get(6)?,
            })
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn count_thread_meta_list(
        &self,
        include_hidden: bool,
        prefix: Option<&str>,
    ) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        let count: i64 = match prefix.map(str::trim).filter(|value| !value.is_empty()) {
            Some(prefix) if include_hidden => conn.query_row(
                "SELECT COUNT(*)
                 FROM thread_meta
                 WHERE substr(thread_id, 1, length(?1)) = ?1",
                params![prefix],
                |row| row.get(0),
            )?,
            Some(prefix) => conn.query_row(
                "SELECT COUNT(*)
                 FROM thread_meta
                 WHERE default_list_hidden = 0
                   AND substr(thread_id, 1, length(?1)) = ?1",
                params![prefix],
                |row| row.get(0),
            )?,
            None if include_hidden => {
                conn.query_row("SELECT COUNT(*) FROM thread_meta", [], |row| row.get(0))?
            }
            None => conn.query_row(
                "SELECT COUNT(*) FROM thread_meta WHERE default_list_hidden = 0",
                [],
                |row| row.get(0),
            )?,
        };
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn list_thread_meta_page(
        &self,
        limit: usize,
        offset: usize,
        include_hidden: bool,
        prefix: Option<&str>,
    ) -> GaryxDbResult<Vec<ThreadMetaRecord>> {
        let conn = self.conn()?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let sql = "SELECT thread_id, workspace_dir, thread_type, thread_label, agent_id,
                          provider_type, created_at, updated_at, message_count,
                          last_user_message, last_assistant_message, last_message_preview,
                          recent_run_id, active_run_id, worktree_json,
                          last_delivery_context_json, last_delivery_updated_at,
                          default_list_hidden, provider_key, selected_model,
                          selected_model_reasoning_effort, selected_model_service_tier,
                          sdk_session_id, projection_version, projected_at
                   FROM thread_meta";
        let order = " ORDER BY COALESCE(updated_at, projected_at) DESC, thread_id ASC
                      LIMIT ?1 OFFSET ?2";
        let mut records = Vec::new();
        match prefix.map(str::trim).filter(|value| !value.is_empty()) {
            Some(prefix) if include_hidden => {
                let mut stmt = conn.prepare(&format!(
                    "{sql} WHERE substr(thread_id, 1, length(?3)) = ?3{order}"
                ))?;
                let rows =
                    stmt.query_map(params![limit, offset, prefix], thread_meta_record_from_row)?;
                for row in rows {
                    records.push(row?);
                }
            }
            Some(prefix) => {
                let mut stmt = conn.prepare(&format!(
                    "{sql} WHERE default_list_hidden = 0
                            AND substr(thread_id, 1, length(?3)) = ?3{order}"
                ))?;
                let rows =
                    stmt.query_map(params![limit, offset, prefix], thread_meta_record_from_row)?;
                for row in rows {
                    records.push(row?);
                }
            }
            None if include_hidden => {
                let mut stmt = conn.prepare(&format!("{sql}{order}"))?;
                let rows = stmt.query_map(params![limit, offset], thread_meta_record_from_row)?;
                for row in rows {
                    records.push(row?);
                }
            }
            None => {
                let mut stmt =
                    conn.prepare(&format!("{sql} WHERE default_list_hidden = 0{order}"))?;
                let rows = stmt.query_map(params![limit, offset], thread_meta_record_from_row)?;
                for row in rows {
                    records.push(row?);
                }
            }
        }
        Ok(records)
    }

    pub fn list_thread_meta(&self) -> GaryxDbResult<Vec<ThreadMetaRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT thread_id, workspace_dir, thread_type, thread_label, agent_id,
                    provider_type, created_at, updated_at, message_count,
                    last_user_message, last_assistant_message, last_message_preview,
                    recent_run_id, active_run_id, worktree_json,
                    last_delivery_context_json, last_delivery_updated_at,
                    default_list_hidden, provider_key, selected_model,
                    selected_model_reasoning_effort, selected_model_service_tier,
                    sdk_session_id, projection_version, projected_at
             FROM thread_meta
             ORDER BY thread_id ASC",
        )?;
        let rows = stmt.query_map([], thread_meta_record_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn sync_thread_meta_projection_snapshot(
        &self,
        snapshot: ThreadMetaProjectionSnapshot,
    ) -> GaryxDbResult<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM thread_meta", [])?;
        tx.execute("DELETE FROM thread_channel_endpoints", [])?;
        tx.execute("DELETE FROM thread_message_routes", [])?;
        for meta in snapshot.thread_meta {
            upsert_thread_meta(&tx, &meta, &now_string())?;
        }
        for endpoint in snapshot.channel_endpoints {
            upsert_thread_channel_endpoint(&tx, &endpoint, &now_string())?;
        }
        for route in snapshot.message_routes {
            upsert_thread_message_route(&tx, &route, &now_string())?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn replace_thread_meta_projection(
        &self,
        draft: ThreadMetaProjectionDraft,
    ) -> GaryxDbResult<()> {
        let thread_id = normalize_thread_id(&draft.thread_id)?;
        let recorded_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        remove_thread_meta_projection_tx(&tx, &thread_id)?;
        let mut thread_meta = draft.thread_meta;
        thread_meta.thread_id = thread_id.clone();
        upsert_thread_meta(&tx, &thread_meta, &recorded_at)?;
        for mut endpoint in draft.channel_endpoints {
            endpoint.thread_id = Some(thread_id.clone());
            upsert_thread_channel_endpoint(&tx, &endpoint, &recorded_at)?;
        }
        for mut route in draft.message_routes {
            route.thread_id = thread_id.clone();
            upsert_thread_message_route(&tx, &route, &recorded_at)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_thread_meta_projection(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.conn()?;
        let removed = remove_thread_meta_projection_tx(&conn, &thread_id)?;
        Ok(removed > 0)
    }

    pub fn upsert_automation_thread_run(
        &self,
        draft: AutomationThreadRunDraft,
    ) -> GaryxDbResult<AutomationThreadRunRecord> {
        let automation_id = normalize_required("automation_id", &draft.automation_id)?;
        let run_id = normalize_required("run_id", &draft.run_id)?;
        let thread_id = normalize_thread_id(&draft.thread_id)?;
        let mode = normalize_automation_thread_run_mode(&draft.mode)?;
        let status = normalize_required("status", &draft.status)?;
        let started_at = normalize_required("started_at", &draft.started_at)?;
        let workspace_dir = normalize_optional(draft.workspace_dir.as_deref());
        let agent_id = normalize_optional(draft.agent_id.as_deref());
        let automation_label_snapshot =
            normalize_optional(draft.automation_label_snapshot.as_deref());
        let finished_at = normalize_optional(draft.finished_at.as_deref());
        let recorded_at = now_string();

        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO automation_thread_runs (
                automation_id, run_id, thread_id, workspace_dir, agent_id,
                automation_label_snapshot, mode, status, started_at, finished_at, recorded_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(automation_id, run_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                workspace_dir = excluded.workspace_dir,
                agent_id = excluded.agent_id,
                automation_label_snapshot = excluded.automation_label_snapshot,
                mode = excluded.mode,
                status = excluded.status,
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                recorded_at = excluded.recorded_at",
            params![
                automation_id,
                run_id,
                thread_id,
                workspace_dir,
                agent_id,
                automation_label_snapshot,
                mode,
                status,
                started_at,
                finished_at,
                recorded_at,
            ],
        )?;

        automation_thread_run_by_key(&conn, &automation_id, &run_id)?.ok_or_else(|| {
            GaryxDbError::BadRequest("automation thread run was not saved".to_owned())
        })
    }

    pub fn finish_automation_thread_run(
        &self,
        automation_id: &str,
        run_id: &str,
        status: &str,
        finished_at: &str,
    ) -> GaryxDbResult<bool> {
        let automation_id = normalize_required("automation_id", automation_id)?;
        let run_id = normalize_required("run_id", run_id)?;
        let status = normalize_required("status", status)?;
        let finished_at = normalize_required("finished_at", finished_at)?;
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE automation_thread_runs
             SET status = ?3, finished_at = ?4, recorded_at = ?5
             WHERE automation_id = ?1 AND run_id = ?2",
            params![automation_id, run_id, status, finished_at, now_string()],
        )?;
        Ok(updated > 0)
    }

    pub fn list_automation_thread_runs(
        &self,
        automation_id: &str,
        mode: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> GaryxDbResult<Vec<AutomationThreadRunRecord>> {
        let automation_id = normalize_required("automation_id", automation_id)?;
        let mode = mode.map(normalize_automation_thread_run_mode).transpose()?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let conn = self.conn()?;
        let sql = if mode.is_some() {
            "SELECT automation_id, run_id, thread_id, workspace_dir, agent_id,
                    automation_label_snapshot, mode, status, started_at, finished_at, recorded_at
             FROM automation_thread_runs
             WHERE automation_id = ?1 AND mode = ?2
             ORDER BY started_at DESC, recorded_at DESC, run_id ASC
             LIMIT ?3 OFFSET ?4"
        } else {
            "SELECT automation_id, run_id, thread_id, workspace_dir, agent_id,
                    automation_label_snapshot, mode, status, started_at, finished_at, recorded_at
             FROM automation_thread_runs
             WHERE automation_id = ?1
             ORDER BY started_at DESC, recorded_at DESC, run_id ASC
             LIMIT ?2 OFFSET ?3"
        };
        let mut stmt = conn.prepare(sql)?;
        let mut records = Vec::new();
        if let Some(mode) = mode {
            let rows = stmt.query_map(
                params![automation_id, mode, limit, offset],
                automation_thread_run_from_row,
            )?;
            for row in rows {
                records.push(row?);
            }
        } else {
            let rows = stmt.query_map(
                params![automation_id, limit, offset],
                automation_thread_run_from_row,
            )?;
            for row in rows {
                records.push(row?);
            }
        }
        Ok(records)
    }

    pub fn count_automation_thread_runs(
        &self,
        automation_id: &str,
        mode: Option<&str>,
    ) -> GaryxDbResult<usize> {
        let automation_id = normalize_required("automation_id", automation_id)?;
        let mode = mode.map(normalize_automation_thread_run_mode).transpose()?;
        let conn = self.conn()?;
        let count: i64 = if let Some(mode) = mode {
            conn.query_row(
                "SELECT COUNT(*) FROM automation_thread_runs WHERE automation_id = ?1 AND mode = ?2",
                params![automation_id, mode],
                |row| row.get(0),
            )?
        } else {
            conn.query_row(
                "SELECT COUNT(*) FROM automation_thread_runs WHERE automation_id = ?1",
                params![automation_id],
                |row| row.get(0),
            )?
        };
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn create_workflow_run(&self, draft: WorkflowRunDraft) -> GaryxDbResult<WorkflowRunRecord> {
        let workflow_id = draft
            .workflow_id
            .as_deref()
            .map(|value| normalize_required("workflow_id", value))
            .transpose()?
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let parent_thread_id = normalize_thread_id(&draft.parent_thread_id)?;
        let name = normalize_required("name", &draft.name)?;
        let status = normalize_workflow_run_status(&draft.status)?;
        let script_text = normalize_required("script_text", &draft.script_text)?;
        let meta_json = normalize_required("meta_json", &draft.meta_json)?;
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO workflow_runs (
                workflow_id, task_id, task_thread_id, workflow_definition_id,
                workflow_definition_version, workflow_definition_snapshot_json, input_json,
                parent_thread_id, parent_run_id, name, description, status,
                current_phase_index, script_text, meta_json, result_json, output_text, error,
                workspace_dir, created_by, total_children, completed_children, failed_children,
                total_input_tokens, total_output_tokens, total_tool_calls, total_cost_usd,
                created_at, started_at, finished_at, updated_at
             )
             VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, 0, 0, 0, 0, 0, 0, 0, ?21, ?22, ?23, ?21
             )",
            params![
                workflow_id,
                normalize_optional(draft.task_id.as_deref()),
                normalize_optional(draft.task_thread_id.as_deref()),
                normalize_optional(draft.workflow_definition_id.as_deref()),
                draft
                    .workflow_definition_version
                    .map(|version| i64::try_from(version).unwrap_or(i64::MAX)),
                normalize_optional(draft.workflow_definition_snapshot_json.as_deref()),
                normalize_optional(draft.input_json.as_deref()),
                parent_thread_id,
                normalize_optional(draft.parent_run_id.as_deref()),
                name,
                normalize_optional(draft.description.as_deref()),
                status,
                draft.current_phase_index,
                script_text,
                meta_json,
                normalize_optional(draft.result_json.as_deref()),
                normalize_optional(draft.output_text.as_deref()),
                normalize_optional(draft.error.as_deref()),
                normalize_optional(draft.workspace_dir.as_deref()),
                normalize_optional(draft.created_by.as_deref()),
                now,
                normalize_optional(draft.started_at.as_deref()),
                normalize_optional(draft.finished_at.as_deref()),
            ],
        )?;
        workflow_run_by_id(&conn, &workflow_id)?
            .ok_or_else(|| GaryxDbError::BadRequest("workflow run was not saved".to_owned()))
    }

    pub fn get_workflow_run(
        &self,
        workflow_run_id: &str,
    ) -> GaryxDbResult<Option<WorkflowRunRecord>> {
        let workflow_run_id = normalize_required("workflowRunId", workflow_run_id)?;
        let conn = self.conn()?;
        workflow_run_by_id(&conn, &workflow_run_id)
    }

    pub fn get_workflow_run_drilldown_snapshot(
        &self,
        workflow_run_id: &str,
        after_event_seq: u64,
        events_limit: usize,
    ) -> GaryxDbResult<Option<WorkflowRunDrilldownSnapshot>> {
        let workflow_run_id = normalize_required("workflowRunId", workflow_run_id)?;
        let after_event_seq = i64::try_from(after_event_seq).unwrap_or(i64::MAX);
        let events_limit = i64::try_from(events_limit).unwrap_or(i64::MAX);
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let Some(workflow) = workflow_run_by_id(&tx, &workflow_run_id)? else {
            tx.commit()?;
            return Ok(None);
        };
        let children = workflow_child_runs_for_workflow(&tx, &workflow_run_id)?;
        let events = workflow_events_after_for_workflow(
            &tx,
            &workflow_run_id,
            after_event_seq,
            events_limit,
        )?;
        let latest_event_seq = latest_workflow_event_seq(&tx, &workflow_run_id)?;
        tx.commit()?;
        Ok(Some(WorkflowRunDrilldownSnapshot {
            workflow,
            children,
            events,
            latest_event_seq,
        }))
    }

    pub fn list_workflow_runs(
        &self,
        parent_thread_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> GaryxDbResult<Vec<WorkflowRunRecord>> {
        let parent_thread_id = parent_thread_id.map(normalize_thread_id).transpose()?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let conn = self.conn()?;
        let sql = if parent_thread_id.is_some() {
            "SELECT workflow_id, task_id, task_thread_id, workflow_definition_id,
                    workflow_definition_version, workflow_definition_snapshot_json, input_json,
                    parent_thread_id, parent_run_id, name, description, status,
                    current_phase_index, script_text, meta_json, result_json, output_text, error,
                    workspace_dir, created_by, total_children, completed_children, failed_children,
                    total_input_tokens, total_output_tokens, total_tool_calls, total_cost_usd,
                    created_at, started_at, finished_at, updated_at
             FROM workflow_runs
             WHERE parent_thread_id = ?1
             ORDER BY created_at DESC, workflow_id ASC
             LIMIT ?2 OFFSET ?3"
        } else {
            "SELECT workflow_id, task_id, task_thread_id, workflow_definition_id,
                    workflow_definition_version, workflow_definition_snapshot_json, input_json,
                    parent_thread_id, parent_run_id, name, description, status,
                    current_phase_index, script_text, meta_json, result_json, output_text, error,
                    workspace_dir, created_by, total_children, completed_children, failed_children,
                    total_input_tokens, total_output_tokens, total_tool_calls, total_cost_usd,
                    created_at, started_at, finished_at, updated_at
             FROM workflow_runs
             ORDER BY created_at DESC, workflow_id ASC
             LIMIT ?1 OFFSET ?2"
        };
        let mut stmt = conn.prepare(sql)?;
        let mut records = Vec::new();
        if let Some(parent_thread_id) = parent_thread_id {
            let rows = stmt.query_map(
                params![parent_thread_id, limit, offset],
                workflow_run_from_row,
            )?;
            for row in rows {
                records.push(row?);
            }
        } else {
            let rows = stmt.query_map(params![limit, offset], workflow_run_from_row)?;
            for row in rows {
                records.push(row?);
            }
        }
        Ok(records)
    }

    pub fn update_workflow_run_status(
        &self,
        workflow_id: &str,
        status: &str,
        result_json: Option<&str>,
        output_text: Option<&str>,
        error: Option<&str>,
    ) -> GaryxDbResult<bool> {
        let workflow_id = normalize_required("workflow_id", workflow_id)?;
        let status = normalize_workflow_run_status(status)?;
        let now = now_string();
        let finished_at = if is_terminal_workflow_status(&status) {
            Some(now.as_str())
        } else {
            None
        };
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE workflow_runs
             SET status = ?2,
                 result_json = COALESCE(?3, result_json),
                 output_text = COALESCE(?4, output_text),
                 error = ?5,
                 finished_at = COALESCE(?6, finished_at),
                 updated_at = ?7
             WHERE workflow_id = ?1
               AND status NOT IN ('succeeded','failed','cancelled')",
            params![
                workflow_id,
                status,
                normalize_optional(result_json),
                normalize_optional(output_text),
                normalize_optional(error),
                finished_at,
                now,
            ],
        )?;
        Ok(updated > 0)
    }

    pub fn cancel_workflow_child_runs(
        &self,
        workflow_id: &str,
        error: &str,
    ) -> GaryxDbResult<usize> {
        let workflow_id = normalize_required("workflow_id", workflow_id)?;
        let error = normalize_required("error", error)?;
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE workflow_child_runs
             SET status = 'cancelled',
                 error = ?2,
                 finished_at = ?3,
                 updated_at = ?3
             WHERE workflow_id = ?1
               AND status NOT IN ('succeeded','failed','cancelled','skipped')",
            params![workflow_id, error, now],
        )?;
        if updated > 0 {
            tx.execute(
                "UPDATE workflow_runs
                 SET total_children = (
                        SELECT COUNT(*) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                     ),
                     completed_children = (
                        SELECT COUNT(*) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                           AND status IN ('succeeded','failed','cancelled','skipped')
                     ),
                     failed_children = (
                        SELECT COUNT(*) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                           AND status IN ('failed','cancelled')
                     ),
                     updated_at = ?2
                 WHERE workflow_id = ?1",
                params![workflow_id, now],
            )?;
        }
        tx.commit()?;
        Ok(updated)
    }

    pub fn upsert_workflow_child_run(
        &self,
        draft: WorkflowChildRunDraft,
    ) -> GaryxDbResult<WorkflowChildRunRecord> {
        let workflow_id = normalize_required("workflow_id", &draft.workflow_id)?;
        let workflow_child_run_id = draft
            .workflow_child_run_id
            .as_deref()
            .map(|value| normalize_required("workflow_child_run_id", value))
            .transpose()?
            .unwrap_or_else(|| format!("workflow-child::{}", Uuid::new_v4()));
        let thread_id = normalize_thread_id(&draft.thread_id)?;
        let phase_title = normalize_required("phase_title", &draft.phase_title)?;
        let label = normalize_required("label", &draft.label)?;
        let status = normalize_workflow_child_status(&draft.status)?;
        let prompt = normalize_required("prompt", &draft.prompt)?;
        let result_mode = normalize_workflow_result_mode(&draft.result_mode)?;
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO workflow_child_runs (
                workflow_id, workflow_child_run_id, thread_id, phase_index, phase_title, label,
                agent_id, status, prompt, result_mode, schema_json, result_text, result_json,
                result_preview, error, input_tokens, output_tokens, tool_calls, cost_usd,
                queued_at, started_at, finished_at, updated_at
             )
             VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?20
             )
             ON CONFLICT(workflow_id, workflow_child_run_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                phase_index = excluded.phase_index,
                phase_title = excluded.phase_title,
                label = excluded.label,
                agent_id = excluded.agent_id,
                status = excluded.status,
                prompt = excluded.prompt,
                result_mode = excluded.result_mode,
                schema_json = excluded.schema_json,
                result_text = excluded.result_text,
                result_json = excluded.result_json,
                result_preview = excluded.result_preview,
                error = excluded.error,
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                tool_calls = excluded.tool_calls,
                cost_usd = excluded.cost_usd,
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                updated_at = excluded.updated_at",
            params![
                workflow_id,
                workflow_child_run_id,
                thread_id,
                draft.phase_index,
                phase_title,
                label,
                normalize_optional(draft.agent_id.as_deref()),
                status,
                prompt,
                result_mode,
                normalize_optional(draft.schema_json.as_deref()),
                normalize_optional(draft.result_text.as_deref()),
                normalize_optional(draft.result_json.as_deref()),
                normalize_optional(draft.result_preview.as_deref()),
                normalize_optional(draft.error.as_deref()),
                draft.input_tokens,
                draft.output_tokens,
                draft.tool_calls,
                draft.cost_usd,
                now,
                normalize_optional(draft.started_at.as_deref()),
                normalize_optional(draft.finished_at.as_deref()),
            ],
        )?;
        conn.execute(
            "UPDATE workflow_runs
             SET total_children = (
                    SELECT COUNT(*) FROM workflow_child_runs
                     WHERE workflow_id = ?1
                 ),
                 updated_at = ?2
             WHERE workflow_id = ?1",
            params![workflow_id, now],
        )?;
        workflow_child_run_by_id(&conn, &workflow_id, &workflow_child_run_id)?
            .ok_or_else(|| GaryxDbError::BadRequest("workflow child run was not saved".to_owned()))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_workflow_child_run(
        &self,
        workflow_id: &str,
        workflow_child_run_id: &str,
        status: &str,
        result_text: Option<&str>,
        result_json: Option<&str>,
        result_preview: Option<&str>,
        error: Option<&str>,
        usage: Option<WorkflowChildRunUsage>,
    ) -> GaryxDbResult<bool> {
        let workflow_id = normalize_required("workflow_id", workflow_id)?;
        let workflow_child_run_id =
            normalize_required("workflow_child_run_id", workflow_child_run_id)?;
        let status = normalize_workflow_child_status(status)?;
        if !is_terminal_workflow_child_status(&status) {
            return Err(GaryxDbError::BadRequest(
                "child run finish status must be terminal".to_owned(),
            ));
        }
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE workflow_child_runs
             SET status = ?3,
                 result_text = ?4,
                 result_json = ?5,
                 result_preview = ?6,
                 error = ?7,
                 input_tokens = COALESCE(?8, input_tokens),
                 output_tokens = COALESCE(?9, output_tokens),
                 tool_calls = COALESCE(?10, tool_calls),
                 cost_usd = COALESCE(?11, cost_usd),
                 finished_at = ?12,
                 updated_at = ?12
             WHERE workflow_id = ?1
               AND workflow_child_run_id = ?2
               AND status NOT IN ('succeeded','failed','cancelled','skipped')",
            params![
                workflow_id,
                workflow_child_run_id,
                status,
                normalize_optional(result_text),
                normalize_optional(result_json),
                normalize_optional(result_preview),
                normalize_optional(error),
                usage.map(|usage| usage.input_tokens),
                usage.map(|usage| usage.output_tokens),
                usage.map(|usage| usage.tool_calls),
                usage.map(|usage| usage.cost_usd),
                now,
            ],
        )?;
        if updated > 0 {
            tx.execute(
                "UPDATE workflow_runs
                 SET total_children = (
                        SELECT COUNT(*) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                     ),
                     completed_children = (
                        SELECT COUNT(*) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                           AND status IN ('succeeded','failed','cancelled','skipped')
                     ),
                     failed_children = (
                        SELECT COUNT(*) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                           AND status IN ('failed','cancelled')
                     ),
                     total_input_tokens = COALESCE((
                        SELECT SUM(input_tokens) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                     ), 0),
                     total_output_tokens = COALESCE((
                        SELECT SUM(output_tokens) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                     ), 0),
                     total_tool_calls = COALESCE((
                        SELECT SUM(tool_calls) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                     ), 0),
                     total_cost_usd = COALESCE((
                        SELECT SUM(cost_usd) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                     ), 0),
                     updated_at = ?2
                 WHERE workflow_id = ?1",
                params![workflow_id, now],
            )?;
        }
        tx.commit()?;
        Ok(updated > 0)
    }

    pub fn submit_workflow_child_result(
        &self,
        workflow_id: &str,
        workflow_child_run_id: &str,
        thread_id: &str,
        result_json: &str,
        result_preview: Option<&str>,
    ) -> GaryxDbResult<bool> {
        let workflow_id = normalize_required("workflow_id", workflow_id)?;
        let workflow_child_run_id =
            normalize_required("workflow_child_run_id", workflow_child_run_id)?;
        let thread_id = normalize_thread_id(thread_id)?;
        let result_json = normalize_required("result_json", result_json)?;
        let now = now_string();
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE workflow_child_runs
             SET result_json = ?4,
                 result_preview = ?5,
                 updated_at = ?6
             WHERE workflow_id = ?1
               AND workflow_child_run_id = ?2
               AND thread_id = ?3
               AND result_json IS NULL
               AND status NOT IN ('succeeded','failed','cancelled','skipped')",
            params![
                workflow_id,
                workflow_child_run_id,
                thread_id,
                result_json,
                normalize_optional(result_preview),
                now,
            ],
        )?;
        Ok(updated > 0)
    }

    pub fn get_workflow_child_run(
        &self,
        workflow_id: &str,
        workflow_child_run_id: &str,
    ) -> GaryxDbResult<Option<WorkflowChildRunRecord>> {
        let workflow_id = normalize_required("workflow_id", workflow_id)?;
        let workflow_child_run_id =
            normalize_required("workflow_child_run_id", workflow_child_run_id)?;
        let conn = self.conn()?;
        workflow_child_run_by_id(&conn, &workflow_id, &workflow_child_run_id)
    }

    pub fn list_workflow_child_runs(
        &self,
        workflow_id: &str,
    ) -> GaryxDbResult<Vec<WorkflowChildRunRecord>> {
        let workflow_id = normalize_required("workflow_id", workflow_id)?;
        let conn = self.conn()?;
        workflow_child_runs_for_workflow(&conn, &workflow_id)
    }

    pub fn append_workflow_event(
        &self,
        draft: WorkflowEventDraft,
    ) -> GaryxDbResult<WorkflowEventRecord> {
        let workflow_id = normalize_required("workflow_id", &draft.workflow_id)?;
        let event_id = draft
            .event_id
            .as_deref()
            .map(|value| normalize_required("event_id", value))
            .transpose()?
            .unwrap_or_else(|| format!("workflow-event::{}", Uuid::new_v4()));
        let event_type = normalize_required("event_type", &draft.event_type)?;
        let payload_json = normalize_required("payload_json", &draft.payload_json)?;
        let created_at = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO workflow_events (
                event_id, workflow_id, workflow_child_run_id, thread_id, event_type,
                payload_json, created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event_id,
                workflow_id,
                normalize_optional(draft.workflow_child_run_id.as_deref()),
                normalize_optional(draft.thread_id.as_deref()),
                event_type,
                payload_json,
                created_at,
            ],
        )?;
        if event_type == "workflow.phase_started"
            && let Some(phase_index) = workflow_phase_index_from_payload(&payload_json)
        {
            conn.execute(
                "UPDATE workflow_runs
                 SET current_phase_index = ?2,
                     updated_at = ?3
                 WHERE workflow_id = ?1",
                params![workflow_id, phase_index, created_at],
            )?;
        }
        workflow_event_by_id(&conn, &event_id)?
            .ok_or_else(|| GaryxDbError::BadRequest("workflow event was not saved".to_owned()))
    }

    pub fn list_workflow_events_after(
        &self,
        workflow_id: &str,
        after_event_seq: u64,
        limit: usize,
    ) -> GaryxDbResult<Vec<WorkflowEventRecord>> {
        let workflow_id = normalize_required("workflow_id", workflow_id)?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let after_event_seq = i64::try_from(after_event_seq).unwrap_or(i64::MAX);
        let conn = self.conn()?;
        workflow_events_after_for_workflow(&conn, &workflow_id, after_event_seq, limit)
    }

    pub fn list_interrupted_workflow_task_references(
        &self,
        created_before_or_at: &str,
    ) -> GaryxDbResult<Vec<InterruptedWorkflowTaskReference>> {
        let created_before_or_at =
            normalize_required("created_before_or_at", created_before_or_at)?;
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT workflow_id, task_thread_id
             FROM workflow_runs
             WHERE status IN ('planning','queued','running')
               AND task_thread_id IS NOT NULL
               AND created_at <= ?1
             ORDER BY created_at ASC, workflow_id ASC",
        )?;
        let rows = stmt.query_map(params![created_before_or_at], |row| {
            Ok(InterruptedWorkflowTaskReference {
                workflow_id: row.get(0)?,
                task_thread_id: row.get(1)?,
            })
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn reconcile_interrupted_workflows(
        &self,
        error: &str,
        created_before_or_at: &str,
    ) -> GaryxDbResult<usize> {
        let error = normalize_required("error", error)?;
        let created_before_or_at =
            normalize_required("created_before_or_at", created_before_or_at)?;
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "SELECT workflow_id
             FROM workflow_runs
             WHERE status IN ('planning','queued','running')
               AND created_at <= ?1
             ORDER BY created_at ASC, workflow_id ASC",
        )?;
        let rows = stmt.query_map(params![created_before_or_at], |row| row.get::<_, String>(0))?;
        let mut workflow_ids = Vec::new();
        for row in rows {
            workflow_ids.push(row?);
        }
        drop(stmt);

        for workflow_id in &workflow_ids {
            tx.execute(
                "UPDATE workflow_child_runs
                 SET status = 'failed',
                     error = ?2,
                     finished_at = ?3,
                     updated_at = ?3
                 WHERE workflow_id = ?1
                   AND status NOT IN ('succeeded','failed','cancelled','skipped')",
                params![workflow_id, error, now],
            )?;
            tx.execute(
                "UPDATE workflow_runs
                 SET status = 'failed',
                     error = ?2,
                     finished_at = ?3,
                     updated_at = ?3,
                     total_children = (
                        SELECT COUNT(*) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                     ),
                     completed_children = (
                        SELECT COUNT(*) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                           AND status IN ('succeeded','failed','cancelled','skipped')
                     ),
                     failed_children = (
                        SELECT COUNT(*) FROM workflow_child_runs
                         WHERE workflow_id = ?1
                           AND status IN ('failed','cancelled')
                     )
                 WHERE workflow_id = ?1
                   AND status IN ('planning','queued','running')",
                params![workflow_id, error, now],
            )?;
            tx.execute(
                "INSERT INTO workflow_events (
                    event_id, workflow_id, workflow_child_run_id, thread_id, event_type,
                    payload_json, created_at
                 )
                 VALUES (?1, ?2, NULL, NULL, 'workflow.failed', ?3, ?4)",
                params![
                    format!("workflow-event::{}", Uuid::new_v4()),
                    workflow_id,
                    serde_json::json!({ "error": error }).to_string(),
                    now,
                ],
            )?;
        }

        tx.commit()?;
        Ok(workflow_ids.len())
    }

    pub fn replace_dreams_in_window(
        &self,
        scanned_from: &str,
        scanned_to: &str,
        source: &str,
        topics: &[DreamTopicDraft],
        error: Option<&str>,
    ) -> GaryxDbResult<DreamScanRunRecord> {
        let scanned_from = normalize_required("scanned_from", scanned_from)?;
        let scanned_to = normalize_required("scanned_to", scanned_to)?;
        let source = normalize_required("source", source)?;
        if scanned_from > scanned_to {
            return Err(GaryxDbError::BadRequest(
                "scanned_from must not be later than scanned_to".to_owned(),
            ));
        }

        let created_at = now_string();
        let run_id = format!("dream_scan::{}", Uuid::new_v4());
        let status = if error.is_some() { "fallback" } else { "ok" }.to_owned();
        let topics_count = topics.len() as u32;
        let spans_count = topics
            .iter()
            .map(|topic| topic.spans.len() as u32)
            .sum::<u32>();

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM dream_topics
             WHERE first_message_at >= ?1 AND last_message_at <= ?2",
            params![scanned_from, scanned_to],
        )?;
        tx.execute(
            "INSERT INTO dream_scan_runs (
                run_id, scanned_from, scanned_to, created_at, source, status,
                topics_count, spans_count, error
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                run_id,
                scanned_from,
                scanned_to,
                created_at,
                source,
                status,
                topics_count,
                spans_count,
                error.map(str::trim).filter(|value| !value.is_empty()),
            ],
        )?;

        for topic in topics {
            let dream_id = normalize_required("dream_id", &topic.dream_id)?;
            let title = normalize_required("title", &topic.title)?;
            let summary = topic.summary.trim().to_owned();
            let first_message_at = normalize_required("first_message_at", &topic.first_message_at)?;
            let last_message_at = normalize_required("last_message_at", &topic.last_message_at)?;
            if first_message_at > last_message_at {
                return Err(GaryxDbError::BadRequest(format!(
                    "dream topic {dream_id} has first_message_at later than last_message_at"
                )));
            }
            let topic_source = normalize_required("source", &topic.source)?;
            let confidence = topic.confidence.clamp(0.0, 1.0);
            let span_count = topic.spans.len() as u32;
            tx.execute(
                "INSERT INTO dream_topics (
                    dream_id, title, summary, first_message_at, last_message_at,
                    updated_at, source, confidence, message_count, span_count
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(dream_id) DO UPDATE SET
                    title = excluded.title,
                    summary = excluded.summary,
                    first_message_at = excluded.first_message_at,
                    last_message_at = excluded.last_message_at,
                    updated_at = excluded.updated_at,
                    source = excluded.source,
                    confidence = excluded.confidence,
                    message_count = excluded.message_count,
                    span_count = excluded.span_count",
                params![
                    dream_id,
                    title,
                    summary,
                    first_message_at,
                    last_message_at,
                    created_at,
                    topic_source,
                    confidence,
                    topic.message_count,
                    span_count,
                ],
            )?;
            for span in &topic.spans {
                let span_id = normalize_required("span_id", &span.span_id)?;
                let thread_id = normalize_thread_id(&span.thread_id)?;
                let start_at = normalize_required("start_at", &span.start_at)?;
                let end_at = normalize_required("end_at", &span.end_at)?;
                if span.start_seq == 0 || span.end_seq == 0 || span.start_seq > span.end_seq {
                    return Err(GaryxDbError::BadRequest(format!(
                        "dream span {span_id} has an invalid sequence range"
                    )));
                }
                if start_at > end_at {
                    return Err(GaryxDbError::BadRequest(format!(
                        "dream span {span_id} has start_at later than end_at"
                    )));
                }
                tx.execute(
                    "INSERT INTO dream_spans (
                        span_id, dream_id, thread_id, workspace_dir, start_seq, end_seq,
                        start_at, end_at, excerpt, message_count, created_at
                     )
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        span_id,
                        dream_id,
                        thread_id,
                        span.workspace_dir
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty()),
                        span.start_seq,
                        span.end_seq,
                        start_at,
                        end_at,
                        span.excerpt.trim(),
                        span.message_count,
                        created_at,
                    ],
                )?;
            }
        }
        tx.commit()?;

        Ok(DreamScanRunRecord {
            run_id,
            scanned_from,
            scanned_to,
            created_at,
            source,
            status,
            topics_count,
            spans_count,
            error: error
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        })
    }

    pub fn upsert_dreams_incremental(
        &self,
        scanned_from: &str,
        scanned_to: &str,
        source: &str,
        topics: &[DreamTopicDraft],
        error: Option<&str>,
    ) -> GaryxDbResult<DreamScanRunRecord> {
        let scanned_from = normalize_required("scanned_from", scanned_from)?;
        let scanned_to = normalize_required("scanned_to", scanned_to)?;
        let source = normalize_required("source", source)?;
        if scanned_from > scanned_to {
            return Err(GaryxDbError::BadRequest(
                "scanned_from must not be later than scanned_to".to_owned(),
            ));
        }

        let created_at = now_string();
        let run_id = format!("dream_scan::{}", Uuid::new_v4());
        let status = if error.is_some() { "fallback" } else { "ok" }.to_owned();
        let topics_count = topics.len() as u32;
        let spans_count = topics
            .iter()
            .map(|topic| topic.spans.len() as u32)
            .sum::<u32>();

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO dream_scan_runs (
                run_id, scanned_from, scanned_to, created_at, source, status,
                topics_count, spans_count, error
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                run_id,
                scanned_from,
                scanned_to,
                created_at,
                source,
                status,
                topics_count,
                spans_count,
                error.map(str::trim).filter(|value| !value.is_empty()),
            ],
        )?;

        for topic in topics {
            let resolution = resolve_incremental_dream_id(&tx, &topic.dream_id, &topic.spans)?;
            let dream_id = resolution.dream_id;
            let title = normalize_required("title", &topic.title)?;
            let summary = topic.summary.trim().to_owned();
            let topic_source = normalize_required("source", &topic.source)?;
            let confidence = topic.confidence.clamp(0.0, 1.0);
            let mut span_map = BTreeMap::<(String, u64, u64), DreamSpanDraft>::new();
            let mut existing_span_keys = BTreeSet::<(String, u64, u64)>::new();

            for (index, span_dream_id) in std::iter::once(&dream_id)
                .chain(resolution.duplicate_dream_ids.iter())
                .enumerate()
            {
                let mut stmt = tx.prepare(
                    "SELECT span_id, thread_id, workspace_dir, start_seq, end_seq,
                            start_at, end_at, excerpt, message_count
                     FROM dream_spans
                     WHERE dream_id = ?1",
                )?;
                let rows = stmt.query_map(params![span_dream_id.as_str()], |row| {
                    Ok(DreamSpanDraft {
                        span_id: row.get(0)?,
                        thread_id: row.get(1)?,
                        workspace_dir: row.get(2)?,
                        start_seq: row.get(3)?,
                        end_seq: row.get(4)?,
                        start_at: row.get(5)?,
                        end_at: row.get(6)?,
                        excerpt: row.get(7)?,
                        message_count: row.get(8)?,
                    })
                })?;
                for row in rows {
                    let span = row?;
                    if index == 0 {
                        existing_span_keys.insert((
                            span.thread_id.clone(),
                            span.start_seq,
                            span.end_seq,
                        ));
                    }
                    span_map
                        .entry((span.thread_id.clone(), span.start_seq, span.end_seq))
                        .or_insert(span);
                }
            }

            for span in &topic.spans {
                let thread_id = normalize_thread_id(&span.thread_id)?;
                let start_at = normalize_required("start_at", &span.start_at)?;
                let end_at = normalize_required("end_at", &span.end_at)?;
                let key = (thread_id.clone(), span.start_seq, span.end_seq);
                let span_id = span_map
                    .get(&key)
                    .or_else(|| {
                        span_map.values().find(|existing| {
                            dream_span_ranges_overlap(
                                &thread_id,
                                span.start_seq,
                                span.end_seq,
                                existing,
                            )
                        })
                    })
                    .map(|existing| existing.span_id.clone())
                    .unwrap_or_else(|| span.span_id.trim().to_owned());
                let span_id = normalize_required("span_id", &span_id)?;
                if span.start_seq == 0 || span.end_seq == 0 || span.start_seq > span.end_seq {
                    return Err(GaryxDbError::BadRequest(format!(
                        "dream span {span_id} has an invalid sequence range"
                    )));
                }
                if start_at > end_at {
                    return Err(GaryxDbError::BadRequest(format!(
                        "dream span {span_id} has start_at later than end_at"
                    )));
                }
                span_map.insert(
                    key,
                    DreamSpanDraft {
                        span_id,
                        thread_id,
                        workspace_dir: span.workspace_dir.clone(),
                        start_seq: span.start_seq,
                        end_seq: span.end_seq,
                        start_at,
                        end_at,
                        excerpt: span.excerpt.trim().to_owned(),
                        message_count: span.message_count,
                    },
                );
            }

            let spans = merge_overlapping_dream_spans(span_map.into_values());
            let retained_span_keys = spans
                .iter()
                .map(|span| (span.thread_id.clone(), span.start_seq, span.end_seq))
                .collect::<BTreeSet<_>>();
            let first_message_at = spans
                .iter()
                .map(|span| span.start_at.as_str())
                .min()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| topic.first_message_at.trim().to_owned());
            let last_message_at = spans
                .iter()
                .map(|span| span.end_at.as_str())
                .max()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| topic.last_message_at.trim().to_owned());
            let first_message_at = normalize_required("first_message_at", &first_message_at)?;
            let last_message_at = normalize_required("last_message_at", &last_message_at)?;
            if first_message_at > last_message_at {
                return Err(GaryxDbError::BadRequest(format!(
                    "dream topic {dream_id} has first_message_at later than last_message_at"
                )));
            }
            let message_count = spans.iter().map(|span| span.message_count).sum::<u32>();
            let span_count = spans.len() as u32;

            for duplicate_dream_id in &resolution.duplicate_dream_ids {
                tx.execute(
                    "DELETE FROM dream_topics WHERE dream_id = ?1",
                    params![duplicate_dream_id],
                )?;
            }

            tx.execute(
                "INSERT INTO dream_topics (
                    dream_id, title, summary, first_message_at, last_message_at,
                    updated_at, source, confidence, message_count, span_count
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(dream_id) DO UPDATE SET
                    title = excluded.title,
                    summary = excluded.summary,
                    first_message_at = excluded.first_message_at,
                    last_message_at = excluded.last_message_at,
                    updated_at = excluded.updated_at,
                    source = excluded.source,
                    confidence = excluded.confidence,
                    message_count = excluded.message_count,
                    span_count = excluded.span_count",
                params![
                    dream_id,
                    title,
                    summary,
                    first_message_at,
                    last_message_at,
                    created_at,
                    topic_source,
                    confidence,
                    message_count,
                    span_count,
                ],
            )?;
            for (thread_id, start_seq, end_seq) in
                existing_span_keys.difference(&retained_span_keys)
            {
                tx.execute(
                    "DELETE FROM dream_spans
                     WHERE dream_id = ?1
                       AND thread_id = ?2
                       AND start_seq = ?3
                       AND end_seq = ?4",
                    params![dream_id, thread_id, start_seq, end_seq],
                )?;
            }
            for span in spans {
                let updated = tx.execute(
                    "UPDATE dream_spans
                     SET workspace_dir = ?1,
                         start_at = ?2,
                         end_at = ?3,
                         excerpt = ?4,
                         message_count = ?5
                     WHERE dream_id = ?6
                       AND thread_id = ?7
                       AND start_seq = ?8
                       AND end_seq = ?9",
                    params![
                        span.workspace_dir
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty()),
                        span.start_at,
                        span.end_at,
                        span.excerpt.trim(),
                        span.message_count,
                        dream_id,
                        span.thread_id,
                        span.start_seq,
                        span.end_seq,
                    ],
                )?;
                if updated == 0 {
                    tx.execute(
                        "INSERT INTO dream_spans (
                            span_id, dream_id, thread_id, workspace_dir, start_seq, end_seq,
                            start_at, end_at, excerpt, message_count, created_at
                         )
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            span.span_id,
                            dream_id,
                            span.thread_id,
                            span.workspace_dir
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty()),
                            span.start_seq,
                            span.end_seq,
                            span.start_at,
                            span.end_at,
                            span.excerpt.trim(),
                            span.message_count,
                            created_at,
                        ],
                    )?;
                }
            }
        }
        tx.commit()?;

        Ok(DreamScanRunRecord {
            run_id,
            scanned_from,
            scanned_to,
            created_at,
            source,
            status,
            topics_count,
            spans_count,
            error: error
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        })
    }

    pub fn list_dream_topics(
        &self,
        from: Option<&str>,
        to: Option<&str>,
        limit: usize,
    ) -> GaryxDbResult<Vec<DreamTopicRecord>> {
        let limit = limit.clamp(1, 500) as i64;
        let from = normalize_optional(from);
        let to = normalize_optional(to);
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT dream_id, title, summary, first_message_at, last_message_at,
                    updated_at, source, confidence, message_count, span_count
             FROM dream_topics
             WHERE (?1 IS NULL OR last_message_at >= ?1)
               AND (?2 IS NULL OR first_message_at <= ?2)
             ORDER BY last_message_at DESC, dream_id ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![from.as_deref(), to.as_deref(), limit], |row| {
            Ok(DreamTopicRecord {
                dream_id: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                first_message_at: row.get(3)?,
                last_message_at: row.get(4)?,
                updated_at: row.get(5)?,
                source: row.get(6)?,
                confidence: row.get(7)?,
                message_count: row.get(8)?,
                span_count: row.get(9)?,
                spans: Vec::new(),
            })
        })?;
        let mut topics = Vec::new();
        for row in rows {
            topics.push(row?);
        }
        attach_dream_spans(&conn, &mut topics)?;
        Ok(topics)
    }

    pub fn get_dream_topic(&self, dream_id: &str) -> GaryxDbResult<Option<DreamTopicRecord>> {
        let dream_id = normalize_required("dream_id", dream_id)?;
        let conn = self.conn()?;
        let mut topic = conn
            .query_row(
                "SELECT dream_id, title, summary, first_message_at, last_message_at,
                        updated_at, source, confidence, message_count, span_count
                 FROM dream_topics
                 WHERE dream_id = ?1",
                params![dream_id],
                |row| {
                    Ok(DreamTopicRecord {
                        dream_id: row.get(0)?,
                        title: row.get(1)?,
                        summary: row.get(2)?,
                        first_message_at: row.get(3)?,
                        last_message_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        source: row.get(6)?,
                        confidence: row.get(7)?,
                        message_count: row.get(8)?,
                        span_count: row.get(9)?,
                        spans: Vec::new(),
                    })
                },
            )
            .optional()?;
        if let Some(topic) = topic.as_mut() {
            attach_dream_spans(&conn, std::slice::from_mut(topic))?;
        }
        Ok(topic)
    }

    pub fn list_dream_topics_for_threads(
        &self,
        thread_ids: &[String],
        from: Option<&str>,
        limit: usize,
    ) -> GaryxDbResult<Vec<DreamTopicRecord>> {
        let mut normalized_thread_ids = thread_ids
            .iter()
            .map(|thread_id| normalize_thread_id(thread_id))
            .collect::<GaryxDbResult<Vec<_>>>()?;
        normalized_thread_ids.sort();
        normalized_thread_ids.dedup();
        if normalized_thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let from = normalize_optional(from);

        let placeholders = std::iter::repeat_n("?", normalized_thread_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let limit = limit.clamp(1, 500).to_string();
        let time_filter = if from.is_some() {
            " AND t.last_message_at >= ?"
        } else {
            ""
        };
        let sql = format!(
            "SELECT DISTINCT t.dream_id, t.title, t.summary, t.first_message_at,
                    t.last_message_at, t.updated_at, t.source, t.confidence,
                    t.message_count, t.span_count
             FROM dream_topics t
             JOIN dream_spans s ON s.dream_id = t.dream_id
             WHERE s.thread_id IN ({placeholders}){time_filter}
             ORDER BY t.last_message_at DESC, t.dream_id ASC
             LIMIT {limit}"
        );
        let mut bind_values = normalized_thread_ids;
        if let Some(from) = from {
            bind_values.push(from);
        }
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(bind_values.iter()), |row| {
            Ok(DreamTopicRecord {
                dream_id: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                first_message_at: row.get(3)?,
                last_message_at: row.get(4)?,
                updated_at: row.get(5)?,
                source: row.get(6)?,
                confidence: row.get(7)?,
                message_count: row.get(8)?,
                span_count: row.get(9)?,
                spans: Vec::new(),
            })
        })?;
        let mut topics = Vec::new();
        for row in rows {
            topics.push(row?);
        }
        attach_dream_spans(&conn, &mut topics)?;
        Ok(topics)
    }

    pub fn latest_dream_scan(&self) -> GaryxDbResult<Option<DreamScanRunRecord>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT run_id, scanned_from, scanned_to, created_at, source, status,
                    topics_count, spans_count, error
             FROM dream_scan_runs
             ORDER BY created_at DESC, rowid DESC
             LIMIT 1",
            [],
            |row| {
                Ok(DreamScanRunRecord {
                    run_id: row.get(0)?,
                    scanned_from: row.get(1)?,
                    scanned_to: row.get(2)?,
                    created_at: row.get(3)?,
                    source: row.get(4)?,
                    status: row.get(5)?,
                    topics_count: row.get(6)?,
                    spans_count: row.get(7)?,
                    error: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }
}

fn merge_overlapping_dream_spans(
    spans: impl IntoIterator<Item = DreamSpanDraft>,
) -> Vec<DreamSpanDraft> {
    let mut by_thread = BTreeMap::<String, Vec<DreamSpanDraft>>::new();
    for span in spans {
        by_thread
            .entry(span.thread_id.clone())
            .or_default()
            .push(span);
    }

    let mut merged = Vec::new();
    for (_thread_id, mut spans) in by_thread {
        spans.sort_by(|left, right| {
            left.start_seq
                .cmp(&right.start_seq)
                .then_with(|| left.end_seq.cmp(&right.end_seq))
        });
        let mut current: Option<DreamSpanDraft> = None;
        for span in spans {
            match current.as_mut() {
                Some(active) if span.start_seq <= active.end_seq => {
                    active.end_seq = active.end_seq.max(span.end_seq);
                    if span.end_at > active.end_at {
                        active.end_at = span.end_at.clone();
                    }
                    if span.start_at < active.start_at {
                        active.start_at = span.start_at.clone();
                    }
                    if span.workspace_dir.is_some() {
                        active.workspace_dir = span.workspace_dir.clone();
                    }
                    if !span.excerpt.trim().is_empty() {
                        active.excerpt = span.excerpt.clone();
                    }
                    active.message_count = active.message_count.max(span.message_count);
                }
                _ => {
                    if let Some(previous) = current.replace(span) {
                        merged.push(previous);
                    }
                }
            }
        }
        if let Some(last) = current {
            merged.push(last);
        }
    }
    merged
}

fn resolve_incremental_dream_id(
    tx: &Transaction<'_>,
    requested_dream_id: &str,
    spans: &[DreamSpanDraft],
) -> GaryxDbResult<DreamIdResolution> {
    let requested_dream_id = normalize_required("dream_id", requested_dream_id)?;
    let requested_exists = tx
        .query_row(
            "SELECT 1 FROM dream_topics WHERE dream_id = ?1",
            params![requested_dream_id.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some();

    let mut overlap_scores = BTreeMap::<String, DreamOverlapCandidate>::new();
    let mut draft_span_keys = BTreeSet::<(String, u64, u64)>::new();
    for span in spans {
        let thread_id = normalize_thread_id(&span.thread_id)?;
        if span.start_seq == 0 || span.end_seq == 0 || span.start_seq > span.end_seq {
            return Err(GaryxDbError::BadRequest(format!(
                "dream span {} has an invalid sequence range",
                span.span_id.trim()
            )));
        }
        draft_span_keys.insert((thread_id.clone(), span.start_seq, span.end_seq));

        let mut stmt = tx.prepare(
            "SELECT s.dream_id, s.start_seq, s.end_seq, t.last_message_at, t.span_count
             FROM dream_spans s
             JOIN dream_topics t ON t.dream_id = s.dream_id
             WHERE s.thread_id = ?1
               AND s.start_seq <= ?2
               AND s.end_seq >= ?3",
        )?;
        let rows = stmt.query_map(params![thread_id, span.end_seq, span.start_seq], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
            ))
        })?;
        for row in rows {
            let (dream_id, existing_start_seq, existing_end_seq, last_message_at, span_count) =
                row?;
            let overlap_start = span.start_seq.max(existing_start_seq);
            let overlap_end = span.end_seq.min(existing_end_seq);
            let overlap_width = overlap_end.saturating_sub(overlap_start) + 1;
            let entry = overlap_scores
                .entry(dream_id)
                .or_insert_with(|| DreamOverlapCandidate {
                    overlap_score: 0,
                    overlap_count: 0,
                    last_message_at: last_message_at.clone(),
                    exact_span_keys: BTreeSet::new(),
                    span_count,
                });
            entry.overlap_score = entry.overlap_score.saturating_add(overlap_width);
            entry.overlap_count += 1;
            if last_message_at > entry.last_message_at {
                entry.last_message_at = last_message_at;
            }
            entry.span_count = entry.span_count.max(span_count);
            if existing_start_seq == span.start_seq && existing_end_seq == span.end_seq {
                entry
                    .exact_span_keys
                    .insert((thread_id.clone(), span.start_seq, span.end_seq));
            }
        }
    }

    let dream_id = if requested_exists {
        requested_dream_id
    } else {
        overlap_scores
            .iter()
            .max_by(|left, right| {
                left.1
                    .overlap_score
                    .cmp(&right.1.overlap_score)
                    .then_with(|| left.1.overlap_count.cmp(&right.1.overlap_count))
                    .then_with(|| left.1.last_message_at.cmp(&right.1.last_message_at))
                    // Prefer the alphabetically smaller dream_id on a full tie
                    // so repeated scans make the same deterministic choice.
                    .then_with(|| right.0.cmp(left.0))
            })
            .map(|(dream_id, _)| dream_id.clone())
            .unwrap_or(requested_dream_id)
    };
    let duplicate_dream_ids = overlap_scores
        .into_iter()
        .filter_map(|(overlapping_dream_id, candidate)| {
            (overlapping_dream_id != dream_id
                && candidate.span_count as usize == draft_span_keys.len()
                && candidate.exact_span_keys == draft_span_keys)
                .then_some(overlapping_dream_id)
        })
        .collect();

    Ok(DreamIdResolution {
        dream_id,
        duplicate_dream_ids,
    })
}

fn dream_span_ranges_overlap(
    thread_id: &str,
    start_seq: u64,
    end_seq: u64,
    existing: &DreamSpanDraft,
) -> bool {
    existing.thread_id == thread_id
        && start_seq <= existing.end_seq
        && existing.start_seq <= end_seq
}

fn initialize_connection(conn: &Connection) -> GaryxDbResult<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS thread_pins (
            thread_id TEXT PRIMARY KEY,
            pinned_at TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS archived_threads (
            thread_id TEXT PRIMARY KEY,
            archived_at TEXT NOT NULL
        ) STRICT;

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
            recorded_at TEXT NOT NULL
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_recent_threads_last_active
            ON recent_threads(last_active_at DESC);

        CREATE TABLE IF NOT EXISTS projection_states (
            projection_name TEXT PRIMARY KEY,
            projection_version INTEGER NOT NULL,
            source_row_count INTEGER NOT NULL,
            projected_at TEXT NOT NULL
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
            provider_key TEXT,
            selected_model TEXT,
            selected_model_reasoning_effort TEXT,
            selected_model_service_tier TEXT,
            sdk_session_id TEXT,
            projection_version INTEGER NOT NULL DEFAULT 4,
            projected_at TEXT NOT NULL
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_thread_meta_workspace
            ON thread_meta(workspace_dir);

        CREATE INDEX IF NOT EXISTS idx_thread_meta_type_updated
            ON thread_meta(thread_type, updated_at DESC);

        CREATE INDEX IF NOT EXISTS idx_thread_meta_last_delivery
            ON thread_meta(last_delivery_updated_at DESC)
            WHERE last_delivery_context_json IS NOT NULL;

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

        CREATE TABLE IF NOT EXISTS thread_message_routes (
            channel TEXT NOT NULL,
            account_id TEXT NOT NULL,
            chat_id TEXT NOT NULL DEFAULT '',
            thread_binding_key TEXT NOT NULL DEFAULT '',
            message_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            projected_at TEXT NOT NULL,
            PRIMARY KEY (channel, account_id, chat_id, thread_binding_key, message_id)
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_thread_message_routes_thread
            ON thread_message_routes(thread_id);

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

        CREATE TABLE IF NOT EXISTS workflow_runs (
            workflow_id TEXT PRIMARY KEY,
            task_id TEXT,
            task_thread_id TEXT,
            workflow_definition_id TEXT,
            workflow_definition_version INTEGER,
            workflow_definition_snapshot_json TEXT,
            input_json TEXT,
            parent_thread_id TEXT NOT NULL,
            parent_run_id TEXT,
            name TEXT NOT NULL,
            description TEXT,
            status TEXT NOT NULL CHECK (
                status IN ('planning','queued','running','succeeded','failed','cancelled')
            ),
            current_phase_index INTEGER,
            script_text TEXT NOT NULL,
            meta_json TEXT NOT NULL,
            result_json TEXT,
            output_text TEXT,
            error TEXT,
            workspace_dir TEXT,
            created_by TEXT,
            total_children INTEGER NOT NULL DEFAULT 0,
            completed_children INTEGER NOT NULL DEFAULT 0,
            failed_children INTEGER NOT NULL DEFAULT 0,
            total_input_tokens INTEGER NOT NULL DEFAULT 0,
            total_output_tokens INTEGER NOT NULL DEFAULT 0,
            total_tool_calls INTEGER NOT NULL DEFAULT 0,
            total_cost_usd REAL NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            updated_at TEXT NOT NULL
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_workflow_runs_parent_thread
            ON workflow_runs(parent_thread_id, created_at DESC);

        CREATE INDEX IF NOT EXISTS idx_workflow_runs_status
            ON workflow_runs(status, updated_at DESC);

        CREATE TABLE IF NOT EXISTS workflow_child_runs (
            workflow_id TEXT NOT NULL,
            workflow_child_run_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            phase_index INTEGER NOT NULL,
            phase_title TEXT NOT NULL,
            label TEXT NOT NULL,
            agent_id TEXT,
            status TEXT NOT NULL CHECK (
                status IN ('queued','running','succeeded','failed','cancelled','skipped')
            ),
            prompt TEXT NOT NULL,
            result_mode TEXT NOT NULL CHECK (result_mode IN ('text','structured')),
            schema_json TEXT,
            result_text TEXT,
            result_json TEXT,
            result_preview TEXT,
            error TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            tool_calls INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL NOT NULL DEFAULT 0,
            queued_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (workflow_id, workflow_child_run_id)
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_workflow_child_runs_workflow
            ON workflow_child_runs(workflow_id, phase_index, queued_at);

        CREATE INDEX IF NOT EXISTS idx_workflow_child_runs_thread
            ON workflow_child_runs(thread_id);

        CREATE TABLE IF NOT EXISTS workflow_events (
            event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL UNIQUE,
            workflow_id TEXT NOT NULL,
            workflow_child_run_id TEXT,
            thread_id TEXT,
            event_type TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_workflow_events_workflow_seq
            ON workflow_events(workflow_id, event_seq);

        CREATE TABLE IF NOT EXISTS workspaces (
            path TEXT PRIMARY KEY,
            name TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted_at TEXT
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
            updated_at    TEXT NOT NULL
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_capsules_updated
            ON capsules(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_capsules_thread
            ON capsules(thread_id);

        CREATE TABLE IF NOT EXISTS dream_topics (
            dream_id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            summary TEXT NOT NULL DEFAULT '',
            first_message_at TEXT NOT NULL,
            last_message_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            source TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0,
            message_count INTEGER NOT NULL DEFAULT 0,
            span_count INTEGER NOT NULL DEFAULT 0
        ) STRICT;

        CREATE TABLE IF NOT EXISTS dream_spans (
            span_id TEXT PRIMARY KEY,
            dream_id TEXT NOT NULL REFERENCES dream_topics(dream_id) ON DELETE CASCADE,
            thread_id TEXT NOT NULL,
            workspace_dir TEXT,
            start_seq INTEGER NOT NULL,
            end_seq INTEGER NOT NULL,
            start_at TEXT NOT NULL,
            end_at TEXT NOT NULL,
            excerpt TEXT NOT NULL DEFAULT '',
            message_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            UNIQUE(dream_id, thread_id, start_seq, end_seq)
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_dream_topics_last_message_at
            ON dream_topics(last_message_at DESC);
        CREATE INDEX IF NOT EXISTS idx_dream_spans_thread
            ON dream_spans(thread_id, start_seq, end_seq);

        CREATE TABLE IF NOT EXISTS dream_scan_runs (
            run_id TEXT PRIMARY KEY,
            scanned_from TEXT NOT NULL,
            scanned_to TEXT NOT NULL,
            created_at TEXT NOT NULL,
            source TEXT NOT NULL,
            status TEXT NOT NULL,
            topics_count INTEGER NOT NULL DEFAULT 0,
            spans_count INTEGER NOT NULL DEFAULT 0,
            error TEXT
        ) STRICT;
        "#,
    )?;
    ensure_thread_meta_projection_columns(conn)?;
    ensure_thread_channel_endpoint_columns(conn)?;
    ensure_workflow_runs_task_columns(conn)?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_thread_channel_endpoints_thread
            ON thread_channel_endpoints(thread_id);

        CREATE INDEX IF NOT EXISTS idx_thread_channel_endpoints_channel_account
            ON thread_channel_endpoints(channel, account_id);

        CREATE INDEX IF NOT EXISTS idx_thread_meta_visible_updated
            ON thread_meta(default_list_hidden, updated_at DESC, projected_at DESC);

        CREATE INDEX IF NOT EXISTS idx_workflow_runs_definition
            ON workflow_runs(workflow_definition_id, created_at DESC);
        "#,
    )?;
    ensure_workspaces_deleted_at_column(conn)?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_workspaces_active_name_path
            ON workspaces(deleted_at, lower(COALESCE(NULLIF(name, ''), path)), lower(path));
        "#,
    )?;
    Ok(())
}

fn normalize_thread_id(thread_id: &str) -> GaryxDbResult<String> {
    let trimmed = thread_id.trim();
    if trimmed.is_empty() {
        return Err(GaryxDbError::BadRequest(
            "thread_id must not be empty".to_owned(),
        ));
    }
    Ok(trimmed.to_owned())
}

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn normalize_required(field: &str, value: &str) -> GaryxDbResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(GaryxDbError::BadRequest(format!(
            "{field} must not be empty"
        )));
    }
    Ok(trimmed.to_owned())
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(ToOwned::to_owned)
}

fn optional_from_stored_string(value: &str) -> Option<String> {
    normalize_optional(Some(value))
}

fn thread_meta_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadMetaRecord> {
    Ok(ThreadMetaRecord {
        thread_id: row.get(0)?,
        workspace_dir: row.get(1)?,
        thread_type: row.get(2)?,
        thread_label: row.get(3)?,
        agent_id: row.get(4)?,
        provider_type: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        message_count: row.get::<_, i64>(8)?.clamp(0, i64::from(u32::MAX)) as u32,
        last_user_message: row.get(9)?,
        last_assistant_message: row.get(10)?,
        last_message_preview: row.get(11)?,
        recent_run_id: row.get(12)?,
        active_run_id: row.get(13)?,
        worktree_json: row.get(14)?,
        last_delivery_context_json: row.get(15)?,
        last_delivery_updated_at: row.get(16)?,
        default_list_hidden: row.get::<_, i64>(17)? != 0,
        provider_key: row.get(18)?,
        selected_model: row.get(19)?,
        selected_model_reasoning_effort: row.get(20)?,
        selected_model_service_tier: row.get(21)?,
        sdk_session_id: row.get(22)?,
        projection_version: row.get(23)?,
        projected_at: row.get(24)?,
    })
}

fn remove_thread_meta_projection_tx(conn: &Connection, thread_id: &str) -> GaryxDbResult<usize> {
    let mut removed = 0usize;
    removed += conn.execute(
        "DELETE FROM thread_meta WHERE thread_id = ?1",
        params![thread_id],
    )?;
    removed += conn.execute(
        "DELETE FROM thread_channel_endpoints WHERE thread_id = ?1",
        params![thread_id],
    )?;
    removed += conn.execute(
        "DELETE FROM thread_message_routes WHERE thread_id = ?1",
        params![thread_id],
    )?;
    Ok(removed)
}

fn upsert_thread_meta(
    tx: &Transaction<'_>,
    meta: &ThreadMetaDraft,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let thread_id = normalize_thread_id(&meta.thread_id)?;
    let workspace_dir = normalize_optional(meta.workspace_dir.as_deref());
    let thread_type =
        normalize_optional(Some(&meta.thread_type)).unwrap_or_else(|| "chat".to_owned());
    let thread_label = normalize_optional(meta.thread_label.as_deref());
    let agent_id = normalize_optional(meta.agent_id.as_deref());
    let provider_type = normalize_optional(meta.provider_type.as_deref());
    let created_at = normalize_optional(meta.created_at.as_deref());
    let updated_at = normalize_optional(meta.updated_at.as_deref());
    let message_count = i64::from(meta.message_count);
    let last_user_message = normalize_optional(meta.last_user_message.as_deref());
    let last_assistant_message = normalize_optional(meta.last_assistant_message.as_deref());
    let last_message_preview = normalize_optional(meta.last_message_preview.as_deref());
    let recent_run_id = normalize_optional(meta.recent_run_id.as_deref());
    let active_run_id = normalize_optional(meta.active_run_id.as_deref());
    let worktree_json = normalize_optional(meta.worktree_json.as_deref());
    let last_delivery_context_json = normalize_optional(meta.last_delivery_context_json.as_deref());
    let last_delivery_updated_at = normalize_optional(meta.last_delivery_updated_at.as_deref());
    let default_list_hidden = if meta.default_list_hidden { 1 } else { 0 };
    let provider_key = normalize_optional(meta.provider_key.as_deref());
    let selected_model = normalize_optional(meta.selected_model.as_deref());
    let selected_model_reasoning_effort =
        normalize_optional(meta.selected_model_reasoning_effort.as_deref());
    let selected_model_service_tier =
        normalize_optional(meta.selected_model_service_tier.as_deref());
    let sdk_session_id = normalize_optional(meta.sdk_session_id.as_deref());

    tx.execute(
        "INSERT INTO thread_meta (
            thread_id, workspace_dir, thread_type, thread_label, agent_id, provider_type,
            created_at, updated_at, message_count, last_user_message, last_assistant_message,
            last_message_preview, recent_run_id, active_run_id, worktree_json,
            last_delivery_context_json, last_delivery_updated_at, default_list_hidden,
            provider_key, selected_model, selected_model_reasoning_effort,
            selected_model_service_tier, sdk_session_id,
            projection_version, projected_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)
         ON CONFLICT(thread_id) DO UPDATE SET
            workspace_dir = excluded.workspace_dir,
            thread_type = excluded.thread_type,
            thread_label = excluded.thread_label,
            agent_id = excluded.agent_id,
            provider_type = excluded.provider_type,
            provider_key = excluded.provider_key,
            selected_model = excluded.selected_model,
            selected_model_reasoning_effort = excluded.selected_model_reasoning_effort,
            selected_model_service_tier = excluded.selected_model_service_tier,
            sdk_session_id = excluded.sdk_session_id,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            message_count = excluded.message_count,
            last_user_message = excluded.last_user_message,
            last_assistant_message = excluded.last_assistant_message,
            last_message_preview = excluded.last_message_preview,
            recent_run_id = excluded.recent_run_id,
            active_run_id = excluded.active_run_id,
            worktree_json = excluded.worktree_json,
            last_delivery_context_json = excluded.last_delivery_context_json,
            last_delivery_updated_at = excluded.last_delivery_updated_at,
            default_list_hidden = excluded.default_list_hidden,
            projection_version = excluded.projection_version,
            projected_at = excluded.projected_at",
        params![
            thread_id,
            workspace_dir,
            thread_type,
            thread_label,
            agent_id,
            provider_type,
            created_at,
            updated_at,
            message_count,
            last_user_message,
            last_assistant_message,
            last_message_preview,
            recent_run_id,
            active_run_id,
            worktree_json,
            last_delivery_context_json,
            last_delivery_updated_at,
            default_list_hidden,
            provider_key,
            selected_model,
            selected_model_reasoning_effort,
            selected_model_service_tier,
            sdk_session_id,
            CURRENT_THREAD_META_PROJECTION_VERSION,
            recorded_at,
        ],
    )?;
    Ok(())
}

fn upsert_thread_channel_endpoint(
    tx: &Transaction<'_>,
    endpoint: &KnownChannelEndpoint,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let endpoint_key = normalize_required("endpoint_key", &endpoint.endpoint_key)?;
    let channel = normalize_required("channel", &endpoint.channel)?;
    let account_id = normalize_required("account_id", &endpoint.account_id)?;
    let binding_key = endpoint.binding_key.trim().to_owned();
    let chat_id = endpoint.chat_id.trim().to_owned();
    let delivery_target_type = normalize_optional(Some(&endpoint.delivery_target_type))
        .unwrap_or_else(|| "chat_id".to_owned());
    let delivery_target_id = endpoint.delivery_target_id.trim().to_owned();
    let display_label = endpoint.display_label.trim().to_owned();
    let thread_id = normalize_optional(endpoint.thread_id.as_deref());
    let thread_label = normalize_optional(endpoint.thread_label.as_deref());
    let workspace_dir = normalize_optional(endpoint.workspace_dir.as_deref());
    let thread_updated_at = normalize_optional(endpoint.thread_updated_at.as_deref());
    let last_inbound_at = normalize_optional(endpoint.last_inbound_at.as_deref());
    let last_delivery_at = normalize_optional(endpoint.last_delivery_at.as_deref());

    tx.execute(
        "INSERT INTO thread_channel_endpoints (
            endpoint_key, channel, account_id, binding_key, chat_id,
            delivery_target_type, delivery_target_id, display_label,
            thread_id, thread_label, workspace_dir, thread_updated_at,
            last_inbound_at, last_delivery_at, projected_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(endpoint_key) DO UPDATE SET
            channel = excluded.channel,
            account_id = excluded.account_id,
            binding_key = excluded.binding_key,
            chat_id = excluded.chat_id,
            delivery_target_type = excluded.delivery_target_type,
            delivery_target_id = excluded.delivery_target_id,
            display_label = excluded.display_label,
            thread_id = excluded.thread_id,
            thread_label = excluded.thread_label,
            workspace_dir = excluded.workspace_dir,
            thread_updated_at = excluded.thread_updated_at,
            last_inbound_at = excluded.last_inbound_at,
            last_delivery_at = excluded.last_delivery_at,
            projected_at = excluded.projected_at",
        params![
            endpoint_key,
            channel,
            account_id,
            binding_key,
            chat_id,
            delivery_target_type,
            delivery_target_id,
            display_label,
            thread_id,
            thread_label,
            workspace_dir,
            thread_updated_at,
            last_inbound_at,
            last_delivery_at,
            recorded_at,
        ],
    )?;
    Ok(())
}

fn upsert_thread_message_route(
    tx: &Transaction<'_>,
    route: &ThreadMessageRouteDraft,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let thread_id = normalize_thread_id(&route.thread_id)?;
    let channel = normalize_required("channel", &route.channel)?;
    let account_id = route.account_id.trim().to_owned();
    let chat_id = route.chat_id.trim().to_owned();
    let thread_binding_key = route
        .thread_binding_key
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    let message_id = normalize_required("message_id", &route.message_id)?;

    tx.execute(
        "INSERT INTO thread_message_routes (
            channel, account_id, chat_id, thread_binding_key, message_id, thread_id, projected_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(channel, account_id, chat_id, thread_binding_key, message_id)
         DO UPDATE SET
            thread_id = excluded.thread_id,
            projected_at = excluded.projected_at",
        params![
            channel,
            account_id,
            chat_id,
            thread_binding_key,
            message_id,
            thread_id,
            recorded_at,
        ],
    )?;
    Ok(())
}

fn normalize_automation_thread_run_mode(value: &str) -> GaryxDbResult<String> {
    let mode = normalize_required("mode", value)?;
    match mode.as_str() {
        "generated_thread" | "target_thread" => Ok(mode),
        _ => Err(GaryxDbError::BadRequest(
            "mode must be generated_thread or target_thread".to_owned(),
        )),
    }
}

fn normalize_workflow_run_status(value: &str) -> GaryxDbResult<String> {
    let status = normalize_required("status", value)?;
    match status.as_str() {
        "planning" | "queued" | "running" | "succeeded" | "failed" | "cancelled" => Ok(status),
        _ => Err(GaryxDbError::BadRequest(
            "status must be planning, queued, running, succeeded, failed, or cancelled".to_owned(),
        )),
    }
}

fn is_terminal_workflow_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

fn normalize_workflow_child_status(value: &str) -> GaryxDbResult<String> {
    let status = normalize_required("status", value)?;
    match status.as_str() {
        "queued" | "running" | "succeeded" | "failed" | "cancelled" | "skipped" => Ok(status),
        _ => Err(GaryxDbError::BadRequest(
            "child status must be queued, running, succeeded, failed, cancelled, or skipped"
                .to_owned(),
        )),
    }
}

fn is_terminal_workflow_child_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled" | "skipped")
}

fn normalize_workflow_result_mode(value: &str) -> GaryxDbResult<String> {
    let mode = normalize_required("result_mode", value)?;
    match mode.as_str() {
        "text" | "structured" => Ok(mode),
        _ => Err(GaryxDbError::BadRequest(
            "result_mode must be text or structured".to_owned(),
        )),
    }
}

fn normalize_workspace_path(path: &str) -> GaryxDbResult<String> {
    let normalized = normalize_required("workspace path", path)?.replace('\\', "/");
    if !is_absolute_workspace_path(&normalized) {
        return Err(GaryxDbError::BadRequest(
            "workspace path must be absolute".to_owned(),
        ));
    }
    Ok(normalized)
}

fn normalize_capsule_id(id: &str) -> GaryxDbResult<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(GaryxDbError::BadRequest(
            "capsule id must not be empty".to_owned(),
        ));
    }
    Uuid::parse_str(trimmed)
        .map(|uuid| uuid.to_string())
        .map_err(|_| GaryxDbError::BadRequest("capsule id must be a UUID".to_owned()))
}

fn normalize_capsule_text(value: &str) -> String {
    value.trim().to_owned()
}

fn normalize_capsule_sha256(value: &str) -> GaryxDbResult<String> {
    let trimmed = normalize_required("html_sha256", value)?.to_ascii_lowercase();
    if trimmed.len() != 64 || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GaryxDbError::BadRequest(
            "html_sha256 must be 64 hex characters".to_owned(),
        ));
    }
    Ok(trimmed)
}

fn normalize_capsule_byte_size(value: i64) -> GaryxDbResult<i64> {
    if value < 0 {
        return Err(GaryxDbError::BadRequest(
            "byte_size must be non-negative".to_owned(),
        ));
    }
    Ok(value)
}

fn is_absolute_workspace_path(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with("//") {
        return true;
    }
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn ensure_workspaces_deleted_at_column(conn: &Connection) -> GaryxDbResult<()> {
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

fn ensure_thread_meta_projection_columns(conn: &Connection) -> GaryxDbResult<()> {
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
    if !columns.contains("projection_version") {
        conn.execute(
            "ALTER TABLE thread_meta
             ADD COLUMN projection_version INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

fn ensure_thread_channel_endpoint_columns(conn: &Connection) -> GaryxDbResult<()> {
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

fn ensure_workflow_runs_task_columns(conn: &Connection) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(workflow_runs)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = BTreeSet::new();
    for row in rows {
        columns.insert(row?);
    }
    for (name, sql_type) in [
        ("task_id", "TEXT"),
        ("task_thread_id", "TEXT"),
        ("workflow_definition_id", "TEXT"),
        ("workflow_definition_version", "INTEGER"),
        ("workflow_definition_snapshot_json", "TEXT"),
        ("input_json", "TEXT"),
        ("output_text", "TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE workflow_runs ADD COLUMN {name} {sql_type}"),
                [],
            )?;
        }
    }
    Ok(())
}

fn workspace_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    Ok(WorkspaceRecord {
        name: row.get(0)?,
        path: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

fn workspace_by_path(conn: &Connection, path: &str) -> GaryxDbResult<Option<WorkspaceRecord>> {
    Ok(conn
        .query_row(
            "SELECT name, path, created_at, updated_at FROM workspaces WHERE path = ?1",
            params![path],
            workspace_from_row,
        )
        .optional()?)
}

fn capsule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CapsuleRecord> {
    Ok(CapsuleRecord {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        thread_id: row.get(3)?,
        run_id: row.get(4)?,
        agent_id: row.get(5)?,
        provider_type: row.get(6)?,
        html_sha256: row.get(7)?,
        byte_size: row.get(8)?,
        revision: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn capsule_by_id(conn: &Connection, id: &str) -> GaryxDbResult<Option<CapsuleRecord>> {
    Ok(conn
        .query_row(
            "SELECT id, title, description, thread_id, run_id, agent_id, provider_type,
                    html_sha256, byte_size, revision, created_at, updated_at
             FROM capsules
             WHERE id = ?1",
            params![id],
            capsule_from_row,
        )
        .optional()?)
}

fn workflow_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRunRecord> {
    Ok(WorkflowRunRecord {
        workflow_id: row.get(0)?,
        task_id: row.get(1)?,
        task_thread_id: row.get(2)?,
        workflow_definition_id: row.get(3)?,
        workflow_definition_version: row
            .get::<_, Option<i64>>(4)?
            .and_then(|value| u64::try_from(value).ok()),
        workflow_definition_snapshot_json: row.get(5)?,
        input_json: row.get(6)?,
        parent_thread_id: row.get(7)?,
        parent_run_id: row.get(8)?,
        name: row.get(9)?,
        description: row.get(10)?,
        status: row.get(11)?,
        current_phase_index: row.get(12)?,
        script_text: row.get(13)?,
        meta_json: row.get(14)?,
        result_json: row.get(15)?,
        output_text: row.get(16)?,
        error: row.get(17)?,
        workspace_dir: row.get(18)?,
        created_by: row.get(19)?,
        total_children: row.get(20)?,
        completed_children: row.get(21)?,
        failed_children: row.get(22)?,
        total_input_tokens: row.get(23)?,
        total_output_tokens: row.get(24)?,
        total_tool_calls: row.get(25)?,
        total_cost_usd: row.get(26)?,
        created_at: row.get(27)?,
        started_at: row.get(28)?,
        finished_at: row.get(29)?,
        updated_at: row.get(30)?,
    })
}

fn workflow_run_by_id(
    conn: &Connection,
    workflow_run_id: &str,
) -> GaryxDbResult<Option<WorkflowRunRecord>> {
    Ok(conn
        .query_row(
            "SELECT workflow_id, task_id, task_thread_id, workflow_definition_id,
                    workflow_definition_version, workflow_definition_snapshot_json, input_json,
                    parent_thread_id, parent_run_id, name, description, status,
                    current_phase_index, script_text, meta_json, result_json, output_text, error,
                    workspace_dir, created_by, total_children, completed_children, failed_children,
                    total_input_tokens, total_output_tokens, total_tool_calls, total_cost_usd,
                    created_at, started_at, finished_at, updated_at
             FROM workflow_runs
             WHERE workflow_id = ?1",
            params![workflow_run_id],
            workflow_run_from_row,
        )
        .optional()?)
}

fn workflow_child_run_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkflowChildRunRecord> {
    Ok(WorkflowChildRunRecord {
        workflow_id: row.get(0)?,
        workflow_child_run_id: row.get(1)?,
        thread_id: row.get(2)?,
        phase_index: row.get(3)?,
        phase_title: row.get(4)?,
        label: row.get(5)?,
        agent_id: row.get(6)?,
        status: row.get(7)?,
        prompt: row.get(8)?,
        result_mode: row.get(9)?,
        schema_json: row.get(10)?,
        result_text: row.get(11)?,
        result_json: row.get(12)?,
        result_preview: row.get(13)?,
        error: row.get(14)?,
        input_tokens: row.get(15)?,
        output_tokens: row.get(16)?,
        tool_calls: row.get(17)?,
        cost_usd: row.get(18)?,
        queued_at: row.get(19)?,
        started_at: row.get(20)?,
        finished_at: row.get(21)?,
        updated_at: row.get(22)?,
    })
}

fn workflow_child_run_by_id(
    conn: &Connection,
    workflow_id: &str,
    workflow_child_run_id: &str,
) -> GaryxDbResult<Option<WorkflowChildRunRecord>> {
    Ok(conn
        .query_row(
            "SELECT workflow_id, workflow_child_run_id, thread_id, phase_index, phase_title,
                    label, agent_id, status, prompt, result_mode, schema_json, result_text,
                    result_json, result_preview, error, input_tokens, output_tokens, tool_calls,
                    cost_usd, queued_at, started_at, finished_at, updated_at
             FROM workflow_child_runs
             WHERE workflow_id = ?1 AND workflow_child_run_id = ?2",
            params![workflow_id, workflow_child_run_id],
            workflow_child_run_from_row,
        )
        .optional()?)
}

fn workflow_child_runs_for_workflow(
    conn: &Connection,
    workflow_id: &str,
) -> GaryxDbResult<Vec<WorkflowChildRunRecord>> {
    let mut stmt = conn.prepare(
        "SELECT workflow_id, workflow_child_run_id, thread_id, phase_index, phase_title,
                label, agent_id, status, prompt, result_mode, schema_json, result_text,
                result_json, result_preview, error, input_tokens, output_tokens, tool_calls,
                cost_usd, queued_at, started_at, finished_at, updated_at
         FROM workflow_child_runs
         WHERE workflow_id = ?1
         ORDER BY phase_index ASC, queued_at ASC, workflow_child_run_id ASC",
    )?;
    let rows = stmt.query_map(params![workflow_id], workflow_child_run_from_row)?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

fn workflow_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowEventRecord> {
    Ok(WorkflowEventRecord {
        event_seq: row.get(0)?,
        event_id: row.get(1)?,
        workflow_id: row.get(2)?,
        workflow_child_run_id: row.get(3)?,
        thread_id: row.get(4)?,
        event_type: row.get(5)?,
        payload_json: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn workflow_events_after_for_workflow(
    conn: &Connection,
    workflow_id: &str,
    after_event_seq: i64,
    limit: i64,
) -> GaryxDbResult<Vec<WorkflowEventRecord>> {
    let mut stmt = conn.prepare(
        "SELECT event_seq, event_id, workflow_id, workflow_child_run_id, thread_id,
                event_type, payload_json, created_at
         FROM workflow_events
         WHERE workflow_id = ?1 AND event_seq > ?2
         ORDER BY event_seq ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![workflow_id, after_event_seq, limit],
        workflow_event_from_row,
    )?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

fn latest_workflow_event_seq(conn: &Connection, workflow_id: &str) -> GaryxDbResult<u64> {
    let seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(event_seq), 0)
         FROM workflow_events
         WHERE workflow_id = ?1",
        params![workflow_id],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(seq).unwrap_or(0))
}

fn workflow_phase_index_from_payload(payload_json: &str) -> Option<i64> {
    let value = serde_json::from_str::<Value>(payload_json).ok()?;
    let index = value
        .get("phaseIndex")
        .or_else(|| value.get("phase_index"))?
        .as_i64()?;
    (index >= 0).then_some(index)
}

fn workflow_event_by_id(
    conn: &Connection,
    event_id: &str,
) -> GaryxDbResult<Option<WorkflowEventRecord>> {
    Ok(conn
        .query_row(
            "SELECT event_seq, event_id, workflow_id, workflow_child_run_id, thread_id,
                    event_type, payload_json, created_at
             FROM workflow_events
             WHERE event_id = ?1",
            params![event_id],
            workflow_event_from_row,
        )
        .optional()?)
}

fn automation_thread_run_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AutomationThreadRunRecord> {
    Ok(AutomationThreadRunRecord {
        automation_id: row.get(0)?,
        run_id: row.get(1)?,
        thread_id: row.get(2)?,
        workspace_dir: row.get(3)?,
        agent_id: row.get(4)?,
        automation_label_snapshot: row.get(5)?,
        mode: row.get(6)?,
        status: row.get(7)?,
        started_at: row.get(8)?,
        finished_at: row.get(9)?,
        recorded_at: row.get(10)?,
    })
}

fn automation_thread_run_by_key(
    conn: &Connection,
    automation_id: &str,
    run_id: &str,
) -> GaryxDbResult<Option<AutomationThreadRunRecord>> {
    Ok(conn
        .query_row(
            "SELECT automation_id, run_id, thread_id, workspace_dir, agent_id,
                    automation_label_snapshot, mode, status, started_at, finished_at, recorded_at
             FROM automation_thread_runs
             WHERE automation_id = ?1 AND run_id = ?2",
            params![automation_id, run_id],
            automation_thread_run_from_row,
        )
        .optional()?)
}

fn attach_dream_spans(conn: &Connection, topics: &mut [DreamTopicRecord]) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare(
        "SELECT span_id, dream_id, thread_id, workspace_dir, start_seq, end_seq,
                start_at, end_at, excerpt, message_count
         FROM dream_spans
         WHERE dream_id = ?1
         ORDER BY start_at ASC, thread_id ASC, start_seq ASC",
    )?;
    for topic in topics {
        let rows = stmt.query_map(params![topic.dream_id], |row| {
            Ok(DreamSpanRecord {
                span_id: row.get(0)?,
                dream_id: row.get(1)?,
                thread_id: row.get(2)?,
                workspace_dir: row.get(3)?,
                start_seq: row.get(4)?,
                end_seq: row.get(5)?,
                start_at: row.get(6)?,
                end_at: row.get(7)?,
                excerpt: row.get(8)?,
                message_count: row.get(9)?,
            })
        })?;
        let mut spans = Vec::new();
        for row in rows {
            spans.push(row?);
        }
        topic.span_count = spans.len() as u32;
        topic.spans = spans;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_configures_wal_normal_synchronous_and_busy_timeout() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");

        let service = GaryxDbService::open(&path).expect("db opens");
        {
            let conn = service.conn().expect("conn");
            let journal_mode: String = conn
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .expect("journal_mode");
            assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
            let synchronous: i64 = conn
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .expect("synchronous");
            assert_eq!(synchronous, 1, "synchronous should be NORMAL (1)");
            let busy_timeout: i64 = conn
                .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                .expect("busy_timeout");
            assert_eq!(busy_timeout, BUSY_TIMEOUT.as_millis() as i64);
        }
        drop(service);

        // WAL is a persistent database property: a reopen must still be WAL.
        let reopened = GaryxDbService::open(&path).expect("db reopens");
        let conn = reopened.conn().expect("conn");
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    }

    #[test]
    fn memory_db_still_works_without_wal() {
        let service = GaryxDbService::memory().expect("memory db");
        service.pin_thread("thread::mem-check").expect("pin");
        let pins = service.list_pinned_threads().expect("list");
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].thread_id, "thread::mem-check");
    }

    #[tokio::test]
    async fn run_blocking_round_trips_reads_and_writes() {
        let service = std::sync::Arc::new(GaryxDbService::memory().expect("memory db"));

        let pinned = service
            .run_blocking(|db| db.pin_thread("thread::async-entry"))
            .await
            .expect("async pin");
        assert_eq!(pinned.thread_id, "thread::async-entry");

        let pins = service
            .run_blocking(|db| db.list_pinned_threads())
            .await
            .expect("async list");
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].thread_id, "thread::async-entry");
    }

    #[test]
    fn opening_legacy_workflow_runs_db_adds_task_columns_before_indexes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        {
            let conn = Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                r#"
                CREATE TABLE workflow_runs (
                    workflow_id TEXT PRIMARY KEY,
                    parent_thread_id TEXT NOT NULL,
                    parent_run_id TEXT,
                    name TEXT NOT NULL,
                    description TEXT,
                    status TEXT NOT NULL CHECK (
                        status IN ('planning','queued','running','succeeded','failed','cancelled')
                    ),
                    current_phase_index INTEGER,
                    script_text TEXT NOT NULL,
                    meta_json TEXT NOT NULL,
                    result_json TEXT,
                    error TEXT,
                    workspace_dir TEXT,
                    created_by TEXT,
                    total_children INTEGER NOT NULL DEFAULT 0,
                    completed_children INTEGER NOT NULL DEFAULT 0,
                    failed_children INTEGER NOT NULL DEFAULT 0,
                    total_input_tokens INTEGER NOT NULL DEFAULT 0,
                    total_output_tokens INTEGER NOT NULL DEFAULT 0,
                    total_tool_calls INTEGER NOT NULL DEFAULT 0,
                    total_cost_usd REAL NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL,
                    started_at TEXT,
                    finished_at TEXT,
                    updated_at TEXT NOT NULL
                ) STRICT;
                "#,
            )
            .expect("legacy workflow_runs");
        }

        let db = GaryxDbService::open(&path).expect("open migrated db");
        assert!(
            db.get_workflow_run("thread::missing")
                .expect("query migrated workflow table")
                .is_none()
        );
    }

    #[test]
    fn opening_legacy_thread_meta_db_adds_projection_columns() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        {
            let conn = Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                r#"
                CREATE TABLE thread_meta (
                    thread_id TEXT PRIMARY KEY,
                    workspace_dir TEXT,
                    thread_type TEXT NOT NULL DEFAULT 'chat',
                    thread_label TEXT,
                    agent_id TEXT,
                    provider_type TEXT,
                    updated_at TEXT,
                    last_delivery_context_json TEXT,
                    last_delivery_updated_at TEXT,
                    default_list_hidden INTEGER NOT NULL DEFAULT 0,
                    projection_version INTEGER NOT NULL DEFAULT 2,
                    projected_at TEXT NOT NULL
                ) STRICT;

                INSERT INTO thread_meta (
                    thread_id, workspace_dir, thread_type, thread_label, agent_id,
                    provider_type, updated_at, default_list_hidden, projection_version,
                    projected_at
                ) VALUES (
                    'thread::legacy', '/workspace/legacy', 'chat', 'Legacy Thread',
                    'claude', 'claude_code', '2026-06-03T00:00:00.000Z',
                    0, 2, '2026-06-03T00:00:01.000Z'
                );
                "#,
            )
            .expect("legacy thread_meta");
        }

        let db = GaryxDbService::open(&path).expect("open migrated db");

        assert!(
            db.thread_meta_projection_needs_backfill()
                .expect("legacy projection needs backfill")
        );
        let rows = db.list_thread_meta().expect("list legacy meta");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].thread_id, "thread::legacy");
        assert_eq!(rows[0].created_at, None);
        assert_eq!(rows[0].message_count, 0);
        assert_eq!(rows[0].last_message_preview, None);
        assert_eq!(rows[0].projection_version, 2);
    }

    #[test]
    fn opening_legacy_thread_channel_endpoint_db_adds_thread_columns() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        {
            let conn = Connection::open(&path).expect("legacy db");
            conn.execute_batch(
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
                    projected_at TEXT NOT NULL
                ) STRICT;

                INSERT INTO thread_channel_endpoints (
                    endpoint_key, channel, account_id, binding_key, chat_id,
                    delivery_target_type, delivery_target_id, display_label,
                    projected_at
                ) VALUES (
                    'telegram::main::1000000001', 'telegram', 'main', '1000000001',
                    '1000000001', 'chat_id', '1000000001', 'Test User',
                    '2026-06-03T00:00:01.000Z'
                );
                "#,
            )
            .expect("legacy thread_channel_endpoints");
        }

        let db = GaryxDbService::open(&path).expect("open migrated db");
        let endpoints = db
            .list_thread_channel_endpoints()
            .expect("list migrated endpoints");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].endpoint_key, "telegram::main::1000000001");
        assert_eq!(endpoints[0].thread_id, None);

        db.sync_thread_meta_projection_snapshot(ThreadMetaProjectionSnapshot {
            thread_meta: vec![ThreadMetaDraft {
                thread_id: "thread::bound".to_owned(),
                thread_label: Some("Bound".to_owned()),
                workspace_dir: Some("/Users/test/project".to_owned()),
                ..Default::default()
            }],
            channel_endpoints: vec![KnownChannelEndpoint {
                endpoint_key: "telegram::main::1000000001".to_owned(),
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                binding_key: "1000000001".to_owned(),
                chat_id: "1000000001".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "1000000001".to_owned(),
                display_label: "Test User".to_owned(),
                thread_id: Some("thread::bound".to_owned()),
                thread_label: Some("Bound".to_owned()),
                workspace_dir: Some("/Users/test/project".to_owned()),
                thread_updated_at: Some("2026-06-03T00:00:02.000Z".to_owned()),
                last_inbound_at: None,
                last_delivery_at: None,
            }],
            message_routes: Vec::new(),
        })
        .expect("write migrated endpoint projection");

        let endpoints = db
            .list_thread_channel_endpoints()
            .expect("list updated endpoints");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].thread_id.as_deref(), Some("thread::bound"));
        assert_eq!(
            endpoints[0].workspace_dir.as_deref(),
            Some("/Users/test/project")
        );
    }

    #[test]
    fn thread_pins_round_trip_in_recency_order() {
        use std::time::Duration;

        let db = GaryxDbService::memory().expect("db opens");
        db.pin_thread("thread::older").expect("pin older");
        std::thread::sleep(Duration::from_millis(2));
        db.pin_thread("thread::newer").expect("pin newer");
        std::thread::sleep(Duration::from_millis(2));
        db.pin_thread("thread::older").expect("repin older");

        let records = db.list_pinned_threads().expect("list pins");
        assert_eq!(
            records
                .iter()
                .map(|record| record.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::older", "thread::newer"],
        );

        assert!(db.unpin_thread("thread::older").expect("unpin older"));
        assert!(!db.unpin_thread("thread::older").expect("unpin older again"));
        assert_eq!(
            db.list_pinned_threads()
                .expect("list remaining")
                .into_iter()
                .map(|record| record.thread_id)
                .collect::<Vec<_>>(),
            vec!["thread::newer"],
        );
    }

    #[test]
    fn empty_thread_id_is_rejected() {
        let db = GaryxDbService::memory().expect("db opens");
        assert!(matches!(
            db.pin_thread("   "),
            Err(GaryxDbError::BadRequest(_))
        ));
    }

    #[test]
    fn workflow_runs_children_and_events_round_trip() {
        let db = GaryxDbService::memory().expect("db opens");
        let workflow = db
            .create_workflow_run(WorkflowRunDraft {
                task_id: Some("#TASK-123".to_owned()),
                task_thread_id: Some("thread::task".to_owned()),
                workflow_definition_id: Some("definition".to_owned()),
                workflow_definition_version: Some(2),
                workflow_definition_snapshot_json: Some(
                    r#"{"workflowId":"definition","version":2}"#.to_owned(),
                ),
                input_json: Some(r#"{"query":"test input"}"#.to_owned()),
                workflow_id: Some("test".to_owned()),
                parent_thread_id: "thread::parent".to_owned(),
                parent_run_id: Some("run::parent".to_owned()),
                name: "Test Workflow".to_owned(),
                description: Some("Check storage".to_owned()),
                status: "running".to_owned(),
                current_phase_index: Some(0),
                script_text: "export const meta = {}".to_owned(),
                meta_json: r#"{"name":"Test Workflow"}"#.to_owned(),
                result_json: None,
                output_text: Some("Workflow output".to_owned()),
                error: None,
                workspace_dir: Some("/Users/test/project".to_owned()),
                created_by: Some("test".to_owned()),
                started_at: Some("2026-05-29T01:00:00.000Z".to_owned()),
                finished_at: None,
            })
            .expect("create workflow");
        assert_eq!(workflow.workflow_id, "test");
        assert_eq!(workflow.status, "running");
        assert_eq!(workflow.task_id.as_deref(), Some("#TASK-123"));
        assert_eq!(
            workflow.workflow_definition_id.as_deref(),
            Some("definition")
        );
        assert_eq!(
            workflow.input_json.as_deref(),
            Some(r#"{"query":"test input"}"#)
        );
        assert_eq!(workflow.output_text.as_deref(), Some("Workflow output"));

        let child = db
            .upsert_workflow_child_run(WorkflowChildRunDraft {
                workflow_id: workflow.workflow_id.clone(),
                workflow_child_run_id: Some("workflow-child::one".to_owned()),
                thread_id: "thread::child".to_owned(),
                phase_index: 0,
                phase_title: "Inspect".to_owned(),
                label: "inspect:ui".to_owned(),
                agent_id: Some("claude".to_owned()),
                status: "running".to_owned(),
                prompt: "Inspect UI".to_owned(),
                result_mode: "structured".to_owned(),
                schema_json: Some(r#"{"type":"object"}"#.to_owned()),
                result_text: None,
                result_json: None,
                result_preview: None,
                error: None,
                input_tokens: 10,
                output_tokens: 5,
                tool_calls: 2,
                cost_usd: 0.01,
                started_at: Some("2026-05-29T01:00:01.000Z".to_owned()),
                finished_at: None,
            })
            .expect("upsert child");
        assert_eq!(child.workflow_child_run_id, "workflow-child::one");
        assert_eq!(child.result_mode, "structured");

        assert!(
            db.finish_workflow_child_run(
                &workflow.workflow_id,
                "workflow-child::one",
                "succeeded",
                None,
                Some(r#"{"ok":true}"#),
                Some("ok"),
                None,
                Some(WorkflowChildRunUsage {
                    input_tokens: 12,
                    output_tokens: 6,
                    tool_calls: 3,
                    cost_usd: 0.02,
                }),
            )
            .expect("finish child")
        );
        let children = db
            .list_workflow_child_runs(&workflow.workflow_id)
            .expect("list children");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].status, "succeeded");
        assert_eq!(children[0].result_json.as_deref(), Some(r#"{"ok":true}"#));
        assert_eq!(children[0].input_tokens, 12);
        assert_eq!(children[0].output_tokens, 6);
        assert_eq!(children[0].tool_calls, 3);
        assert_eq!(children[0].cost_usd, 0.02);
        let refreshed = db
            .get_workflow_run(&workflow.workflow_id)
            .expect("get workflow")
            .expect("workflow exists");
        assert_eq!(refreshed.total_input_tokens, 12);
        assert_eq!(refreshed.total_output_tokens, 6);
        assert_eq!(refreshed.total_tool_calls, 3);
        assert_eq!(refreshed.total_cost_usd, 0.02);
        db.append_workflow_event(WorkflowEventDraft {
            event_id: Some("workflow-event::one".to_owned()),
            workflow_id: workflow.workflow_id.clone(),
            workflow_child_run_id: Some("workflow-child::one".to_owned()),
            thread_id: Some("thread::child".to_owned()),
            event_type: "workflow.child_succeeded".to_owned(),
            payload_json: r#"{"preview":"ok"}"#.to_owned(),
        })
        .expect("append event");
        let events = db
            .list_workflow_events_after(&workflow.workflow_id, 0, 10)
            .expect("list events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "workflow.child_succeeded");
    }

    #[test]
    fn workflow_events_use_monotonic_seq_cursor() {
        let db = GaryxDbService::memory().expect("db opens");
        db.create_workflow_run(WorkflowRunDraft {
            task_id: None,
            task_thread_id: None,
            workflow_definition_id: None,
            workflow_definition_version: None,
            workflow_definition_snapshot_json: None,
            input_json: None,
            workflow_id: Some("cursor".to_owned()),
            parent_thread_id: "thread::parent".to_owned(),
            parent_run_id: None,
            name: "Cursor".to_owned(),
            description: None,
            status: "running".to_owned(),
            current_phase_index: None,
            script_text: "return {}".to_owned(),
            meta_json: "{}".to_owned(),
            result_json: None,
            output_text: None,
            error: None,
            workspace_dir: None,
            created_by: None,
            started_at: None,
            finished_at: None,
        })
        .expect("create workflow");

        let first = db
            .append_workflow_event(WorkflowEventDraft {
                event_id: Some("workflow-event::first".to_owned()),
                workflow_id: "cursor".to_owned(),
                workflow_child_run_id: None,
                thread_id: None,
                event_type: "workflow.created".to_owned(),
                payload_json: "{}".to_owned(),
            })
            .expect("first event");
        let second = db
            .append_workflow_event(WorkflowEventDraft {
                event_id: Some("workflow-event::second".to_owned()),
                workflow_id: "cursor".to_owned(),
                workflow_child_run_id: None,
                thread_id: None,
                event_type: "workflow.phase_started".to_owned(),
                payload_json: r#"{"phaseIndex":2,"title":"Review"}"#.to_owned(),
            })
            .expect("second event");

        assert!(second.event_seq > first.event_seq);
        let workflow = db
            .get_workflow_run("cursor")
            .expect("get workflow")
            .expect("workflow exists");
        assert_eq!(workflow.current_phase_index, Some(2));
        let snapshot = db
            .get_workflow_run_drilldown_snapshot("cursor", 0, 10)
            .expect("snapshot")
            .expect("workflow snapshot");
        assert_eq!(snapshot.latest_event_seq, second.event_seq);
        assert_eq!(snapshot.events.len(), 2);
        let after_first = db
            .list_workflow_events_after("cursor", first.event_seq, 10)
            .expect("events after first");
        assert_eq!(
            after_first
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["workflow-event::second"],
        );
    }

    #[test]
    fn workflow_terminal_status_is_immutable_and_cancel_marks_children() {
        let db = GaryxDbService::memory().expect("db opens");
        db.create_workflow_run(WorkflowRunDraft {
            task_id: None,
            task_thread_id: None,
            workflow_definition_id: None,
            workflow_definition_version: None,
            workflow_definition_snapshot_json: None,
            input_json: None,
            workflow_id: Some("cancel".to_owned()),
            parent_thread_id: "thread::parent".to_owned(),
            parent_run_id: None,
            name: "Cancel".to_owned(),
            description: None,
            status: "running".to_owned(),
            current_phase_index: None,
            script_text: "return {}".to_owned(),
            meta_json: "{}".to_owned(),
            result_json: None,
            output_text: None,
            error: None,
            workspace_dir: None,
            created_by: None,
            started_at: None,
            finished_at: None,
        })
        .expect("create workflow");
        db.upsert_workflow_child_run(WorkflowChildRunDraft {
            workflow_id: "cancel".to_owned(),
            workflow_child_run_id: Some("workflow-child::running".to_owned()),
            thread_id: "thread::child".to_owned(),
            phase_index: 0,
            phase_title: "Run".to_owned(),
            label: "run".to_owned(),
            agent_id: None,
            status: "running".to_owned(),
            prompt: "Run".to_owned(),
            result_mode: "text".to_owned(),
            schema_json: None,
            result_text: None,
            result_json: None,
            result_preview: None,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            tool_calls: 0,
            cost_usd: 0.0,
            started_at: None,
            finished_at: None,
        })
        .expect("create child");

        assert!(
            db.update_workflow_run_status(
                "cancel",
                "cancelled",
                None,
                None,
                Some("cancelled by user"),
            )
            .expect("cancel workflow")
        );
        assert_eq!(
            db.cancel_workflow_child_runs("cancel", "cancelled by user")
                .expect("cancel children"),
            1
        );
        assert!(
            !db.update_workflow_run_status(
                "cancel",
                "succeeded",
                Some(r#"{"late":true}"#),
                Some("late"),
                None,
            )
            .expect("late success ignored")
        );
        assert!(
            !db.finish_workflow_child_run(
                "cancel",
                "workflow-child::running",
                "succeeded",
                Some("late"),
                None,
                Some("late"),
                None,
                None,
            )
            .expect("late child success ignored")
        );

        let workflow = db
            .get_workflow_run("cancel")
            .expect("get workflow")
            .expect("workflow");
        assert_eq!(workflow.status, "cancelled");
        assert_eq!(workflow.error.as_deref(), Some("cancelled by user"));
        let child = db
            .list_workflow_child_runs("cancel")
            .expect("list children")
            .pop()
            .expect("child");
        assert_eq!(child.status, "cancelled");
        assert_eq!(child.error.as_deref(), Some("cancelled by user"));
    }

    #[test]
    fn workflow_restart_reconciliation_fails_non_terminal_rows() {
        let db = GaryxDbService::memory().expect("db opens");
        db.create_workflow_run(WorkflowRunDraft {
            task_id: None,
            task_thread_id: None,
            workflow_definition_id: None,
            workflow_definition_version: None,
            workflow_definition_snapshot_json: None,
            input_json: None,
            workflow_id: Some("restart".to_owned()),
            parent_thread_id: "thread::parent".to_owned(),
            parent_run_id: None,
            name: "Restart".to_owned(),
            description: None,
            status: "running".to_owned(),
            current_phase_index: None,
            script_text: "return {}".to_owned(),
            meta_json: "{}".to_owned(),
            result_json: None,
            output_text: None,
            error: None,
            workspace_dir: None,
            created_by: None,
            started_at: None,
            finished_at: None,
        })
        .expect("create workflow");
        db.upsert_workflow_child_run(WorkflowChildRunDraft {
            workflow_id: "restart".to_owned(),
            workflow_child_run_id: Some("workflow-child::running".to_owned()),
            thread_id: "thread::child".to_owned(),
            phase_index: 0,
            phase_title: "Run".to_owned(),
            label: "run".to_owned(),
            agent_id: None,
            status: "running".to_owned(),
            prompt: "Run".to_owned(),
            result_mode: "text".to_owned(),
            schema_json: None,
            result_text: None,
            result_json: None,
            result_preview: None,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            tool_calls: 0,
            cost_usd: 0.0,
            started_at: None,
            finished_at: None,
        })
        .expect("create child");

        let reconciled = db
            .reconcile_interrupted_workflows("gateway restarted", "9999-12-31T23:59:59.999Z")
            .expect("reconcile");
        assert_eq!(reconciled, 1);
        let workflow = db
            .get_workflow_run("restart")
            .expect("get workflow")
            .expect("workflow exists");
        assert_eq!(workflow.status, "failed");
        assert_eq!(workflow.error.as_deref(), Some("gateway restarted"));
        let child = db
            .list_workflow_child_runs("restart")
            .expect("list children")
            .pop()
            .expect("child exists");
        assert_eq!(child.status, "failed");
        assert_eq!(child.error.as_deref(), Some("gateway restarted"));
        let events = db
            .list_workflow_events_after("restart", 0, 10)
            .expect("list events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "workflow.failed");
    }

    #[test]
    fn workspaces_round_trip_in_app_state_db() {
        let db = GaryxDbService::memory().expect("db opens");
        let first = db
            .upsert_workspace(WorkspaceDraft {
                name: Some(" Repo B ".to_owned()),
                path: " /workspace/repo-b ".to_owned(),
            })
            .expect("upsert first");
        assert_eq!(first.name.as_deref(), Some("Repo B"));
        assert_eq!(first.path, "/workspace/repo-b");

        db.upsert_workspace(WorkspaceDraft {
            name: None,
            path: "/workspace/repo-a".to_owned(),
        })
        .expect("upsert second");
        let updated = db
            .upsert_workspace(WorkspaceDraft {
                name: Some("Repo A".to_owned()),
                path: "/workspace/repo-a".to_owned(),
            })
            .expect("update second");
        assert_eq!(updated.name.as_deref(), Some("Repo A"));

        let workspaces = db.list_workspaces().expect("list workspaces");
        assert_eq!(
            workspaces
                .iter()
                .map(|workspace| workspace.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/workspace/repo-a", "/workspace/repo-b"],
        );

        assert!(db.delete_workspace("/workspace/repo-a").expect("delete"));
        assert!(
            !db.delete_workspace("/workspace/repo-a")
                .expect("delete again")
        );
        assert_eq!(db.count_workspace_rows().expect("count rows"), 2);
        assert_eq!(
            db.list_workspaces()
                .expect("list remaining")
                .into_iter()
                .map(|workspace| workspace.path)
                .collect::<Vec<_>>(),
            vec!["/workspace/repo-b"],
        );
    }

    #[test]
    fn workspace_seed_only_runs_before_any_workspace_row_exists() {
        let db = GaryxDbService::memory().expect("db opens");
        assert!(
            db.seed_workspaces_if_empty(vec![WorkspaceDraft {
                name: None,
                path: "/workspace/from-config".to_owned(),
            }])
            .expect("seed initial")
        );
        assert!(
            !db.seed_workspaces_if_empty(vec![WorkspaceDraft {
                name: None,
                path: "/workspace/ignored".to_owned(),
            }])
            .expect("skip second seed")
        );
        assert_eq!(
            db.list_workspaces()
                .expect("list active")
                .into_iter()
                .map(|workspace| workspace.path)
                .collect::<Vec<_>>(),
            vec!["/workspace/from-config"],
        );

        assert!(
            db.delete_workspace("/workspace/from-config")
                .expect("soft delete")
        );
        assert_eq!(db.count_workspace_rows().expect("count tombstone"), 1);
        assert!(db.list_workspaces().expect("list after delete").is_empty());
        assert!(
            !db.seed_workspaces_if_empty(vec![WorkspaceDraft {
                name: None,
                path: "/workspace/from-config".to_owned(),
            }])
            .expect("tombstone prevents reseed")
        );
        assert!(db.list_workspaces().expect("list remains empty").is_empty());
    }

    #[test]
    fn empty_workspace_path_is_rejected() {
        let db = GaryxDbService::memory().expect("db opens");
        assert!(matches!(
            db.upsert_workspace(WorkspaceDraft {
                name: None,
                path: "   ".to_owned(),
            }),
            Err(GaryxDbError::BadRequest(_))
        ));
    }

    #[test]
    fn relative_workspace_path_is_rejected() {
        let db = GaryxDbService::memory().expect("db opens");
        assert!(matches!(
            db.upsert_workspace(WorkspaceDraft {
                name: None,
                path: "relative/project".to_owned(),
            }),
            Err(GaryxDbError::BadRequest(_))
        ));
    }

    fn capsule_draft(id: &str, title: &str, thread_id: &str) -> CapsuleCreateDraft {
        CapsuleCreateDraft {
            id: id.to_owned(),
            title: title.to_owned(),
            description: format!("{} description", title.trim()),
            thread_id: Some(thread_id.to_owned()),
            run_id: Some(format!("run::{title}")),
            agent_id: Some("agent::capsule".to_owned()),
            provider_type: Some("codex_app_server".to_owned()),
            html_sha256: "a".repeat(64),
            byte_size: 42,
        }
    }

    #[test]
    fn capsules_crud_create_update_get_list_delete() {
        let db = GaryxDbService::memory().expect("db opens");
        let id = Uuid::new_v4().to_string();
        let created = db
            .create_capsule(capsule_draft(&id, " Demo ", "thread::capsules"))
            .expect("create capsule");
        assert_eq!(created.id, id);
        assert_eq!(created.title, "Demo");
        assert_eq!(created.description, "Demo description");
        assert_eq!(created.thread_id.as_deref(), Some("thread::capsules"));
        assert_eq!(created.run_id.as_deref(), Some("run:: Demo"));
        assert_eq!(created.agent_id.as_deref(), Some("agent::capsule"));
        assert_eq!(created.provider_type.as_deref(), Some("codex_app_server"));
        assert_eq!(created.byte_size, 42);
        assert_eq!(created.revision, 1);
        assert_eq!(created.created_at, created.updated_at);

        let fetched = db
            .get_capsule(&id)
            .expect("get capsule")
            .expect("capsule exists");
        assert_eq!(fetched, created);

        let updated = db
            .update_capsule(
                &id,
                CapsuleUpdateDraft {
                    title: Some("Updated".to_owned()),
                    description: Some("New description".to_owned()),
                    html_sha256: Some("b".repeat(64)),
                    byte_size: Some(84),
                },
            )
            .expect("update capsule")
            .expect("updated capsule");
        assert_eq!(updated.title, "Updated");
        assert_eq!(updated.description, "New description");
        assert_eq!(updated.html_sha256, "b".repeat(64));
        assert_eq!(updated.byte_size, 84);
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.created_at, created.created_at);
        assert_eq!(updated.thread_id, created.thread_id);
        assert_eq!(updated.agent_id, created.agent_id);
        assert_eq!(
            db.list_capsules().expect("list capsules"),
            vec![updated.clone()]
        );

        assert!(db.delete_capsule(&id).expect("delete capsule"));
        assert!(!db.delete_capsule(&id).expect("delete missing capsule"));
        assert!(db.get_capsule(&id).expect("get after delete").is_none());
    }

    #[test]
    fn capsules_list_orders_updated_desc_and_filters_thread() {
        let db = GaryxDbService::memory().expect("db opens");
        let first_id = Uuid::new_v4().to_string();
        let second_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        db.create_capsule(capsule_draft(&first_id, "First", "thread::one"))
            .expect("create first");
        db.create_capsule(capsule_draft(&second_id, "Second", "thread::one"))
            .expect("create second");
        db.create_capsule(capsule_draft(&other_id, "Other", "thread::two"))
            .expect("create other");
        std::thread::sleep(std::time::Duration::from_millis(2));
        db.update_capsule(
            &first_id,
            CapsuleUpdateDraft {
                title: Some("First updated".to_owned()),
                ..Default::default()
            },
        )
        .expect("update first");

        let all = db.list_capsules().expect("list all");
        assert_eq!(all[0].id, first_id);
        let thread_one = db
            .list_capsules_for_thread("thread::one")
            .expect("list thread one");
        assert_eq!(thread_one.len(), 2);
        assert_eq!(thread_one[0].id, first_id);
        assert!(thread_one.iter().any(|record| record.id == first_id));
        assert!(thread_one.iter().any(|record| record.id == second_id));
        assert!(
            thread_one
                .iter()
                .all(|record| record.thread_id.as_deref() == Some("thread::one"))
        );
    }

    #[test]
    fn capsules_reject_invalid_uuid_hash_and_size() {
        let db = GaryxDbService::memory().expect("db opens");
        assert!(matches!(
            db.create_capsule(capsule_draft("not-a-uuid", "Bad", "thread::bad")),
            Err(GaryxDbError::BadRequest(_))
        ));
        let id = Uuid::new_v4().to_string();
        let mut bad_hash = capsule_draft(&id, "Bad Hash", "thread::bad");
        bad_hash.html_sha256 = "not-hex".to_owned();
        assert!(matches!(
            db.create_capsule(bad_hash),
            Err(GaryxDbError::BadRequest(_))
        ));
        let mut bad_size = capsule_draft(&id, "Bad Size", "thread::bad");
        bad_size.byte_size = -1;
        assert!(matches!(
            db.create_capsule(bad_size),
            Err(GaryxDbError::BadRequest(_))
        ));
        assert!(matches!(
            db.get_capsule("../escape"),
            Err(GaryxDbError::BadRequest(_))
        ));
    }

    #[test]
    fn recent_threads_upsert_list_and_remove() {
        let db = GaryxDbService::memory().expect("db opens");
        db.upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::older".to_owned(),
            title: "Older Thread".to_owned(),
            workspace_dir: Some("/work/test-older".to_owned()),
            thread_type: "chat".to_owned(),
            provider_type: Some("claude".to_owned()),
            agent_id: Some("agent::test".to_owned()),
            message_count: 3,
            last_message_preview: "older preview".to_owned(),
            recent_run_id: Some("run::older".to_owned()),
            active_run_id: None,
            run_state: "completed".to_owned(),
            updated_at: Some("2026-05-23T10:00:00.000Z".to_owned()),
            last_active_at: "2026-05-23T10:00:00.000Z".to_owned(),
        })
        .expect("upsert older");
        db.upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::newer".to_owned(),
            title: "Newer Thread".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 1,
            last_message_preview: "newer preview".to_owned(),
            recent_run_id: None,
            active_run_id: Some("run::active".to_owned()),
            run_state: "running".to_owned(),
            updated_at: Some("2026-05-23T11:00:00.000Z".to_owned()),
            last_active_at: "2026-05-23T11:00:00.000Z".to_owned(),
        })
        .expect("upsert newer");
        db.upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::older".to_owned(),
            title: "Older Thread Renamed".to_owned(),
            workspace_dir: Some("/work/test-older-renamed".to_owned()),
            thread_type: "task".to_owned(),
            provider_type: Some("codex".to_owned()),
            agent_id: None,
            message_count: 4,
            last_message_preview: "updated preview".to_owned(),
            recent_run_id: Some("run::older-two".to_owned()),
            active_run_id: None,
            run_state: "completed".to_owned(),
            updated_at: Some("2026-05-23T12:00:00.000Z".to_owned()),
            last_active_at: "2026-05-23T12:00:00.000Z".to_owned(),
        })
        .expect("update older");

        let records = db.list_recent_threads(10, 0).expect("list recent threads");
        assert_eq!(
            records
                .iter()
                .map(|record| record.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::older", "thread::newer"],
        );
        assert_eq!(records[0].title, "Older Thread Renamed");
        assert_eq!(
            records[0].workspace_dir.as_deref(),
            Some("/work/test-older-renamed")
        );
        assert_eq!(records[0].thread_type, "task");
        assert_eq!(records[0].provider_type.as_deref(), Some("codex"));
        assert_eq!(records[0].message_count, 4);
        assert_eq!(records[0].last_message_preview, "updated preview");
        assert_eq!(records[0].recent_run_id.as_deref(), Some("run::older-two"));
        assert_eq!(records[0].run_state, "completed");

        let limited = db
            .list_recent_threads(1, 0)
            .expect("list limited recent threads");
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].thread_id, "thread::older");
        let offset = db
            .list_recent_threads(1, 1)
            .expect("list offset recent threads");
        assert_eq!(offset.len(), 1);
        assert_eq!(offset[0].thread_id, "thread::newer");
        assert_eq!(db.count_recent_threads().expect("count recent threads"), 2);

        assert!(
            db.remove_recent_thread("thread::older")
                .expect("remove older")
        );
        assert!(
            !db.remove_recent_thread("thread::older")
                .expect("remove older again")
        );
        assert_eq!(
            db.list_recent_threads(10, 0)
                .expect("list remaining recent threads")
                .into_iter()
                .map(|record| record.thread_id)
                .collect::<Vec<_>>(),
            vec!["thread::newer"],
        );
    }

    #[test]
    fn thread_meta_projection_round_trip_and_remove() {
        let db = GaryxDbService::memory().expect("db opens");
        assert!(
            db.thread_meta_projection_needs_backfill()
                .expect("empty projection needs backfill")
        );
        let delivery_json = r#"{"channel":"telegram","account_id":"main","chat_id":"42","user_id":"42","delivery_target_type":"chat_id","delivery_target_id":"42"}"#.to_owned();
        db.replace_thread_meta_projection(ThreadMetaProjectionDraft {
            thread_id: "thread::workflow".to_owned(),
            thread_meta: ThreadMetaDraft {
                thread_id: "thread::workflow".to_owned(),
                workspace_dir: Some("/work/project".to_owned()),
                thread_type: "workflow_run".to_owned(),
                thread_label: Some("Workflow Run".to_owned()),
                agent_id: Some("deep-research".to_owned()),
                provider_type: Some("workflow".to_owned()),
                provider_key: None,
                selected_model: None,
                selected_model_reasoning_effort: None,
                selected_model_service_tier: None,
                sdk_session_id: None,
                created_at: Some("2026-06-03T07:59:00.000Z".to_owned()),
                updated_at: Some("2026-06-03T08:00:00.000Z".to_owned()),
                message_count: 2,
                last_user_message: Some("start workflow".to_owned()),
                last_assistant_message: Some("done".to_owned()),
                last_message_preview: Some("done".to_owned()),
                recent_run_id: Some("run::workflow".to_owned()),
                active_run_id: None,
                worktree_json: Some(r#"{"path":"/work/project"}"#.to_owned()),
                last_delivery_context_json: Some(delivery_json.clone()),
                last_delivery_updated_at: Some("2026-06-03T08:00:01.000Z".to_owned()),
                default_list_hidden: false,
            },
            channel_endpoints: vec![KnownChannelEndpoint {
                endpoint_key: "telegram::main::42".to_owned(),
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                binding_key: "42".to_owned(),
                chat_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                display_label: "Test User".to_owned(),
                thread_id: Some("thread::workflow".to_owned()),
                thread_label: Some("Workflow Run".to_owned()),
                workspace_dir: Some("/work/project".to_owned()),
                thread_updated_at: Some("2026-06-03T08:00:00.000Z".to_owned()),
                last_inbound_at: Some("2026-06-03T07:59:59.000Z".to_owned()),
                last_delivery_at: Some("2026-06-03T08:00:01.000Z".to_owned()),
            }],
            message_routes: vec![ThreadMessageRouteDraft {
                thread_id: "thread::workflow".to_owned(),
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "42".to_owned(),
                thread_binding_key: Some("42".to_owned()),
                message_id: "message-1".to_owned(),
            }],
        })
        .expect("project thread meta");
        assert!(
            !db.thread_meta_projection_needs_backfill()
                .expect("current projection does not need backfill")
        );

        let meta = db.list_thread_meta().expect("list meta");
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].thread_id, "thread::workflow");
        assert_eq!(meta[0].thread_type, "workflow_run");
        assert_eq!(meta[0].workspace_dir.as_deref(), Some("/work/project"));
        assert_eq!(
            meta[0].last_delivery_context_json.as_deref(),
            Some(delivery_json.as_str())
        );
        assert_eq!(
            meta[0].last_delivery_updated_at.as_deref(),
            Some("2026-06-03T08:00:01.000Z")
        );

        let endpoints = db
            .list_thread_channel_endpoints()
            .expect("list channel endpoints");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].endpoint_key, "telegram::main::42");
        assert_eq!(endpoints[0].thread_id.as_deref(), Some("thread::workflow"));

        let routes = db.list_thread_message_routes().expect("list routes");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].message_id, "message-1");
        assert_eq!(routes[0].thread_binding_key.as_deref(), Some("42"));

        assert!(
            db.remove_thread_meta_projection("thread::workflow")
                .expect("remove projection")
        );
        assert!(
            db.list_thread_meta()
                .expect("list meta after remove")
                .is_empty()
        );
        assert!(
            db.list_thread_channel_endpoints()
                .expect("list endpoints after remove")
                .is_empty()
        );
        assert!(
            db.list_thread_message_routes()
                .expect("list routes after remove")
                .is_empty()
        );
        assert!(
            db.thread_meta_projection_needs_backfill()
                .expect("removed projection needs backfill")
        );
    }

    #[test]
    fn automation_thread_runs_round_trip_and_finish() {
        let db = GaryxDbService::memory().expect("db opens");
        let record = db
            .upsert_automation_thread_run(AutomationThreadRunDraft {
                automation_id: "automation::daily".to_owned(),
                run_id: "run-1".to_owned(),
                thread_id: "thread::generated".to_owned(),
                workspace_dir: Some("/Users/test/project".to_owned()),
                agent_id: Some("claude".to_owned()),
                automation_label_snapshot: Some("Daily".to_owned()),
                mode: "generated_thread".to_owned(),
                status: "running".to_owned(),
                started_at: "2026-05-28T00:00:00Z".to_owned(),
                finished_at: None,
            })
            .expect("insert automation run");

        assert_eq!(record.status, "running");
        assert_eq!(
            db.count_automation_thread_runs("automation::daily", Some("generated_thread"))
                .expect("count"),
            1
        );

        assert!(
            db.finish_automation_thread_run(
                "automation::daily",
                "run-1",
                "success",
                "2026-05-28T00:00:05Z",
            )
            .expect("finish")
        );

        let records = db
            .list_automation_thread_runs("automation::daily", Some("generated_thread"), 10, 0)
            .expect("list runs");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].thread_id, "thread::generated");
        assert_eq!(records[0].status, "success");
        assert_eq!(
            records[0].automation_label_snapshot.as_deref(),
            Some("Daily")
        );
    }

    #[test]
    fn recent_threads_snapshot_sync_prunes_absent_rows_and_batches_updates() {
        let db = GaryxDbService::memory().expect("db opens");
        db.upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::stale".to_owned(),
            title: "Stale".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 0,
            last_message_preview: String::new(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-05-23T08:00:00.000Z".to_owned()),
            last_active_at: "2026-05-23T08:00:00.000Z".to_owned(),
        })
        .expect("seed stale");

        db.sync_recent_threads_snapshot(
            vec![
                RecentThreadDraft {
                    thread_id: "thread::kept".to_owned(),
                    title: "Kept".to_owned(),
                    workspace_dir: None,
                    thread_type: "chat".to_owned(),
                    provider_type: None,
                    agent_id: None,
                    message_count: 1,
                    last_message_preview: "kept preview".to_owned(),
                    recent_run_id: None,
                    active_run_id: None,
                    run_state: "idle".to_owned(),
                    updated_at: Some("2026-05-23T10:00:00.000Z".to_owned()),
                    last_active_at: "2026-05-23T10:00:00.000Z".to_owned(),
                },
                RecentThreadDraft {
                    thread_id: "thread::pruned-by-limit".to_owned(),
                    title: "Pruned".to_owned(),
                    workspace_dir: None,
                    thread_type: "chat".to_owned(),
                    provider_type: None,
                    agent_id: None,
                    message_count: 1,
                    last_message_preview: "old preview".to_owned(),
                    recent_run_id: None,
                    active_run_id: None,
                    run_state: "idle".to_owned(),
                    updated_at: Some("2026-05-23T09:00:00.000Z".to_owned()),
                    last_active_at: "2026-05-23T09:00:00.000Z".to_owned(),
                },
            ],
            1,
        )
        .expect("sync snapshot");

        assert_eq!(
            db.list_recent_threads(10, 0)
                .expect("list synced recent threads")
                .into_iter()
                .map(|record| record.thread_id)
                .collect::<Vec<_>>(),
            vec!["thread::kept"],
        );
    }

    #[test]
    fn dreams_replace_window_lists_topics_with_spans() {
        let db = GaryxDbService::memory().expect("db opens");
        let older = DreamTopicDraft {
            dream_id: "dream::older".to_owned(),
            title: "Old Plan".to_owned(),
            summary: "Outside the scanned window.".to_owned(),
            first_message_at: "2026-05-20T08:00:00.000Z".to_owned(),
            last_message_at: "2026-05-20T08:10:00.000Z".to_owned(),
            source: "heuristic".to_owned(),
            confidence: 0.5,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::older".to_owned(),
                thread_id: "thread::older".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-20T08:00:00.000Z".to_owned(),
                end_at: "2026-05-20T08:10:00.000Z".to_owned(),
                excerpt: "old".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-20T00:00:00.000Z",
            "2026-05-20T23:59:59.999Z",
            "heuristic",
            &[older],
            None,
        )
        .expect("insert older dream");

        let topic = DreamTopicDraft {
            dream_id: "dream::today".to_owned(),
            title: "Gateway Pin Polish".to_owned(),
            summary: "Review pinned-thread routing and mobile state.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:20:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.92,
            message_count: 2,
            spans: vec![DreamSpanDraft {
                span_id: "span::today".to_owned(),
                thread_id: "thread::today".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 3,
                end_seq: 5,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:20:00.000Z".to_owned(),
                excerpt: "pin routing".to_owned(),
                message_count: 2,
            }],
        };
        let scan = db
            .replace_dreams_in_window(
                "2026-05-21T00:00:00.000Z",
                "2026-05-21T23:59:59.999Z",
                "claude",
                std::slice::from_ref(&topic),
                None,
            )
            .expect("insert today's dreams");

        assert_eq!(scan.topics_count, 1);
        assert_eq!(scan.spans_count, 1);

        let records = db
            .list_dream_topics(Some("2026-05-21T00:00:00.000Z"), None, 20)
            .expect("list dreams");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "Gateway Pin Polish");
        assert_eq!(records[0].spans[0].thread_id, "thread::today");
        assert_eq!(
            records[0].spans[0].workspace_dir.as_deref(),
            Some("/workspace/test")
        );

        let detail = db
            .get_dream_topic("dream::today")
            .expect("get dream")
            .expect("dream exists");
        assert_eq!(
            detail.summary,
            "Review pinned-thread routing and mobile state."
        );
        assert_eq!(detail.spans.len(), 1);

        let latest_scan = db
            .latest_dream_scan()
            .expect("latest scan")
            .expect("scan exists");
        assert_eq!(latest_scan.run_id, scan.run_id);
    }

    #[test]
    fn dreams_replace_window_removes_previous_overlapping_topics() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::original".to_owned(),
            title: "Original".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "heuristic".to_owned(),
            confidence: 0.5,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::original".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "heuristic",
            &[original],
            None,
        )
        .expect("insert original");

        db.replace_dreams_in_window(
            "2026-05-21T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "heuristic",
            &[],
            Some("no user messages"),
        )
        .expect("replace with empty scan");

        assert!(
            db.list_dream_topics(Some("2026-05-21T00:00:00.000Z"), None, 20)
                .expect("list dreams")
                .is_empty()
        );
        assert!(matches!(
            db.latest_dream_scan().expect("scan exists"),
            Some(DreamScanRunRecord { status, .. }) if status == "fallback"
        ));
    }

    #[test]
    fn dreams_replace_window_keeps_partially_overlapping_topics() {
        let db = GaryxDbService::memory().expect("db opens");
        let spanning = DreamTopicDraft {
            dream_id: "dream::spanning".to_owned(),
            title: "Spanning".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-20T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 2,
            spans: vec![DreamSpanDraft {
                span_id: "span::spanning".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 2,
                start_at: "2026-05-20T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 2,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-20T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "claude",
            &[spanning],
            None,
        )
        .expect("insert spanning topic");

        db.replace_dreams_in_window(
            "2026-05-21T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "claude",
            &[],
            None,
        )
        .expect("replace narrow window");

        assert!(
            db.get_dream_topic("dream::spanning")
                .expect("get spanning topic")
                .is_some()
        );
    }

    #[test]
    fn dreams_incremental_upsert_extends_existing_topics_without_deleting_old_spans() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::incremental".to_owned(),
            title: "Dreams".to_owned(),
            summary: "Initial dream topic.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::first".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "initial".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::incremental".to_owned(),
            title: "Dreams Auto Scan".to_owned(),
            summary: "Initial topic plus automatic scan work.".to_owned(),
            first_message_at: "2026-05-21T10:30:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:35:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::second".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 2,
                end_seq: 2,
                start_at: "2026-05-21T10:30:00.000Z".to_owned(),
                end_at: "2026-05-21T10:35:00.000Z".to_owned(),
                excerpt: "automatic scan".to_owned(),
                message_count: 1,
            }],
        };
        let scan = db
            .upsert_dreams_incremental(
                "2026-05-21T10:00:00.000Z",
                "2026-05-21T11:00:00.000Z",
                "claude_incremental",
                &[update],
                None,
            )
            .expect("incremental upsert succeeds");

        assert_eq!(scan.topics_count, 1);
        assert_eq!(scan.spans_count, 1);

        let topic = db
            .get_dream_topic("dream::incremental")
            .expect("get topic")
            .expect("topic exists");
        assert_eq!(topic.title, "Dreams Auto Scan");
        assert_eq!(topic.message_count, 2);
        assert_eq!(topic.span_count, 2);
        assert_eq!(topic.first_message_at, "2026-05-21T10:00:00.000Z");
        assert_eq!(topic.last_message_at, "2026-05-21T10:35:00.000Z");
        assert_eq!(
            topic
                .spans
                .iter()
                .map(|span| span.span_id.as_str())
                .collect::<Vec<_>>(),
            vec!["span::first", "span::second"]
        );
    }

    #[test]
    fn dreams_incremental_upsert_preserves_existing_span_identity() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::stable-span".to_owned(),
            title: "Stable Span".to_owned(),
            summary: "Original summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::stable".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "original".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::stable-span".to_owned(),
            title: "Stable Span Updated".to_owned(),
            summary: "Updated summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "updated excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        let topic = db
            .get_dream_topic("dream::stable-span")
            .expect("get topic")
            .expect("topic exists");
        assert_eq!(topic.spans.len(), 1);
        assert_eq!(topic.spans[0].span_id, "span::stable");
        assert_eq!(topic.spans[0].excerpt, "updated excerpt");
    }

    #[test]
    fn dreams_incremental_upsert_reuses_existing_topic_for_overlapping_new_id() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::existing-topic".to_owned(),
            title: "Existing Topic".to_owned(),
            summary: "Original summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::existing-topic".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "original".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::fresh-topic".to_owned(),
            title: "Existing Topic Updated".to_owned(),
            summary: "Updated summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh-topic".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "updated excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        assert!(
            db.get_dream_topic("dream::fresh-topic")
                .expect("get fresh topic")
                .is_none()
        );
        let topic = db
            .get_dream_topic("dream::existing-topic")
            .expect("get existing topic")
            .expect("topic exists");
        assert_eq!(topic.title, "Existing Topic Updated");
        assert_eq!(topic.spans.len(), 1);
        assert_eq!(topic.spans[0].span_id, "span::existing-topic");
        assert_eq!(topic.spans[0].excerpt, "updated excerpt");
    }

    #[test]
    fn dreams_incremental_upsert_merges_duplicate_existing_topics_on_overlap() {
        let db = GaryxDbService::memory().expect("db opens");
        let alpha = DreamTopicDraft {
            dream_id: "dream::alpha".to_owned(),
            title: "Alpha".to_owned(),
            summary: "Original alpha.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::alpha".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "alpha excerpt".to_owned(),
                message_count: 1,
            }],
        };
        let beta = DreamTopicDraft {
            dream_id: "dream::beta".to_owned(),
            title: "Beta".to_owned(),
            summary: "Duplicate beta.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::beta".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "beta excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[alpha, beta],
            None,
        )
        .expect("insert duplicate topics");

        let update = DreamTopicDraft {
            dream_id: "dream::fresh".to_owned(),
            title: "Merged Topic".to_owned(),
            summary: "Merged summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "merged excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        let topics = db
            .list_dream_topics(Some("2026-05-21T00:00:00.000Z"), None, 20)
            .expect("list topics");
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].dream_id, "dream::alpha");
        assert_eq!(topics[0].title, "Merged Topic");
        assert_eq!(topics[0].spans.len(), 1);
        assert_eq!(topics[0].spans[0].span_id, "span::alpha");
        assert_eq!(topics[0].spans[0].excerpt, "merged excerpt");
    }

    #[test]
    fn dreams_incremental_upsert_merges_overlapping_spans_with_stable_identity() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::overlap-span".to_owned(),
            title: "Overlap Span".to_owned(),
            summary: "Original summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::stable-overlap".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "original".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::overlap-span".to_owned(),
            title: "Overlap Span Updated".to_owned(),
            summary: "Updated summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:10:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 2,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh-overlap".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 2,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:10:00.000Z".to_owned(),
                excerpt: "expanded excerpt".to_owned(),
                message_count: 2,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        let topic = db
            .get_dream_topic("dream::overlap-span")
            .expect("get topic")
            .expect("topic exists");
        assert_eq!(topic.spans.len(), 1);
        assert_eq!(topic.spans[0].span_id, "span::stable-overlap");
        assert_eq!(topic.spans[0].start_seq, 1);
        assert_eq!(topic.spans[0].end_seq, 2);
        assert_eq!(topic.spans[0].excerpt, "expanded excerpt");
    }

    #[test]
    fn dreams_incremental_upsert_reuses_overlapping_topic_for_generated_id() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::existing".to_owned(),
            title: "Existing Topic".to_owned(),
            summary: "Original summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::existing".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "original".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::generated".to_owned(),
            title: "Existing Topic Updated".to_owned(),
            summary: "Updated summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "updated excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        assert!(
            db.get_dream_topic("dream::generated")
                .expect("get generated topic")
                .is_none()
        );
        let topic = db
            .get_dream_topic("dream::existing")
            .expect("get existing topic")
            .expect("existing topic remains");
        assert_eq!(topic.title, "Existing Topic Updated");
        assert_eq!(topic.spans.len(), 1);
        assert_eq!(topic.spans[0].span_id, "span::existing");
        assert_eq!(topic.spans[0].excerpt, "updated excerpt");

        let topics = db
            .list_dream_topics_for_threads(&["thread::one".to_owned()], None, 10)
            .expect("list thread topics");
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].dream_id, "dream::existing");
    }

    #[test]
    fn dreams_incremental_upsert_keeps_distinct_overlapping_topics() {
        let db = GaryxDbService::memory().expect("db opens");
        let broad = DreamTopicDraft {
            dream_id: "dream::broad".to_owned(),
            title: "Broad Topic".to_owned(),
            summary: "Broad summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:50:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 10,
            spans: vec![DreamSpanDraft {
                span_id: "span::broad".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 10,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:50:00.000Z".to_owned(),
                excerpt: "broad".to_owned(),
                message_count: 10,
            }],
        };
        let narrow = DreamTopicDraft {
            dream_id: "dream::narrow".to_owned(),
            title: "Narrow Topic".to_owned(),
            summary: "Narrow summary.".to_owned(),
            first_message_at: "2026-05-21T10:10:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:20:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 2,
            spans: vec![DreamSpanDraft {
                span_id: "span::narrow".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 3,
                end_seq: 4,
                start_at: "2026-05-21T10:10:00.000Z".to_owned(),
                end_at: "2026-05-21T10:20:00.000Z".to_owned(),
                excerpt: "narrow".to_owned(),
                message_count: 2,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:50:00.000Z",
            "claude",
            &[broad, narrow],
            None,
        )
        .expect("insert original topics");

        let update = DreamTopicDraft {
            dream_id: "dream::generated-broad".to_owned(),
            title: "Broad Topic Updated".to_owned(),
            summary: "Updated broad summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:50:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 10,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh-broad".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 10,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:50:00.000Z".to_owned(),
                excerpt: "updated broad".to_owned(),
                message_count: 10,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        assert!(
            db.get_dream_topic("dream::generated-broad")
                .expect("get generated topic")
                .is_none()
        );
        assert_eq!(
            db.get_dream_topic("dream::broad")
                .expect("get broad topic")
                .expect("broad exists")
                .title,
            "Broad Topic Updated"
        );
        assert_eq!(
            db.get_dream_topic("dream::narrow")
                .expect("get narrow topic")
                .expect("narrow exists")
                .title,
            "Narrow Topic"
        );
        let topics = db
            .list_dream_topics_for_threads(&["thread::one".to_owned()], None, 10)
            .expect("list thread topics");
        assert_eq!(topics.len(), 2);
    }

    #[test]
    fn dreams_list_topics_for_threads_returns_only_matching_topics() {
        let db = GaryxDbService::memory().expect("db opens");
        let matching = DreamTopicDraft {
            dream_id: "dream::matching".to_owned(),
            title: "Matching".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::matching".to_owned(),
                thread_id: "thread::matching".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 1,
            }],
        };
        let other = DreamTopicDraft {
            dream_id: "dream::other".to_owned(),
            title: "Other".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-21T11:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T11:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::other".to_owned(),
                thread_id: "thread::other".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T11:00:00.000Z".to_owned(),
                end_at: "2026-05-21T11:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 1,
            }],
        };
        let old_matching = DreamTopicDraft {
            dream_id: "dream::old-matching".to_owned(),
            title: "Old Matching".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-19T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-19T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::old-matching".to_owned(),
                thread_id: "thread::matching".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-19T10:00:00.000Z".to_owned(),
                end_at: "2026-05-19T10:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-19T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "claude",
            &[matching, other, old_matching],
            None,
        )
        .expect("insert dreams");

        let topics = db
            .list_dream_topics_for_threads(
                &["thread::matching".to_owned()],
                Some("2026-05-21T00:00:00.000Z"),
                20,
            )
            .expect("list topics by thread");

        assert_eq!(
            topics
                .iter()
                .map(|topic| topic.dream_id.as_str())
                .collect::<Vec<_>>(),
            vec!["dream::matching"]
        );
        assert_eq!(topics[0].spans[0].thread_id, "thread::matching");
    }
}
