use std::cmp::Ordering as CmpOrdering;
#[cfg(any(test, feature = "test-seams"))]
use std::collections::HashMap;
use std::collections::{BTreeSet, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-seams"))]
use std::sync::{Condvar, atomic::AtomicBool, atomic::Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use garyx_router::{KnownChannelEndpoint, is_thread_key};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, params, params_from_iter,
    types::Value as SqlValue,
};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

mod meetings;
mod task_forest;

pub use meetings::*;
pub use task_forest::{
    CURRENT_TASK_PROJECTION_VERSION, TaskForestNode, TaskForestPage, TaskForestScope,
    TaskProjectionDraft,
};

const CURRENT_THREAD_META_PROJECTION_VERSION: i64 = 5;
pub(crate) const THREAD_META_SUMMARY_MIGRATION_NAME: &str = "thread_meta_summary_v1";
const THREAD_META_SUMMARY_MIGRATION_VERSION: i64 = 1;
pub(crate) const RECENT_TASK_THREAD_KIND_MIGRATION_NAME: &str = "recent_task_thread_kind_v1";
const RECENT_TASK_THREAD_KIND_MIGRATION_VERSION: i64 = 1;
pub(crate) const ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME: &str = "endpoint_holder_dedup_v1";
const ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION: i64 = 1;
pub(crate) const THREAD_PIN_SORT_ORDER_MIGRATION_NAME: &str = "thread_pin_sort_order_v1";
const THREAD_PIN_SORT_ORDER_MIGRATION_VERSION: i64 = 1;
pub(crate) const DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME: &str = "drop_thread_message_routes_v1";
const DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION: i64 = 1;
pub(crate) const RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME: &str = "recent_thread_activity_seq_v1";
const RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION: i64 = 1;
pub(crate) const RECENT_MEMBERSHIP_MIGRATION_NAME: &str = "recent_membership_v2";
const RECENT_MEMBERSHIP_MIGRATION_VERSION: i64 = 2;
const LEGACY_IMPORT_GENERATION_NAME: &str = "legacy_import_generation";
const LEGACY_IMPORT_GENERATION_VERSION: i64 = 1;
pub(crate) const MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE: i64 = 9_007_199_254_740_991;

#[cfg(any(test, feature = "test-seams"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TestDbFaultPoint {
    LegacyMarkerPairRead,
    LegacyImportCommit,
    LegacyImportAfterIncarnationRotation,
    LegacyRetirementMarkerWrite,
    ArchivedThreadRead,
    LegacyGenerationSeedWrite,
    DeleteThreadRecord,
}

#[cfg(any(test, feature = "test-seams"))]
#[derive(Debug, Default)]
struct TestDbFaults {
    calls: HashMap<TestDbFaultPoint, usize>,
    fail_on: HashSet<(TestDbFaultPoint, usize)>,
    mutation_barriers: HashMap<TestDbMutationPoint, std::sync::Arc<TestDbMutationBarrierState>>,
}

#[cfg(any(test, feature = "test-seams"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TestDbMutationPoint {
    ArchiveThreadRecord,
    DeleteThreadRecord,
}

#[cfg(any(test, feature = "test-seams"))]
#[derive(Debug)]
struct TestDbMutationBarrierState {
    started: AtomicBool,
    started_notify: tokio::sync::Notify,
    released: Mutex<bool>,
    release_notify: Condvar,
}

#[cfg(any(test, feature = "test-seams"))]
impl TestDbMutationBarrierState {
    fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            started_notify: tokio::sync::Notify::new(),
            released: Mutex::new(false),
            release_notify: Condvar::new(),
        }
    }

    fn block(&self) {
        self.started.store(true, Ordering::Release);
        self.started_notify.notify_waiters();
        let mut released = self
            .released
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*released {
            released = self
                .release_notify
                .wait(released)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn release(&self) {
        *self
            .released
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        self.release_notify.notify_all();
    }
}

/// One-shot deterministic seam proving that a coordinator-owned blocking
/// mutation outlives cancellation of the HTTP future that initiated it.
#[cfg(any(test, feature = "test-seams"))]
pub(crate) struct TestDbMutationBarrier {
    state: std::sync::Arc<TestDbMutationBarrierState>,
}

#[cfg(any(test, feature = "test-seams"))]
impl TestDbMutationBarrier {
    pub(crate) async fn wait_until_started(&self) {
        loop {
            let notified = self.state.started_notify.notified();
            if self.state.started.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn release(&self) {
        self.state.release();
    }
}

#[cfg(any(test, feature = "test-seams"))]
impl Drop for TestDbMutationBarrier {
    fn drop(&mut self) {
        // A failed test must never strand a blocking-pool worker.
        self.state.release();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GaryxDbError {
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("thread is archived: {0}")]
    ThreadArchived(String),
    #[error("database lock poisoned")]
    LockPoisoned,
    #[error("blocking database task failed: {0}")]
    Join(String),
    #[error("database configuration failed: {0}")]
    Configuration(String),
    #[error(
        "data dir is occupied by another Garyx gateway: {path} (waited {wait_secs}s); stop the running gateway or choose a different sessions.data_dir"
    )]
    DataDirLocked { path: PathBuf, wait_secs: u64 },
    #[error(
        "pre-lock Garyx parent process {parent_pid} did not exit within {wait_secs}s; refusing destructive database initialization"
    )]
    ParentHandoffTimedOut { parent_pid: u32, wait_secs: u64 },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub type GaryxDbResult<T> = Result<T, GaryxDbError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveRecentThreadPage {
    pub thread_ids: Vec<String>,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OneShotMigrationSummary {
    pub source_row_count: usize,
    pub updated_row_count: usize,
    pub already_completed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinnedThreadRecord {
    pub thread_id: String,
    pub pinned_at: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThreadPinsPage {
    pub pins: Vec<PinnedThreadRecord>,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StoreIncarnation {
    pub store_incarnation_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FavoriteThreadRecord {
    pub thread_id: String,
    pub favorited_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThreadFavoritesPage {
    pub favorites: Vec<FavoriteThreadRecord>,
    pub revision: i64,
    pub store_incarnation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FavoriteThreadResult {
    Updated {
        changed: bool,
        page: ThreadFavoritesPage,
    },
    Conflict(ThreadFavoritesPage),
    WrongIncarnation(ThreadFavoritesPage),
    NotFound(ThreadFavoritesPage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadFavoritesSnapshot {
    pub page: ThreadFavoritesPage,
    pub recent_threads: Vec<RecentThreadRecord>,
    pub recent_total: usize,
    pub recent_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadFavoritesSummarySnapshot {
    pub snapshot: ThreadFavoritesSnapshot,
    pub summaries: Vec<ThreadSummaryRow>,
    pub summaries_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReorderThreadPinsResult {
    Updated(ThreadPinsPage),
    Conflict(ThreadPinsPage),
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
    pub activity_seq: i64,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RecentThreadTaskFilter {
    #[default]
    Include,
    Exclude,
    Only,
}

impl RecentThreadTaskFilter {
    pub(crate) fn cursor_value(self) -> &'static str {
        match self {
            Self::Include => "include",
            Self::Exclude => "exclude",
            Self::Only => "only",
        }
    }

    fn count_sql(self) -> &'static str {
        match self {
            Self::Include => "SELECT COUNT(*) FROM recent_threads",
            Self::Exclude => "SELECT COUNT(*) FROM recent_threads WHERE thread_type <> 'task'",
            Self::Only => "SELECT COUNT(*) FROM recent_threads WHERE thread_type = 'task'",
        }
    }

    fn page_sql(self) -> &'static str {
        match self {
            Self::Include => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  ORDER BY activity_seq DESC
                  LIMIT ?1 OFFSET ?2"
            }
            Self::Exclude => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type <> 'task'
                  ORDER BY activity_seq DESC
                  LIMIT ?1 OFFSET ?2"
            }
            Self::Only => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type = 'task'
                  ORDER BY activity_seq DESC
                  LIMIT ?1 OFFSET ?2"
            }
        }
    }

    fn keyset_page_sql(self, has_cursor: bool) -> &'static str {
        match (self, has_cursor) {
            (Self::Include, false) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  ORDER BY activity_seq DESC
                  LIMIT ?1"
            }
            (Self::Include, true) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE activity_seq < ?1
                  ORDER BY activity_seq DESC
                  LIMIT ?2"
            }
            (Self::Exclude, false) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type <> 'task'
                  ORDER BY activity_seq DESC
                  LIMIT ?1"
            }
            (Self::Exclude, true) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type <> 'task' AND activity_seq < ?1
                  ORDER BY activity_seq DESC
                  LIMIT ?2"
            }
            (Self::Only, false) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type = 'task'
                  ORDER BY activity_seq DESC
                  LIMIT ?1"
            }
            (Self::Only, true) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type = 'task' AND activity_seq < ?1
                  ORDER BY activity_seq DESC
                  LIMIT ?2"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecentThreadDbPage {
    pub records: Vec<RecentThreadRecord>,
    pub total: usize,
    pub offset: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecentThreadKeysetDbPage {
    pub records: Vec<RecentThreadRecord>,
    pub total: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThreadSummaryRow {
    pub thread_id: String,
    pub title: Option<String>,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub provider_type: Option<String>,
    pub agent_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub message_count: u32,
    pub last_user_message: Option<String>,
    pub last_assistant_message: Option<String>,
    pub last_message_preview: Option<String>,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub worktree: Option<Value>,
    pub excluded_from_recent: bool,
    #[serde(skip)]
    pub(crate) sort_updated_at_us: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThreadSummaryDbPage {
    pub records: Vec<ThreadSummaryRow>,
    pub has_more: bool,
    pub store_incarnation_id: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ThreadSummaryTaskFilter {
    #[default]
    Include,
    Exclude,
    Only,
}

impl ThreadSummaryTaskFilter {
    pub(crate) fn cursor_value(self) -> &'static str {
        match self {
            Self::Include => "include",
            Self::Exclude => "exclude",
            Self::Only => "only",
        }
    }

    fn page_sql(self, scoped: bool, has_query: bool, has_cursor: bool) -> &'static str {
        match self {
            Self::Include => thread_summary_include_sql(scoped, has_query, has_cursor),
            Self::Exclude => thread_summary_exclude_sql(scoped, has_query, has_cursor),
            Self::Only => thread_summary_only_sql(scoped, has_query, has_cursor),
        }
    }
}

macro_rules! thread_summary_sql {
    ("", "", $query:literal, $cursor:literal) => {
        thread_summary_sql!("idx_thread_meta_summary_visible", "", "", $query, $cursor)
    };
    ("\n   AND workspace_dir = ?", "", $query:literal, $cursor:literal) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_workspace_visible",
            "\n   AND workspace_dir = ?",
            "",
            $query,
            $cursor
        )
    };
    ("", "\n   AND thread_type <> 'task'", $query:literal, $cursor:literal) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_non_task",
            "",
            "\n   AND thread_type <> 'task'",
            $query,
            $cursor
        )
    };
    (
        "\n   AND workspace_dir = ?",
        "\n   AND thread_type <> 'task'",
        $query:literal,
        $cursor:literal
    ) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_workspace_non_task",
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type <> 'task'",
            $query,
            $cursor
        )
    };
    ("", "\n   AND thread_type = 'task'", $query:literal, $cursor:literal) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_task",
            "",
            "\n   AND thread_type = 'task'",
            $query,
            $cursor
        )
    };
    (
        "\n   AND workspace_dir = ?",
        "\n   AND thread_type = 'task'",
        $query:literal,
        $cursor:literal
    ) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_workspace_task",
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type = 'task'",
            $query,
            $cursor
        )
    };
    ($index:literal, $scope:literal, $task:literal, $query:literal, $cursor:literal) => {
        concat!(
            "SELECT thread_id, thread_label, workspace_dir, thread_type, provider_type,\n",
            "       agent_id, created_at, updated_at, message_count, last_user_message,\n",
            "       last_assistant_message, last_message_preview, recent_run_id,\n",
            "       active_run_id, worktree_json, excluded_from_recent, sort_updated_at_us\n",
            "  FROM thread_meta INDEXED BY ",
            $index,
            "\n",
            " WHERE default_list_hidden = 0",
            $scope,
            $task,
            $query,
            $cursor,
            "\n ORDER BY sort_updated_at_us DESC, thread_id DESC\n LIMIT ?"
        )
    };
}

fn thread_summary_include_sql(scoped: bool, has_query: bool, has_cursor: bool) -> &'static str {
    match (scoped, has_query, has_cursor) {
        (false, false, false) => thread_summary_sql!("", "", "", ""),
        (false, false, true) => thread_summary_sql!(
            "",
            "",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (false, true, false) => {
            thread_summary_sql!("", "", "\n   AND instr(search_text, ?) > 0", "")
        }
        (false, true, true) => thread_summary_sql!(
            "",
            "",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, false, false) => {
            thread_summary_sql!("\n   AND workspace_dir = ?", "", "", "")
        }
        (true, false, true) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, true, false) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (true, true, true) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
    }
}

fn thread_summary_exclude_sql(scoped: bool, has_query: bool, has_cursor: bool) -> &'static str {
    match (scoped, has_query, has_cursor) {
        (false, false, false) => {
            thread_summary_sql!("", "\n   AND thread_type <> 'task'", "", "")
        }
        (false, false, true) => thread_summary_sql!(
            "",
            "\n   AND thread_type <> 'task'",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (false, true, false) => thread_summary_sql!(
            "",
            "\n   AND thread_type <> 'task'",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (false, true, true) => thread_summary_sql!(
            "",
            "\n   AND thread_type <> 'task'",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, false, false) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type <> 'task'",
            "",
            ""
        ),
        (true, false, true) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type <> 'task'",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, true, false) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type <> 'task'",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (true, true, true) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type <> 'task'",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
    }
}

fn thread_summary_only_sql(scoped: bool, has_query: bool, has_cursor: bool) -> &'static str {
    match (scoped, has_query, has_cursor) {
        (false, false, false) => {
            thread_summary_sql!("", "\n   AND thread_type = 'task'", "", "")
        }
        (false, false, true) => thread_summary_sql!(
            "",
            "\n   AND thread_type = 'task'",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (false, true, false) => thread_summary_sql!(
            "",
            "\n   AND thread_type = 'task'",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (false, true, true) => thread_summary_sql!(
            "",
            "\n   AND thread_type = 'task'",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, false, false) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type = 'task'",
            "",
            ""
        ),
        (true, false, true) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type = 'task'",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, true, false) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type = 'task'",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (true, true, true) => thread_summary_sql!(
            "\n   AND workspace_dir = ?",
            "\n   AND thread_type = 'task'",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
    }
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
    pub excluded_from_recent: bool,
    pub sort_updated_at_us: i64,
    pub search_text: String,
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
    pub excluded_from_recent: bool,
    pub sort_updated_at_us: i64,
    pub search_text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadMetaProjectionDraft {
    pub thread_id: String,
    pub thread_meta: ThreadMetaDraft,
    pub channel_endpoints: Vec<KnownChannelEndpoint>,
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
    pub favorited_at: Option<String>,
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

pub struct GaryxDbService {
    /// Process-lifetime ownership of this database's data directory. The lock
    /// is acquired before SQLite is opened, so schema initialization, imports,
    /// startup purges, and orphan-run cleanup cannot overlap another gateway.
    _data_dir_lock: Option<DataDirLock>,
    conn: Mutex<Connection>,
    /// Independent read connections (WAL snapshot reads) so point reads
    /// never queue behind the writer — or behind each other: WAL supports
    /// arbitrary concurrent readers, and a single shared read connection
    /// measurably serialized concurrent list queries (4 parallel reads took
    /// 4× one read's wall time). Empty for in-memory databases, which
    /// degrade to the single connection (#TASK-1864 batch 2, D4).
    readers: Vec<Mutex<Connection>>,
    /// Round-robin cursor into `readers`.
    next_reader: std::sync::atomic::AtomicUsize,
    #[cfg(any(test, feature = "test-seams"))]
    test_faults: Mutex<TestDbFaults>,
}

/// Narrow read-only handle for offline control-plane reads while the gateway
/// process may still own the writable database connection. Unlike
/// `GaryxDbService::open`, this never creates a database, changes WAL mode,
/// initializes schema, or exposes mutation methods.
pub(crate) struct ReadOnlyGaryxDb {
    conn: Connection,
}

const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(5_000);
const DEFAULT_DATA_LOCK_WAIT: Duration = Duration::from_secs(30);
const PRE_R5_PARENT_HANDOFF_WAIT: Duration = Duration::from_secs(60);
const STARTUP_WAIT_POLL: Duration = Duration::from_millis(50);
pub const THREAD_FAVORITES_SNAPSHOT_CAP: usize = 500;
/// Read-pool size: enough to keep the common concurrent readers (desktop,
/// mobile, a handful of agents) off each other's locks without holding a
/// meaningful number of file handles.
const READ_POOL_SIZE: usize = 4;

/// Durability/concurrency settings for the on-disk database: WAL journal
/// (persistent, readers never block the single writer), NORMAL fsync
/// (sub-ms commits, still crash-safe under WAL), and a busy timeout so
/// cross-process contention retries instead of failing fast.
fn configure_file_connection(conn: &Connection) -> GaryxDbResult<()> {
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

struct DataDirLock {
    file: File,
    _path: PathBuf,
}

impl DataDirLock {
    fn acquire(database_path: &Path, wait: Duration) -> GaryxDbResult<Self> {
        let data_dir = database_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(data_dir)?;
        let lock_path = data_dir.join("garyx.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        set_close_on_exec(&file)?;
        acquire_exclusive_flock(&file, &lock_path, wait)?;

        // The advisory lock is the authority; the PID is diagnostic data for
        // operators and deterministic restart tests only.
        file.set_len(0)?;
        file.write_all(std::process::id().to_string().as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(Self {
            file,
            _path: lock_path,
        })
    }

    #[cfg(any(test, feature = "test-seams"))]
    fn close_on_exec(&self) -> GaryxDbResult<bool> {
        close_on_exec_is_set(&self.file)
    }
}

impl Drop for DataDirLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        // Closing the file would release flock as well; the explicit unlock
        // makes the ownership boundary clear and lets a waiter proceed before
        // any later field-drop work.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(unix)]
fn set_close_on_exec(file: &File) -> GaryxDbResult<()> {
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error().into());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_close_on_exec(_file: &File) -> GaryxDbResult<()> {
    Err(GaryxDbError::Configuration(
        "per-data-dir flock is only supported on Unix".to_owned(),
    ))
}

#[cfg(all(unix, any(test, feature = "test-seams")))]
fn close_on_exec_is_set(file: &File) -> GaryxDbResult<bool> {
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(flags & libc::FD_CLOEXEC != 0)
}

#[cfg(all(not(unix), any(test, feature = "test-seams")))]
fn close_on_exec_is_set(_file: &File) -> GaryxDbResult<bool> {
    Ok(false)
}

#[cfg(unix)]
fn acquire_exclusive_flock(file: &File, lock_path: &Path, wait: Duration) -> GaryxDbResult<()> {
    let started = Instant::now();
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(code) if code == libc::EINTR => continue,
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN => {
                let elapsed = started.elapsed();
                if elapsed >= wait {
                    return Err(GaryxDbError::DataDirLocked {
                        path: lock_path.to_path_buf(),
                        wait_secs: wait.as_secs(),
                    });
                }
                std::thread::sleep(STARTUP_WAIT_POLL.min(wait.saturating_sub(elapsed)));
            }
            _ => return Err(error.into()),
        }
    }
}

#[cfg(not(unix))]
fn acquire_exclusive_flock(_file: &File, _lock_path: &Path, _wait: Duration) -> GaryxDbResult<()> {
    Err(GaryxDbError::Configuration(
        "per-data-dir flock is only supported on Unix".to_owned(),
    ))
}

fn configured_data_lock_wait() -> GaryxDbResult<Duration> {
    let Some(raw) = std::env::var_os("GARYX_DATA_LOCK_WAIT_SECS") else {
        return Ok(DEFAULT_DATA_LOCK_WAIT);
    };
    let raw = raw.to_string_lossy();
    let seconds = raw.trim().parse::<u64>().map_err(|_| {
        GaryxDbError::Configuration(
            "GARYX_DATA_LOCK_WAIT_SECS must be a non-negative integer".to_owned(),
        )
    })?;
    Ok(Duration::from_secs(seconds))
}

#[cfg(unix)]
fn wait_for_pre_r5_parent_handoff() -> GaryxDbResult<()> {
    let parent_pid = unsafe { libc::getppid() };
    if parent_pid <= 1 || !parent_has_same_executable_name(parent_pid as u32)? {
        return Ok(());
    }
    wait_for_parent_exit(parent_pid as u32, PRE_R5_PARENT_HANDOFF_WAIT, || {
        process_is_alive(parent_pid as u32)
    })
}

#[cfg(not(unix))]
fn wait_for_pre_r5_parent_handoff() -> GaryxDbResult<()> {
    Ok(())
}

fn wait_for_parent_exit(
    parent_pid: u32,
    wait: Duration,
    mut is_alive: impl FnMut() -> bool,
) -> GaryxDbResult<()> {
    let started = Instant::now();
    loop {
        if !is_alive() {
            return Ok(());
        }
        let elapsed = started.elapsed();
        if elapsed >= wait {
            return Err(GaryxDbError::ParentHandoffTimedOut {
                parent_pid,
                wait_secs: wait.as_secs(),
            });
        }
        std::thread::sleep(STARTUP_WAIT_POLL.min(wait.saturating_sub(elapsed)));
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    // EPERM still proves the process exists. Unknown errors are treated as
    // alive so the handoff barrier fails closed.
    !matches!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH))
}

#[cfg(unix)]
fn parent_has_same_executable_name(parent_pid: u32) -> GaryxDbResult<bool> {
    parent_has_same_executable_name_with(parent_pid, parent_executable_path)
}

#[cfg(unix)]
fn parent_has_same_executable_name_with(
    parent_pid: u32,
    resolve_parent: impl FnOnce(u32) -> GaryxDbResult<PathBuf>,
) -> GaryxDbResult<bool> {
    let current = std::env::current_exe()?;
    let current_name = current.file_name().ok_or_else(|| {
        GaryxDbError::Configuration(format!(
            "current executable path has no file name: {}",
            current.display()
        ))
    })?;
    let parent = resolve_parent(parent_pid)?;
    let parent_name = parent.file_name().ok_or_else(|| {
        GaryxDbError::Configuration(format!(
            "parent executable path has no file name: {}",
            parent.display()
        ))
    })?;
    Ok(parent_name == current_name)
}

#[cfg(target_os = "linux")]
fn parent_executable_path(parent_pid: u32) -> GaryxDbResult<PathBuf> {
    Ok(std::fs::read_link(format!("/proc/{parent_pid}/exe"))?)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn parent_executable_path(parent_pid: u32) -> GaryxDbResult<PathBuf> {
    let output = std::process::Command::new("ps")
        .args(["-p", &parent_pid.to_string(), "-o", "comm="])
        .output()
        .map_err(|error| {
            GaryxDbError::Configuration(format!(
                "failed to resolve parent executable with ps for pid {parent_pid}: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(GaryxDbError::Configuration(format!(
            "ps failed while resolving parent executable for pid {parent_pid}: {}",
            output.status
        )));
    }
    let path = String::from_utf8(output.stdout).map_err(|error| {
        GaryxDbError::Configuration(format!(
            "ps returned non-UTF-8 parent executable for pid {parent_pid}: {error}"
        ))
    })?;
    let path = path.trim();
    if path.is_empty() {
        return Err(GaryxDbError::Configuration(format!(
            "ps returned an empty parent executable for pid {parent_pid}"
        )));
    }
    Ok(PathBuf::from(path))
}

impl GaryxDbService {
    pub fn open(path: impl AsRef<Path>) -> GaryxDbResult<Self> {
        Self::open_with_lock_wait(path, configured_data_lock_wait()?)
    }

    fn open_with_lock_wait(path: impl AsRef<Path>, lock_wait: Duration) -> GaryxDbResult<Self> {
        let path = path.as_ref();
        // This must stay before Connection::open and every schema/import/
        // purge action. It is also the pre-R5 fallback cutover boundary:
        // once we own the new lock, a still-live same-executable parent must
        // exit before this binary may touch the database.
        let data_dir_lock = DataDirLock::acquire(path, lock_wait)?;
        wait_for_pre_r5_parent_handoff()?;
        let conn = Connection::open(path)?;
        configure_file_connection(&conn)?;
        initialize_connection(&conn)?;
        // Dedicated read connections: under WAL they see consistent
        // snapshots, never block on (or block) the writer, and run
        // concurrently with each other.
        let mut readers = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            let reader = Connection::open(path)?;
            reader.busy_timeout(BUSY_TIMEOUT)?;
            reader.pragma_update(None, "query_only", "ON")?;
            readers.push(Mutex::new(reader));
        }
        Ok(Self {
            _data_dir_lock: Some(data_dir_lock),
            conn: Mutex::new(conn),
            readers,
            next_reader: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-seams"))]
            test_faults: Mutex::new(TestDbFaults::default()),
        })
    }

    pub fn memory() -> GaryxDbResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        initialize_connection(&conn)?;
        Ok(Self {
            _data_dir_lock: None,
            conn: Mutex::new(conn),
            readers: Vec::new(),
            next_reader: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-seams"))]
            test_faults: Mutex::new(TestDbFaults::default()),
        })
    }

    pub fn store_incarnation_id(&self) -> GaryxDbResult<String> {
        let conn = self.read_conn()?;
        read_store_incarnation_id(&conn)
    }

    /// Rotate the persistent CAS identity for an offline full-data-dir
    /// restore/clone. Normal opens and process restarts never call this.
    pub fn rotate_store_incarnation(&self) -> GaryxDbResult<StoreIncarnation> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let store_incarnation_id = rotate_store_incarnation_tx(&tx)?;
        tx.commit()?;
        Ok(StoreIncarnation {
            store_incarnation_id,
        })
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub(crate) fn fail_test_db_call(&self, point: TestDbFaultPoint, occurrence: usize) {
        assert!(occurrence > 0, "fault occurrence is one-based");
        self.test_faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .fail_on
            .insert((point, occurrence));
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub(crate) fn block_test_db_mutation(
        &self,
        point: TestDbMutationPoint,
    ) -> TestDbMutationBarrier {
        let state = std::sync::Arc::new(TestDbMutationBarrierState::new());
        self.test_faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .mutation_barriers
            .insert(point, state.clone());
        TestDbMutationBarrier { state }
    }

    #[cfg(any(test, feature = "test-seams"))]
    fn maybe_block_test_db_mutation(&self, point: TestDbMutationPoint) {
        let barrier = self
            .test_faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .mutation_barriers
            .remove(&point);
        if let Some(barrier) = barrier {
            barrier.block();
        }
    }

    #[cfg(any(test, feature = "test-seams"))]
    fn maybe_fail_test_db_call(&self, point: TestDbFaultPoint) -> GaryxDbResult<()> {
        let mut faults = self
            .test_faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let occurrence = {
            let calls = faults.calls.entry(point).or_default();
            *calls += 1;
            *calls
        };
        if faults.fail_on.remove(&(point, occurrence)) {
            return Err(GaryxDbError::Configuration(format!(
                "injected database fault at {point:?} call {occurrence}"
            )));
        }
        Ok(())
    }

    pub(crate) fn list_active_recent_thread_ids(
        &self,
        limit: usize,
    ) -> GaryxDbResult<ActiveRecentThreadPage> {
        let mut conn = self.read_conn()?;
        list_active_recent_thread_ids(&mut conn, limit)
    }

    fn conn(&self) -> GaryxDbResult<MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| GaryxDbError::LockPoisoned)
    }

    /// Lock a read connection (file databases), or fall back to the writer
    /// connection (in-memory databases). Prefers an idle pool slot — one
    /// long read must not queue short reads behind it — and blocks on the
    /// round-robin slot only when every connection is busy.
    fn read_conn(&self) -> GaryxDbResult<MutexGuard<'_, Connection>> {
        if self.readers.is_empty() {
            return self.conn();
        }
        let start = self
            .next_reader
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.readers.len();
        for offset in 0..self.readers.len() {
            let index = (start + offset) % self.readers.len();
            match self.readers[index].try_lock() {
                Ok(guard) => return Ok(guard),
                Err(std::sync::TryLockError::WouldBlock) => continue,
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(GaryxDbError::LockPoisoned);
                }
            }
        }
        self.readers[start]
            .lock()
            .map_err(|_| GaryxDbError::LockPoisoned)
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

    pub fn list_pinned_threads(&self) -> GaryxDbResult<ThreadPinsPage> {
        self.list_pinned_threads_inner(|| Ok(()))
    }

    fn list_pinned_threads_inner<F>(&self, after_pins: F) -> GaryxDbResult<ThreadPinsPage>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let pins = read_thread_pins_tx(&tx)?;

        // Deterministic test seam: a concurrent writer may commit here, but
        // the revision read below remains on this WAL snapshot.
        after_pins()?;

        let revision = read_thread_pins_revision_tx(&tx)?;
        tx.commit()?;
        Ok(ThreadPinsPage { pins, revision })
    }

    pub fn pin_thread(&self, thread_id: &str) -> GaryxDbResult<ThreadPinsPage> {
        let thread_id = normalize_thread_id(thread_id)?;
        let pinned_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "INSERT INTO thread_pins (thread_id, pinned_at, sort_order)
             VALUES (
                 ?1,
                 ?2,
                 COALESCE((SELECT MIN(sort_order) FROM thread_pins), 0) - 1
             )
             ON CONFLICT(thread_id) DO NOTHING",
            params![thread_id, pinned_at],
        )? > 0;
        bump_thread_pins_revision_if_changed_tx(&tx, changed)?;
        let page = read_thread_pins_page_tx(&tx)?;
        tx.commit()?;
        Ok(page)
    }

    pub fn unpin_thread(&self, thread_id: &str) -> GaryxDbResult<(bool, ThreadPinsPage)> {
        let thread_id = normalize_thread_id(thread_id)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM thread_pins WHERE thread_id = ?1",
            params![thread_id],
        )? > 0;
        bump_thread_pins_revision_if_changed_tx(&tx, removed)?;
        let page = read_thread_pins_page_tx(&tx)?;
        tx.commit()?;
        Ok((removed, page))
    }

    pub fn reorder_thread_pins(
        &self,
        ordered_ids: Vec<String>,
        expected_revision: i64,
    ) -> GaryxDbResult<ReorderThreadPinsResult> {
        let ordered_ids = normalize_thread_pin_order(ordered_ids)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let current = read_thread_pins_page_tx(&tx)?;
        if current.revision != expected_revision {
            tx.commit()?;
            return Ok(ReorderThreadPinsResult::Conflict(current));
        }

        let current_ids = current
            .pins
            .iter()
            .map(|pin| pin.thread_id.as_str())
            .collect::<BTreeSet<_>>();
        let requested_ids = ordered_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut next_order = Vec::with_capacity(current.pins.len());
        for thread_id in &ordered_ids {
            if current_ids.contains(thread_id.as_str()) {
                next_order.push(thread_id.clone());
            }
        }
        for pin in &current.pins {
            if !requested_ids.contains(pin.thread_id.as_str()) {
                next_order.push(pin.thread_id.clone());
            }
        }

        {
            let mut stmt = tx.prepare(
                "UPDATE thread_pins
                    SET sort_order = ?1
                  WHERE thread_id = ?2",
            )?;
            for (index, thread_id) in next_order.iter().enumerate() {
                let sort_order = i64::try_from(index).map_err(|_| {
                    GaryxDbError::BadRequest("too many thread_ids to reorder".to_owned())
                })?;
                stmt.execute(params![sort_order, thread_id])?;
            }
        }
        bump_thread_pins_revision_if_changed_tx(&tx, true)?;
        let page = read_thread_pins_page_tx(&tx)?;
        tx.commit()?;
        Ok(ReorderThreadPinsResult::Updated(page))
    }

    pub fn list_thread_favorites(&self) -> GaryxDbResult<ThreadFavoritesPage> {
        self.list_thread_favorites_inner(|| Ok(()))
    }

    fn list_thread_favorites_inner<F>(
        &self,
        after_favorites: F,
    ) -> GaryxDbResult<ThreadFavoritesPage>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let favorites = read_thread_favorites_tx(&tx)?;

        // Deterministic WAL seam: the identity and revision below must stay
        // on the same snapshot even if another writer commits here.
        after_favorites()?;

        let page = read_thread_favorites_page_with_rows_tx(&tx, favorites)?;
        tx.commit()?;
        Ok(page)
    }

    pub fn set_thread_favorite(
        &self,
        thread_id: &str,
        favorited: bool,
        expected_revision: i64,
        expected_store_incarnation: &str,
    ) -> GaryxDbResult<FavoriteThreadResult> {
        let thread_id = normalize_thread_id(thread_id)?;
        if expected_revision < 0 {
            return Err(GaryxDbError::BadRequest(
                "expected_revision must be a non-negative integer".to_owned(),
            ));
        }
        let expected_store_incarnation = Uuid::parse_str(expected_store_incarnation)
            .map(|uuid| uuid.to_string())
            .map_err(|_| {
                GaryxDbError::BadRequest("expected_store_incarnation must be a UUID".to_owned())
            })?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;

        // Identity is the outer CAS fence: an old revision must never become
        // usable merely because a restored store happens to reuse its value.
        let current_incarnation = read_store_incarnation_id(&tx)?;
        if current_incarnation != expected_store_incarnation {
            let page = read_thread_favorites_page_tx(&tx)?;
            tx.commit()?;
            return Ok(FavoriteThreadResult::WrongIncarnation(page));
        }
        let current_revision = read_thread_favorites_revision_tx(&tx)?;
        if current_revision != expected_revision {
            let page = read_thread_favorites_page_tx(&tx)?;
            tx.commit()?;
            return Ok(FavoriteThreadResult::Conflict(page));
        }

        let changed = if favorited {
            let favorited_at = now_string();
            let inserted = tx.execute(
                "INSERT INTO thread_favorites (thread_id, favorited_at)
                 SELECT ?1, ?2
                  WHERE EXISTS (SELECT 1 FROM thread_records WHERE key = ?1)
                 ON CONFLICT(thread_id) DO NOTHING",
                params![thread_id, favorited_at],
            )? > 0;
            if !inserted && !thread_record_exists_tx(&tx, &thread_id)? {
                let page = read_thread_favorites_page_tx(&tx)?;
                tx.commit()?;
                return Ok(FavoriteThreadResult::NotFound(page));
            }
            inserted
        } else {
            if !thread_record_exists_tx(&tx, &thread_id)? {
                let page = read_thread_favorites_page_tx(&tx)?;
                tx.commit()?;
                return Ok(FavoriteThreadResult::NotFound(page));
            }
            tx.execute(
                "DELETE FROM thread_favorites WHERE thread_id = ?1",
                params![thread_id],
            )? > 0
        };

        // Every accepted conditional write advances the fence, including an
        // idempotent repeated PUT or no-op DELETE.
        bump_thread_favorites_revision_tx(&tx)?;
        let page = read_thread_favorites_page_tx(&tx)?;
        tx.commit()?;
        Ok(FavoriteThreadResult::Updated { changed, page })
    }

    pub fn thread_favorites_snapshot(&self) -> GaryxDbResult<ThreadFavoritesSnapshot> {
        self.thread_favorites_snapshot_inner(|| Ok(()))
    }

    pub fn thread_favorites_snapshot_with_summaries(
        &self,
    ) -> GaryxDbResult<ThreadFavoritesSummarySnapshot> {
        let (snapshot, summaries) = self.thread_favorites_snapshot_with_options(true, || Ok(()))?;
        let (summaries, summaries_truncated) =
            summaries.expect("enhanced favorites snapshot always computes its summary window");
        Ok(ThreadFavoritesSummarySnapshot {
            snapshot,
            summaries,
            summaries_truncated,
        })
    }

    fn thread_favorites_snapshot_inner<F>(
        &self,
        after_favorites: F,
    ) -> GaryxDbResult<ThreadFavoritesSnapshot>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        self.thread_favorites_snapshot_with_options(false, after_favorites)
            .map(|(snapshot, _)| snapshot)
    }

    fn thread_favorites_snapshot_with_options<F>(
        &self,
        include_summaries: bool,
        after_favorites: F,
    ) -> GaryxDbResult<(
        ThreadFavoritesSnapshot,
        Option<(Vec<ThreadSummaryRow>, bool)>,
    )>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let favorites = read_thread_favorites_tx(&tx)?;
        let page = read_thread_favorites_page_with_rows_tx(&tx, favorites)?;

        // The joined recent rows and membership page are one atomic read
        // unit. A commit here must be invisible until the next snapshot.
        after_favorites()?;

        let recent_total: i64 = tx.query_row(
            "SELECT COUNT(*)
               FROM recent_threads AS recent
               JOIN thread_favorites AS favorite
                 ON favorite.thread_id = recent.thread_id",
            [],
            |row| row.get(0),
        )?;
        let recent_total = usize::try_from(recent_total).unwrap_or(usize::MAX);
        let mut stmt = tx.prepare(
            "SELECT recent.thread_id, recent.title, recent.workspace_dir,
                    recent.thread_type, recent.provider_type, recent.agent_id,
                    recent.message_count, recent.last_message_preview,
                    recent.recent_run_id, recent.active_run_id, recent.run_state,
                    recent.updated_at, recent.last_active_at, recent.activity_seq,
                    recent.recorded_at
               FROM recent_threads AS recent
               JOIN thread_favorites AS favorite
                 ON favorite.thread_id = recent.thread_id
              ORDER BY recent.activity_seq DESC
              LIMIT ?1",
        )?;
        let rows = stmt.query_map(
            params![i64::try_from(THREAD_FAVORITES_SNAPSHOT_CAP).unwrap_or(i64::MAX)],
            recent_thread_record_from_row,
        )?;
        let mut recent_threads = Vec::new();
        for row in rows {
            recent_threads.push(row?);
        }
        drop(stmt);
        let summaries = if include_summaries {
            let summaries_truncated = page.favorites.len() > THREAD_FAVORITES_SNAPSHOT_CAP;
            let mut stmt = tx.prepare(
                "WITH summary_window AS (
                    SELECT favorite.thread_id,
                           recent.activity_seq,
                           favorite.favorited_at,
                           CASE WHEN recent.thread_id IS NULL THEN 1 ELSE 0 END AS raw_segment
                      FROM thread_favorites AS favorite
                      LEFT JOIN recent_threads AS recent
                        ON recent.thread_id = favorite.thread_id
                     ORDER BY raw_segment ASC,
                              recent.activity_seq DESC,
                              favorite.favorited_at DESC,
                              favorite.thread_id ASC
                     LIMIT ?1
                 )
                 SELECT meta.thread_id, meta.thread_label, meta.workspace_dir,
                        meta.thread_type, meta.provider_type, meta.agent_id,
                        meta.created_at, meta.updated_at, meta.message_count,
                        meta.last_user_message, meta.last_assistant_message,
                        meta.last_message_preview, meta.recent_run_id,
                        meta.active_run_id, meta.worktree_json,
                        meta.excluded_from_recent, meta.sort_updated_at_us
                   FROM summary_window AS member
                   JOIN thread_meta AS meta ON meta.thread_id = member.thread_id
                  WHERE meta.default_list_hidden = 0
                  ORDER BY member.raw_segment ASC,
                           member.activity_seq DESC,
                           member.favorited_at DESC,
                           member.thread_id ASC",
            )?;
            let rows = stmt.query_map(
                params![i64::try_from(THREAD_FAVORITES_SNAPSHOT_CAP).unwrap_or(i64::MAX)],
                thread_summary_row_from_row,
            )?;
            let mut summaries = Vec::new();
            for row in rows {
                summaries.push(row?);
            }
            drop(stmt);
            Some((summaries, summaries_truncated))
        } else {
            None
        };
        tx.commit()?;
        Ok((
            ThreadFavoritesSnapshot {
                page,
                recent_truncated: recent_total > recent_threads.len(),
                recent_total,
                recent_threads,
            },
            summaries,
        ))
    }

    /// Product archive semantics in one transaction: write the tombstone
    /// and delete the record, its projection rows, pin, and favorite together.
    /// Returns whether a record existed. Nothing is left to repair on any
    /// other path — a write racing this transaction either lands before
    /// the tombstone (and is deleted here) or is rejected by the in-tx
    /// tombstone check in `write_thread_record_with_projections`.
    pub(crate) fn archive_thread_record(&self, thread_id: &str) -> GaryxDbResult<bool> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_block_test_db_mutation(TestDbMutationPoint::ArchiveThreadRecord);
        let thread_id = normalize_thread_id(thread_id)?;
        let archived_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO archived_threads (thread_id, archived_at)
             VALUES (?1, ?2)
             ON CONFLICT(thread_id) DO UPDATE SET archived_at = excluded.archived_at",
            params![thread_id, archived_at],
        )?;
        let removed = tx.execute(
            "DELETE FROM thread_records WHERE key = ?1",
            params![thread_id],
        )? > 0;
        remove_thread_meta_projection_tx(&tx, &thread_id)?;
        remove_task_projection_tx(&tx, &thread_id)?;
        remove_recent_thread_tx(&tx, &thread_id)?;
        let removed_pin = tx.execute(
            "DELETE FROM thread_pins WHERE thread_id = ?1",
            params![thread_id],
        )? > 0;
        bump_thread_pins_revision_if_changed_tx(&tx, removed_pin)?;
        let removed_favorite = tx.execute(
            "DELETE FROM thread_favorites WHERE thread_id = ?1",
            params![thread_id],
        )? > 0;
        // Archive tombstones prevent record resurrection, so a missing
        // favorite needs no extra fence; only a changed collection bumps.
        bump_thread_favorites_revision_if_changed_tx(&tx, removed_favorite)?;
        tx.commit()?;
        Ok(removed)
    }

    pub fn is_thread_archived(&self, thread_id: &str) -> GaryxDbResult<bool> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::ArchivedThreadRead)?;
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.read_conn()?;
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
        let conn = self.read_conn()?;
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
        let conn = self.read_conn()?;
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

    pub fn set_capsule_favorite(
        &self,
        id: &str,
        favorited: bool,
    ) -> GaryxDbResult<Option<CapsuleRecord>> {
        let id = normalize_capsule_id(id)?;
        let conn = self.conn()?;
        let updated = if favorited {
            conn.execute(
                "UPDATE capsules
                 SET favorited_at = COALESCE(favorited_at, ?2)
                 WHERE id = ?1",
                params![id, now_string()],
            )?
        } else {
            conn.execute(
                "UPDATE capsules SET favorited_at = NULL WHERE id = ?1",
                params![id],
            )?
        };
        if updated == 0 {
            return Ok(None);
        }
        capsule_by_id(&conn, &id)
    }

    pub fn get_capsule(&self, id: &str) -> GaryxDbResult<Option<CapsuleRecord>> {
        let id = normalize_capsule_id(id)?;
        let conn = self.read_conn()?;
        capsule_by_id(&conn, &id)
    }

    pub fn list_capsules(&self) -> GaryxDbResult<Vec<CapsuleRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, description, thread_id, run_id, agent_id, provider_type,
                    html_sha256, byte_size, revision, created_at, updated_at, favorited_at
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
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, description, thread_id, run_id, agent_id, provider_type,
                    html_sha256, byte_size, revision, created_at, updated_at, favorited_at
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
        Ok(self
            .list_recent_threads_page(RecentThreadTaskFilter::Include, limit, offset)?
            .records)
    }

    pub(crate) fn list_recent_threads_page(
        &self,
        filter: RecentThreadTaskFilter,
        limit: usize,
        requested_offset: usize,
    ) -> GaryxDbResult<RecentThreadDbPage> {
        self.list_recent_threads_page_inner(filter, limit, requested_offset, || Ok(()))
    }

    pub(crate) fn list_recent_threads_keyset_page(
        &self,
        filter: RecentThreadTaskFilter,
        limit: usize,
        before_activity_seq: Option<i64>,
    ) -> GaryxDbResult<RecentThreadKeysetDbPage> {
        self.list_recent_threads_keyset_page_inner(filter, limit, before_activity_seq, || Ok(()))
    }

    pub(crate) fn contains_selectable_recent_thread(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT 1
                   FROM recent_threads
                  WHERE thread_id = ?1 AND thread_type <> 'task'",
                params![thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn list_recent_threads_page_inner<F>(
        &self,
        filter: RecentThreadTaskFilter,
        limit: usize,
        requested_offset: usize,
        after_count: F,
    ) -> GaryxDbResult<RecentThreadDbPage>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let total: i64 = tx.query_row(filter.count_sql(), [], |row| row.get(0))?;
        let total = usize::try_from(total).unwrap_or(usize::MAX);
        let offset = requested_offset.min(total);

        // Test seam for proving that the count and page stay on one WAL read
        // snapshot when a writer commits between the two statements.
        after_count()?;

        let limit_param = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset_param = i64::try_from(offset).unwrap_or(i64::MAX);
        let mut stmt = tx.prepare(filter.page_sql())?;
        let rows = stmt.query_map(
            params![limit_param, offset_param],
            recent_thread_record_from_row,
        )?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        drop(stmt);
        tx.commit()?;

        let has_more = offset.saturating_add(records.len()) < total;
        Ok(RecentThreadDbPage {
            records,
            total,
            offset,
            has_more,
        })
    }

    fn list_recent_threads_keyset_page_inner<F>(
        &self,
        filter: RecentThreadTaskFilter,
        limit: usize,
        before_activity_seq: Option<i64>,
        after_count: F,
    ) -> GaryxDbResult<RecentThreadKeysetDbPage>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let total: i64 = tx.query_row(filter.count_sql(), [], |row| row.get(0))?;
        let total = usize::try_from(total).unwrap_or(usize::MAX);

        // Count and page are display metadata from one WAL snapshot. A
        // concurrent writer may commit here, but this page must not mix it
        // with the earlier total.
        after_count()?;

        let fetch_limit = limit.saturating_add(1);
        let fetch_limit = i64::try_from(fetch_limit).unwrap_or(i64::MAX);
        let mut stmt = tx.prepare(filter.keyset_page_sql(before_activity_seq.is_some()))?;
        let mut rows = match before_activity_seq {
            Some(activity_seq) => stmt.query(params![activity_seq, fetch_limit])?,
            None => stmt.query(params![fetch_limit])?,
        };
        let mut records = Vec::with_capacity(limit.saturating_add(1));
        while let Some(row) = rows.next()? {
            records.push(recent_thread_record_from_row(row)?);
        }
        drop(rows);
        drop(stmt);
        tx.commit()?;

        let has_more = records.len() > limit;
        if has_more {
            records.truncate(limit);
        }
        Ok(RecentThreadKeysetDbPage {
            records,
            total,
            has_more,
        })
    }

    /// Startup crash recovery: the bridge run index is rebuilt empty on
    /// boot, so any projected `active_run_id`/`running` row is a dangling
    /// orphan from the previous process. One SQL pass settles both
    /// projection tables — no store scan, no file reads (#TASK-1864
    /// closing batch; replaces the retired reconcile walk).
    pub fn clear_stale_active_runs(&self) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        // Deliberately does not allocate activity_seq: merely settling a run
        // orphan from the previous boot must not move an old thread to the
        // head. RuntimeAssembler invokes this under the data-dir lock before
        // listener bind; the source guard pins this as a pre-bind-only direct
        // recent_threads UPDATE.
        let recent = conn.execute(
            "UPDATE recent_threads
                SET active_run_id = NULL,
                    run_state = CASE
                        WHEN recent_run_id IS NULL OR recent_run_id = '' THEN 'idle'
                        ELSE 'completed'
                    END
              WHERE active_run_id IS NOT NULL OR run_state = 'running'",
            [],
        )?;
        let meta = conn.execute(
            "UPDATE thread_meta SET active_run_id = NULL WHERE active_run_id IS NOT NULL",
            [],
        )?;
        Ok(recent + meta)
    }

    pub fn count_recent_threads(&self) -> GaryxDbResult<usize> {
        Ok(self
            .list_recent_threads_page(RecentThreadTaskFilter::Include, 0, 0)?
            .total)
    }

    /// Read the import and retirement markers in one SQL query. The boot
    /// importer double-checks this pair after taking the lifecycle lock.
    pub(crate) fn legacy_import_marker_pair(&self) -> GaryxDbResult<(bool, bool)> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyMarkerPairRead)?;
        let conn = self.read_conn()?;
        let pair = conn.query_row(
            "SELECT
                 COALESCE(MAX(CASE
                     WHEN projection_name = ?1 AND projection_version = ?2 THEN 1 ELSE 0
                 END), 0),
                 COALESCE(MAX(CASE
                     WHEN projection_name = ?3 AND projection_version = ?4 THEN 1 ELSE 0
                 END), 0)
               FROM projection_states",
            params![
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
                crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_NAME,
                crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_VERSION,
            ],
            |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)? != 0)),
        )?;
        Ok(pair)
    }

    /// Commit the frozen import marker and the next monotonic import
    /// generation together. Recovery also clears the retirement marker in
    /// this transaction, making `(0,1) -> (1,0)` atomic.
    pub(crate) fn commit_legacy_import(
        &self,
        source_row_count: usize,
        recovery: bool,
    ) -> GaryxDbResult<i64> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyImportCommit)?;
        let source_row_count = i64::try_from(source_row_count).unwrap_or(i64::MAX);
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let generation = match legacy_import_generation_row_tx(&tx)? {
            Some(generation) => generation,
            None => legacy_import_compat_generation_tx(&tx)?,
        };
        let next_generation = generation.checked_add(1).ok_or_else(|| {
            GaryxDbError::Configuration("legacy import generation overflow".to_owned())
        })?;
        record_projection_state_tx(
            &tx,
            crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
            crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
            source_row_count,
            None,
        )?;
        record_projection_state_tx(
            &tx,
            LEGACY_IMPORT_GENERATION_NAME,
            LEGACY_IMPORT_GENERATION_VERSION,
            next_generation,
            None,
        )?;
        if recovery {
            rotate_store_incarnation_tx(&tx)?;
            #[cfg(any(test, feature = "test-seams"))]
            self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyImportAfterIncarnationRotation)?;
            tx.execute(
                "DELETE FROM projection_states WHERE projection_name = ?1",
                params![crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_NAME],
            )?;
        }
        tx.commit()?;
        Ok(next_generation)
    }

    pub(crate) fn record_legacy_archive_retirement(&self) -> GaryxDbResult<()> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyRetirementMarkerWrite)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        record_projection_state_tx(
            &tx,
            crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_NAME,
            crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_VERSION,
            0,
            None,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Current generation for cutover gating. A pre-generation database with
    /// the frozen import marker is lazily seeded to generation 1; a builder
    /// that never ran the boot importer observes generation 0 without
    /// creating a generation row.
    #[cfg(any(test, feature = "test-seams"))]
    pub(crate) fn current_legacy_import_generation(&self) -> GaryxDbResult<i64> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let generation = self.current_legacy_import_generation_tx(&tx)?;
        tx.commit()?;
        Ok(generation)
    }

    fn current_legacy_import_generation_tx(&self, tx: &Transaction<'_>) -> GaryxDbResult<i64> {
        if let Some(generation) = legacy_import_generation_row_tx(tx)? {
            return Ok(generation);
        }
        let generation = legacy_import_compat_generation_tx(tx)?;
        if generation == 1 {
            #[cfg(any(test, feature = "test-seams"))]
            self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyGenerationSeedWrite)?;
            record_projection_state_tx(
                tx,
                LEGACY_IMPORT_GENERATION_NAME,
                LEGACY_IMPORT_GENERATION_VERSION,
                generation,
                None,
            )?;
        }
        Ok(generation)
    }

    fn import_generation_cutover_gate(
        &self,
        tx: &Transaction<'_>,
        migration_name: &str,
        migration_version: i64,
    ) -> GaryxDbResult<(i64, Option<i64>)> {
        let generation = self.current_legacy_import_generation_tx(tx)?;
        let completed = tx
            .query_row(
                "SELECT source_row_count,
                        COALESCE(based_on_import_generation, 1)
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![migration_name, migration_version],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let completed_source_count = completed
            .and_then(|(source_count, based_on)| (based_on == generation).then_some(source_count));
        Ok((generation, completed_source_count))
    }

    /// Run every versioned thread-data migration that must complete after
    /// the one-shot archive import and before the gateway starts serving.
    pub(crate) fn run_thread_data_startup_migrations(&self) -> GaryxDbResult<()> {
        // Destructive cleanup belongs after the boot import, not in schema
        // initialization. GaryxDbService's process-lifetime data-dir lock is
        // already held, and RuntimeAssembler runs this before listener bind.
        {
            let conn = self.conn()?;
            purge_retired_workflow_state(&conn)?;
        }
        self.drop_thread_message_routes_v1()?;
        self.migrate_thread_pin_sort_order_v1()?;
        self.migrate_recent_task_thread_kind_v1()?;
        self.migrate_thread_meta_summary_v1()?;
        self.migrate_recent_thread_activity_seq_v1()?;
        self.migrate_recent_membership_v2()?;
        self.migrate_endpoint_holder_dedup_v1()?;
        Ok(())
    }

    /// Backfill the monotonic recent-thread ordering key exactly once. This
    /// marker is intentionally independent of legacy import generations:
    /// recovery imports use the normal allocator and must never reset either
    /// the marker or the meta high-water mark.
    pub(crate) fn migrate_recent_thread_activity_seq_v1(
        &self,
    ) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let completed_source_count = tx
            .query_row(
                "SELECT source_row_count
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![
                    RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME,
                    RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let source_row_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM recent_threads", [], |row| row.get(0))?;
        let meta_activity_seq: i64 = tx.query_row(
            "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        let existing_max: i64 = tx.query_row(
            "SELECT COALESCE(MAX(activity_seq), 0) FROM recent_threads",
            [],
            |row| row.get(0),
        )?;
        let starting_activity_seq = meta_activity_seq.max(existing_max);
        let final_activity_seq = starting_activity_seq
            .checked_add(source_row_count)
            .filter(|value| *value < MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE)
            .ok_or_else(|| {
                GaryxDbError::Configuration(
                    "recent thread activity sequence space is exhausted".to_owned(),
                )
            })?;

        // Re-running after an explicitly cleared marker remains deterministic
        // and safe even if the prior unique index is still present.
        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_recent_threads_activity_seq;
             DROP INDEX IF EXISTS idx_recent_threads_task_activity_seq;
             DROP INDEX IF EXISTS idx_recent_threads_non_task_activity_seq;",
        )?;

        let thread_ids = {
            let mut stmt = tx.prepare(
                "SELECT thread_id
                   FROM recent_threads
                  ORDER BY last_active_at ASC, thread_id DESC",
            )?;
            stmt.query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for (offset, thread_id) in thread_ids.iter().enumerate() {
            let offset = i64::try_from(offset).unwrap_or(i64::MAX);
            let activity_seq = starting_activity_seq
                .checked_add(offset)
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| {
                    GaryxDbError::Configuration(
                        "recent thread activity sequence space is exhausted".to_owned(),
                    )
                })?;
            // Pre-bind one-shot migration: this direct UPDATE is the sole
            // backfill allow-list entry in addition to pre-bind orphan/type
            // cleanup. Runtime projection writes always use the allocator.
            tx.execute(
                "UPDATE recent_threads SET activity_seq = ?1 WHERE thread_id = ?2",
                params![activity_seq, thread_id],
            )?;
        }
        tx.execute(
            "UPDATE recent_threads_meta SET activity_seq = ?1 WHERE id = 1",
            params![final_activity_seq],
        )?;

        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_recent_threads_last_active;
             DROP INDEX IF EXISTS idx_recent_threads_task_last_active;
             DROP INDEX IF EXISTS idx_recent_threads_non_task_last_active;
             CREATE UNIQUE INDEX idx_recent_threads_activity_seq
                 ON recent_threads(activity_seq DESC);
             CREATE INDEX idx_recent_threads_task_activity_seq
                 ON recent_threads(activity_seq DESC)
                 WHERE thread_type = 'task';
             CREATE INDEX idx_recent_threads_non_task_activity_seq
                 ON recent_threads(activity_seq DESC)
                 WHERE thread_type <> 'task';",
        )?;
        record_projection_state_tx(
            &tx,
            RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME,
            RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION,
            source_row_count,
            None,
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count: thread_ids.len(),
            already_completed: false,
        })
    }

    /// Make hidden the sole recent-membership predicate exactly once per
    /// legacy-import generation. The cutover deliberately bypasses the normal
    /// recent allocator: it freezes the old order, repairs canonical/meta
    /// state, rebuilds the exact visible-live member set, and assigns one new
    /// contiguous sequence range in the same transaction as its marker.
    pub(crate) fn migrate_recent_membership_v2(&self) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (import_generation, completed_source_count) = self.import_generation_cutover_gate(
            &tx,
            RECENT_MEMBERSHIP_MIGRATION_NAME,
            RECENT_MEMBERSHIP_MIGRATION_VERSION,
        )?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        // Registration contract: summary derivation and the first monotonic
        // activity sequence must both precede this membership rewrite. The
        // summary marker is generation-aware, so recovery cannot run S5 over
        // a stale generation's thread_meta rows.
        let summary_prerequisite: bool = tx
            .query_row(
                "SELECT 1
                   FROM projection_states
                  WHERE projection_name = ?1
                    AND projection_version = ?2
                    AND COALESCE(based_on_import_generation, 1) = ?3",
                params![
                    THREAD_META_SUMMARY_MIGRATION_NAME,
                    THREAD_META_SUMMARY_MIGRATION_VERSION,
                    import_generation,
                ],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        let activity_prerequisite: bool = tx
            .query_row(
                "SELECT 1
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![
                    RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME,
                    RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION,
                ],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !summary_prerequisite || !activity_prerequisite {
            return Err(GaryxDbError::Configuration(
                "recent_membership_v2 must run after thread_meta_summary_v1 and recent_thread_activity_seq_v1"
                    .to_owned(),
            ));
        }

        // a. Freeze the pre-cutover membership, its exact ascending sequence
        // order, and H before any insertion. Indexes go immediately because
        // step c intentionally gives every new member the same placeholder.
        let frozen_recent = {
            let mut stmt = tx.prepare(
                "SELECT thread_id, last_active_at
                   FROM recent_threads
                  ORDER BY activity_seq ASC, thread_id ASC",
            )?;
            stmt.query_map([], |row| {
                Ok(FrozenRecentMembershipRow {
                    thread_id: row.get(0)?,
                    last_active_at: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let pre_cutover_recent_ids = frozen_recent
            .iter()
            .map(|row| row.thread_id.clone())
            .collect::<HashSet<_>>();
        let meta_high_water: i64 = tx.query_row(
            "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        let row_high_water: i64 = tx.query_row(
            "SELECT COALESCE(MAX(activity_seq), 0) FROM recent_threads",
            [],
            |row| row.get(0),
        )?;
        let frozen_high_water = meta_high_water.max(row_high_water);
        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_recent_threads_activity_seq;
             DROP INDEX IF EXISTS idx_recent_threads_task_activity_seq;
             DROP INDEX IF EXISTS idx_recent_threads_non_task_activity_seq;",
        )?;

        // b. Normalize every live canonical record, strip all four retired
        // flag paths, and rederive every thread_meta row even when the body is
        // byte-identical. Do not touch recent_threads in this phase.
        let canonical_rows = {
            let mut stmt = tx.prepare(
                "SELECT record.key, record.body
                   FROM thread_records AS record
                  WHERE substr(record.key, 1, 8) = 'thread::'
                    AND NOT EXISTS (
                        SELECT 1
                          FROM archived_threads AS archived
                         WHERE archived.thread_id = record.key
                    )
                  ORDER BY record.key ASC",
            )?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let source_row_count = i64::try_from(canonical_rows.len()).unwrap_or(i64::MAX);
        let projected_at = now_string();
        let mut canonical_updated_count = 0usize;
        let mut target_drafts = Vec::with_capacity(canonical_rows.len());
        for (thread_id, body) in canonical_rows {
            let mut data: Value = serde_json::from_str(&body).map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "recent membership cutover could not decode {thread_id}: {error}"
                ))
            })?;
            let canonical_changed = normalize_recent_membership_canonical_record(&mut data);
            if canonical_changed {
                let normalized_body = serde_json::to_string(&data).map_err(|error| {
                    GaryxDbError::Configuration(format!(
                        "recent membership cutover could not encode {thread_id}: {error}"
                    ))
                })?;
                canonical_updated_count += tx.execute(
                    "UPDATE thread_records SET body = ?1 WHERE key = ?2",
                    params![normalized_body, thread_id],
                )?;
            }

            let projection = crate::thread_meta_projection::
                thread_meta_projection_from_thread_data_with_active_run(&thread_id, &data, None)
                .ok_or_else(|| {
                    GaryxDbError::Configuration(format!(
                        "recent membership cutover rejected canonical id {thread_id}"
                    ))
                })?;
            upsert_thread_meta(&tx, &projection.thread_meta, &projected_at)?;
            if let Some(draft) = crate::recent_thread_projection::
                recent_thread_draft_from_thread_data_with_active_run(&thread_id, &data, None)
            {
                target_drafts.push(draft);
            }
        }
        tx.execute("UPDATE thread_meta SET excluded_from_recent = 0", [])?;
        let nonzero_excluded_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM thread_meta WHERE excluded_from_recent != 0",
            [],
            |row| row.get(0),
        )?;
        if nonzero_excluded_count != 0 {
            return Err(GaryxDbError::Configuration(
                "recent membership cutover left nonzero excluded_from_recent rows".to_owned(),
            ));
        }

        let target_ids = target_drafts
            .iter()
            .map(|draft| draft.thread_id.clone())
            .collect::<HashSet<_>>();
        let target_count = i64::try_from(target_drafts.len()).unwrap_or(i64::MAX);
        let final_high_water = frozen_high_water
            .checked_add(target_count)
            .filter(|value| *value < MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE)
            .ok_or_else(|| {
                GaryxDbError::Configuration(
                    "recent thread activity sequence space is exhausted".to_owned(),
                )
            })?;

        // c. Exact target membership: add target-pre with a shared placeholder
        // and remove every pre-target orphan/hidden row. The indexes were
        // already dropped in step a, so multiple placeholder zeroes are valid.
        let mut new_drafts = target_drafts
            .iter()
            .filter(|draft| !pre_cutover_recent_ids.contains(&draft.thread_id))
            .cloned()
            .collect::<Vec<_>>();
        for draft in &new_drafts {
            insert_recent_membership_placeholder_tx(&tx, draft, &projected_at)?;
        }
        let mut removed_recent_count = 0usize;
        for row in &frozen_recent {
            if !target_ids.contains(&row.thread_id) {
                removed_recent_count += tx.execute(
                    "DELETE FROM recent_threads WHERE thread_id = ?1",
                    params![row.thread_id],
                )?;
            }
        }

        // d. Existing members retain their exact frozen relative order.
        // New members are bucketed by the count of retained rows whose
        // timestamp/id key is smaller; the retained rows are never sorted by
        // timestamp, which avoids cycles when old timestamps are inverted.
        let retained_existing_order = frozen_recent
            .iter()
            .filter(|row| target_ids.contains(&row.thread_id))
            .cloned()
            .collect::<Vec<_>>();
        new_drafts.sort_by(|left, right| {
            compare_recent_membership_order(
                &left.last_active_at,
                &left.thread_id,
                &right.last_active_at,
                &right.thread_id,
            )
        });
        let mut insertion_buckets = vec![Vec::<String>::new(); retained_existing_order.len() + 1];
        for draft in &new_drafts {
            let insertion_index = retained_existing_order
                .iter()
                .filter(|existing| {
                    compare_recent_membership_order(
                        &existing.last_active_at,
                        &existing.thread_id,
                        &draft.last_active_at,
                        &draft.thread_id,
                    ) == CmpOrdering::Less
                })
                .count();
            insertion_buckets[insertion_index].push(draft.thread_id.clone());
        }
        let mut final_order = Vec::with_capacity(target_drafts.len());
        for index in 0..=retained_existing_order.len() {
            final_order.append(&mut insertion_buckets[index]);
            if let Some(existing) = retained_existing_order.get(index) {
                final_order.push(existing.thread_id.clone());
            }
        }
        if final_order.len() != target_drafts.len() {
            return Err(GaryxDbError::Configuration(
                "recent membership cutover produced an incomplete order".to_owned(),
            ));
        }
        for (offset, thread_id) in final_order.iter().enumerate() {
            let offset = i64::try_from(offset).unwrap_or(i64::MAX);
            let activity_seq = frozen_high_water
                .checked_add(offset)
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| {
                    GaryxDbError::Configuration(
                        "recent thread activity sequence space is exhausted".to_owned(),
                    )
                })?;
            let updated = tx.execute(
                "UPDATE recent_threads SET activity_seq = ?1 WHERE thread_id = ?2",
                params![activity_seq, thread_id],
            )?;
            if updated != 1 {
                return Err(GaryxDbError::Configuration(format!(
                    "recent membership cutover lost target row {thread_id}"
                )));
            }
        }

        // e. Publish the new high-water mark, recreate all three ordering
        // indexes, and verify the row/max invariants before the marker lands.
        let updated_meta = tx.execute(
            "UPDATE recent_threads_meta SET activity_seq = ?1 WHERE id = 1",
            params![final_high_water],
        )?;
        if updated_meta != 1 {
            return Err(GaryxDbError::Configuration(
                "recent_threads_meta singleton is missing".to_owned(),
            ));
        }
        tx.execute_batch(
            "CREATE UNIQUE INDEX idx_recent_threads_activity_seq
                 ON recent_threads(activity_seq DESC);
             CREATE INDEX idx_recent_threads_task_activity_seq
                 ON recent_threads(activity_seq DESC)
                 WHERE thread_type = 'task';
             CREATE INDEX idx_recent_threads_non_task_activity_seq
                 ON recent_threads(activity_seq DESC)
                 WHERE thread_type <> 'task';",
        )?;
        let (actual_count, actual_max): (i64, i64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(MAX(activity_seq), 0) FROM recent_threads",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if actual_count != target_count
            || (actual_count > 0 && actual_max != final_high_water)
            || (actual_count == 0 && final_high_water != frozen_high_water)
        {
            return Err(GaryxDbError::Configuration(
                "recent membership cutover high-water/member count mismatch".to_owned(),
            ));
        }
        assert_recent_membership_parity_tx(&tx)?;

        // f. Data and generation-aware marker commit atomically.
        record_projection_state_tx(
            &tx,
            RECENT_MEMBERSHIP_MIGRATION_NAME,
            RECENT_MEMBERSHIP_MIGRATION_VERSION,
            source_row_count,
            Some(import_generation),
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count: canonical_updated_count
                .saturating_add(new_drafts.len())
                .saturating_add(removed_recent_count)
                .saturating_add(final_order.len()),
            already_completed: false,
        })
    }

    pub(crate) fn drop_thread_message_routes_v1(&self) -> GaryxDbResult<OneShotMigrationSummary> {
        self.drop_thread_message_routes_v1_inner(|_| Ok(()))
    }

    fn drop_thread_message_routes_v1_inner<F>(
        &self,
        after_drop: F,
    ) -> GaryxDbResult<OneShotMigrationSummary>
    where
        F: FnOnce(&Transaction<'_>) -> GaryxDbResult<()>,
    {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let completed_source_count = tx
            .query_row(
                "SELECT source_row_count
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![
                    DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME,
                    DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let table_exists = tx
            .query_row(
                "SELECT 1 FROM sqlite_master
                  WHERE type = 'table' AND name = 'thread_message_routes'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let source_row_count = if table_exists { 1 } else { 0 };

        tx.execute("DROP TABLE IF EXISTS thread_message_routes", [])?;
        after_drop(&tx)?;
        record_projection_state_tx(
            &tx,
            DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME,
            DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION,
            source_row_count,
            None,
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count: usize::from(table_exists),
            already_completed: false,
        })
    }

    pub(crate) fn migrate_thread_pin_sort_order_v1(
        &self,
    ) -> GaryxDbResult<OneShotMigrationSummary> {
        self.migrate_thread_pin_sort_order_v1_inner(|_| Ok(()))
    }

    fn migrate_thread_pin_sort_order_v1_inner<F>(
        &self,
        after_backfill: F,
    ) -> GaryxDbResult<OneShotMigrationSummary>
    where
        F: FnOnce(&Transaction<'_>) -> GaryxDbResult<()>,
    {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let completed_source_count = tx
            .query_row(
                "SELECT source_row_count
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![
                    THREAD_PIN_SORT_ORDER_MIGRATION_NAME,
                    THREAD_PIN_SORT_ORDER_MIGRATION_VERSION
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let source_row_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM thread_pins", [], |row| row.get(0))?;
        let updated_row_count = tx.execute(
            "WITH ranked AS (
                 SELECT thread_id,
                        ROW_NUMBER() OVER (
                            ORDER BY pinned_at DESC, thread_id ASC
                        ) - 1 AS next_sort_order
                   FROM thread_pins
             )
             UPDATE thread_pins
                SET sort_order = (
                    SELECT next_sort_order
                      FROM ranked
                     WHERE ranked.thread_id = thread_pins.thread_id
                )",
            [],
        )?;

        after_backfill(&tx)?;

        tx.execute(
            "INSERT INTO projection_states (
                projection_name, projection_version, source_row_count, projected_at
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(projection_name) DO UPDATE SET
                projection_version = excluded.projection_version,
                source_row_count = excluded.source_row_count,
                projected_at = excluded.projected_at",
            params![
                THREAD_PIN_SORT_ORDER_MIGRATION_NAME,
                THREAD_PIN_SORT_ORDER_MIGRATION_VERSION,
                source_row_count,
                now_string(),
            ],
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    /// Establish the canonical invariant that one endpoint appears on at
    /// most one thread record. Winner selection exactly follows the existing
    /// preference order: parsed timestamp, raw timestamp, then thread id.
    /// Canonical JSON and the endpoint projection are rewritten in one
    /// transaction so no ghost holder can survive the cutover to point reads.
    pub(crate) fn migrate_endpoint_holder_dedup_v1(
        &self,
    ) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (import_generation, completed_source_count) = self.import_generation_cutover_gate(
            &tx,
            ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
            ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION,
        )?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        tx.execute_batch(
            "DROP TABLE IF EXISTS temp.endpoint_holder_dedup_rows;
             DROP TABLE IF EXISTS temp.endpoint_holder_dedup_winners;
             CREATE TEMP TABLE endpoint_holder_dedup_rows (
                 thread_id TEXT NOT NULL,
                 binding_index INTEGER NOT NULL,
                 endpoint_key TEXT NOT NULL,
                 channel TEXT NOT NULL,
                 account_id TEXT NOT NULL,
                 binding_key TEXT NOT NULL,
                 chat_id TEXT NOT NULL,
                 delivery_target_type TEXT NOT NULL,
                 delivery_target_id TEXT NOT NULL,
                 display_label TEXT NOT NULL,
                 last_inbound_at TEXT,
                 last_delivery_at TEXT,
                 thread_label TEXT,
                 workspace_dir TEXT,
                 thread_updated_at TEXT NOT NULL
             ) STRICT;
             CREATE TEMP TABLE endpoint_holder_dedup_winners (
                 endpoint_key TEXT PRIMARY KEY,
                 thread_id TEXT NOT NULL
             ) STRICT;",
        )?;
        tx.execute(
            "INSERT INTO endpoint_holder_dedup_rows (
                 thread_id, binding_index, endpoint_key, channel, account_id,
                 binding_key, chat_id, delivery_target_type, delivery_target_id,
                 display_label, last_inbound_at, last_delivery_at, thread_label,
                 workspace_dir, thread_updated_at
             )
             SELECT record.key,
                    CAST(binding.key AS INTEGER),
                    COALESCE(json_extract(binding.value, '$.channel'), '') || '::' ||
                    COALESCE(json_extract(binding.value, '$.account_id'), '') || '::' ||
                    trim(CASE
                        WHEN json_type(binding.value, '$.binding_key') = 'text'
                            THEN json_extract(binding.value, '$.binding_key')
                        WHEN json_type(binding.value, '$.thread_scope') = 'text'
                            THEN json_extract(binding.value, '$.thread_scope')
                        WHEN json_type(binding.value, '$.peer_id') = 'text'
                            THEN json_extract(binding.value, '$.peer_id')
                        ELSE ''
                    END),
                    COALESCE(json_extract(binding.value, '$.channel'), ''),
                    COALESCE(json_extract(binding.value, '$.account_id'), ''),
                    trim(CASE
                        WHEN json_type(binding.value, '$.binding_key') = 'text'
                            THEN json_extract(binding.value, '$.binding_key')
                        WHEN json_type(binding.value, '$.thread_scope') = 'text'
                            THEN json_extract(binding.value, '$.thread_scope')
                        WHEN json_type(binding.value, '$.peer_id') = 'text'
                            THEN json_extract(binding.value, '$.peer_id')
                        ELSE ''
                    END),
                    trim(COALESCE(json_extract(binding.value, '$.chat_id'), '')),
                    trim(COALESCE(json_extract(binding.value, '$.delivery_target_type'), '')),
                    trim(COALESCE(json_extract(binding.value, '$.delivery_target_id'), '')),
                    trim(COALESCE(json_extract(binding.value, '$.display_label'), '')),
                    CASE WHEN json_type(binding.value, '$.last_inbound_at') = 'text'
                         THEN json_extract(binding.value, '$.last_inbound_at') END,
                    CASE WHEN json_type(binding.value, '$.last_delivery_at') = 'text'
                         THEN json_extract(binding.value, '$.last_delivery_at') END,
                    CASE WHEN json_type(record.body, '$.label') = 'text'
                         THEN json_extract(record.body, '$.label') END,
                    CASE WHEN json_type(record.body, '$.workspace_dir') = 'text'
                         THEN json_extract(record.body, '$.workspace_dir') END,
                    CASE WHEN json_type(record.body, '$.updated_at') = 'text'
                         THEN json_extract(record.body, '$.updated_at') ELSE '' END
               FROM thread_records AS record,
                    json_each(json_extract(record.body, '$.channel_bindings')) AS binding
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND json_type(binding.value) = 'object'
                AND (json_type(binding.value, '$.channel') IS NULL OR
                     json_type(binding.value, '$.channel') = 'text')
                AND (json_type(binding.value, '$.account_id') IS NULL OR
                     json_type(binding.value, '$.account_id') = 'text')
                AND (json_type(binding.value, '$.binding_key') IS NULL OR
                     json_type(binding.value, '$.binding_key') = 'text')
                AND (json_type(binding.value, '$.chat_id') IS NULL OR
                     json_type(binding.value, '$.chat_id') = 'text')
                AND (json_type(binding.value, '$.delivery_target_type') IS NULL OR
                     json_type(binding.value, '$.delivery_target_type') = 'text')
                AND (json_type(binding.value, '$.delivery_target_id') IS NULL OR
                     json_type(binding.value, '$.delivery_target_id') = 'text')
                AND (json_type(binding.value, '$.display_label') IS NULL OR
                     json_type(binding.value, '$.display_label') = 'text')
                AND (json_type(binding.value, '$.last_inbound_at') IS NULL OR
                     json_type(binding.value, '$.last_inbound_at') = 'text')
                AND (json_type(binding.value, '$.last_delivery_at') IS NULL OR
                     json_type(binding.value, '$.last_delivery_at') = 'text')",
            [],
        )?;
        tx.execute(
            "INSERT INTO endpoint_holder_dedup_winners (endpoint_key, thread_id)
             SELECT endpoint_key, thread_id
               FROM (
                   SELECT endpoint_key,
                          thread_id,
                          ROW_NUMBER() OVER (
                              PARTITION BY endpoint_key
                              ORDER BY
                                  CASE
                                      WHEN thread_updated_at GLOB
                                           '????-??-??T??:??:??*'
                                           AND julianday(thread_updated_at) IS NOT NULL
                                      THEN 1 ELSE 0
                                  END DESC,
                                  CASE
                                      WHEN thread_updated_at GLOB
                                           '????-??-??T??:??:??*'
                                      THEN julianday(thread_updated_at)
                                  END DESC,
                                  thread_updated_at DESC,
                                  thread_id DESC
                          ) AS preference_rank
                     FROM endpoint_holder_dedup_rows
               )
              WHERE preference_rank = 1",
            [],
        )?;
        let source_row_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM endpoint_holder_dedup_rows",
            [],
            |row| row.get(0),
        )?;

        let updated_row_count = tx.execute(
            "UPDATE thread_records
                SET body = json_set(
                    body,
                    '$.channel_bindings',
                    json(COALESCE((
                        SELECT json_group_array(json(binding.value))
                          FROM json_each(
                              json_extract(thread_records.body, '$.channel_bindings')
                          ) AS binding
                         WHERE NOT EXISTS (
                             SELECT 1
                               FROM endpoint_holder_dedup_rows AS holder
                               JOIN endpoint_holder_dedup_winners AS winner
                                 ON winner.endpoint_key = holder.endpoint_key
                              WHERE holder.thread_id = thread_records.key
                                AND holder.binding_index = CAST(binding.key AS INTEGER)
                                AND winner.thread_id <> holder.thread_id
                         )
                    ), '[]'))
                )
              WHERE key IN (
                  SELECT DISTINCT holder.thread_id
                    FROM endpoint_holder_dedup_rows AS holder
                    JOIN endpoint_holder_dedup_winners AS winner
                      ON winner.endpoint_key = holder.endpoint_key
                   WHERE winner.thread_id <> holder.thread_id
              )",
            [],
        )?;

        tx.execute("DELETE FROM thread_channel_endpoints", [])?;
        tx.execute(
            "INSERT OR REPLACE INTO thread_channel_endpoints (
                 endpoint_key, channel, account_id, binding_key, chat_id,
                 delivery_target_type, delivery_target_id, display_label,
                 thread_id, thread_label, workspace_dir, thread_updated_at,
                 last_inbound_at, last_delivery_at, projected_at
             )
             SELECT holder.endpoint_key,
                    holder.channel,
                    holder.account_id,
                    holder.binding_key,
                    holder.chat_id,
                    CASE
                        WHEN holder.delivery_target_id <> '' THEN
                            CASE WHEN holder.delivery_target_type = 'open_id'
                                 THEN 'open_id' ELSE 'chat_id' END
                        WHEN holder.channel = 'feishu'
                             AND holder.chat_id <> ''
                             AND holder.chat_id = holder.binding_key
                             AND holder.chat_id LIKE 'ou_%'
                        THEN 'open_id'
                        ELSE 'chat_id'
                    END,
                    CASE
                        WHEN holder.delivery_target_id <> ''
                        THEN holder.delivery_target_id
                        WHEN holder.channel = 'feishu'
                             AND holder.chat_id <> ''
                             AND holder.chat_id = holder.binding_key
                             AND holder.chat_id LIKE 'ou_%'
                        THEN CASE WHEN holder.binding_key <> ''
                                  THEN holder.binding_key ELSE holder.chat_id END
                        ELSE CASE WHEN holder.chat_id <> ''
                                  THEN holder.chat_id ELSE holder.binding_key END
                    END,
                    holder.display_label,
                    holder.thread_id,
                    NULLIF(trim(holder.thread_label), ''),
                    NULLIF(trim(holder.workspace_dir), ''),
                    NULLIF(trim(holder.thread_updated_at), ''),
                    NULLIF(trim(holder.last_inbound_at), ''),
                    NULLIF(trim(holder.last_delivery_at), ''),
                    ?1
               FROM endpoint_holder_dedup_rows AS holder
               JOIN endpoint_holder_dedup_winners AS winner
                 ON winner.endpoint_key = holder.endpoint_key
                AND winner.thread_id = holder.thread_id
              ORDER BY holder.thread_id ASC, holder.binding_index ASC",
            params![now_string()],
        )?;
        record_projection_state_tx(
            &tx,
            ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
            ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION,
            source_row_count,
            Some(import_generation),
        )?;
        tx.execute_batch(
            "DROP TABLE endpoint_holder_dedup_winners;
             DROP TABLE endpoint_holder_dedup_rows;",
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    /// Persist task identity on legacy backing threads. The migration is a
    /// one-shot, set-based transaction: canonical bodies and both type
    /// projections move together, while activity timestamps and titles stay
    /// untouched.
    pub(crate) fn migrate_recent_task_thread_kind_v1(
        &self,
    ) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (import_generation, completed_source_count) = self.import_generation_cutover_gate(
            &tx,
            RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
            RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
        )?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let source_row_count: i64 = tx.query_row(
            "SELECT COUNT(*)
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND (
                    json_extract(record.body, '$.thread_kind') = 'task'
                    OR json_extract(record.body, '$.thread_title_source') = 'task'
                    OR EXISTS (
                        SELECT 1
                          FROM task_projection AS task
                         WHERE task.thread_id = record.key
                    )
                )",
            [],
            |row| row.get(0),
        )?;

        let updated_row_count = tx.execute(
            "UPDATE thread_records
                SET body = json_set(body, '$.thread_kind', 'task')
              WHERE substr(key, 1, 8) = 'thread::'
                AND (
                    json_extract(body, '$.thread_kind') = 'task'
                    OR json_extract(body, '$.thread_title_source') = 'task'
                    OR EXISTS (
                        SELECT 1
                          FROM task_projection AS task
                         WHERE task.thread_id = thread_records.key
                    )
                )
                AND COALESCE(json_extract(body, '$.thread_kind'), '') <> 'task'",
            [],
        )?;
        // Pre-bind one-shot projection correction. Changing the persisted
        // thread kind is not user activity, so it intentionally preserves
        // activity_seq rather than moving the row to the head.
        tx.execute(
            "UPDATE recent_threads
                SET thread_type = 'task'
              WHERE thread_id IN (
                    SELECT key
                      FROM thread_records
                     WHERE substr(key, 1, 8) = 'thread::'
                       AND json_extract(body, '$.thread_kind') = 'task'
                )
                AND thread_type <> 'task'",
            [],
        )?;
        tx.execute(
            "UPDATE thread_meta
                SET thread_type = 'task'
              WHERE thread_id IN (
                    SELECT key
                      FROM thread_records
                     WHERE substr(key, 1, 8) = 'thread::'
                       AND json_extract(body, '$.thread_kind') = 'task'
                )
                AND thread_type <> 'task'",
            [],
        )?;
        record_projection_state_tx(
            &tx,
            RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
            RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
            source_row_count,
            Some(import_generation),
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    /// Backfill the three list-summary columns from canonical thread records
    /// exactly once per legacy-import generation. Normal writes derive the
    /// same fields before entering the record/projection transaction.
    pub(crate) fn migrate_thread_meta_summary_v1(&self) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (import_generation, completed_source_count) = self.import_generation_cutover_gate(
            &tx,
            THREAD_META_SUMMARY_MIGRATION_NAME,
            THREAD_META_SUMMARY_MIGRATION_VERSION,
        )?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let source_rows = {
            let mut stmt = tx.prepare(
                "SELECT meta.thread_id, record.body
                   FROM thread_meta AS meta
                   LEFT JOIN thread_records AS record ON record.key = meta.thread_id
                  ORDER BY meta.thread_id ASC",
            )?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let source_row_count = i64::try_from(source_rows.len()).unwrap_or(i64::MAX);

        let mut updated_row_count = 0usize;
        for (thread_id, body) in source_rows {
            let body = body.ok_or_else(|| {
                GaryxDbError::Configuration(
                    "thread_meta summary cutover found a projection without a canonical record"
                        .to_owned(),
                )
            })?;
            let data: Value = serde_json::from_str(&body).map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "thread_meta summary cutover could not decode {thread_id}: {error}"
                ))
            })?;
            let projection = crate::thread_meta_projection::
                thread_meta_projection_from_thread_data_with_active_run(&thread_id, &data, None)
                .ok_or_else(|| {
                    GaryxDbError::Configuration(format!(
                        "thread_meta summary cutover rejected canonical id {thread_id}"
                    ))
                })?;
            updated_row_count += tx.execute(
                "UPDATE thread_meta
                    SET excluded_from_recent = ?1,
                        sort_updated_at_us = ?2,
                        search_text = ?3
                  WHERE thread_id = ?4",
                params![
                    if projection.thread_meta.excluded_from_recent {
                        1
                    } else {
                        0
                    },
                    projection.thread_meta.sort_updated_at_us,
                    projection.thread_meta.search_text,
                    thread_id,
                ],
            )?;
        }
        record_projection_state_tx(
            &tx,
            THREAD_META_SUMMARY_MIGRATION_NAME,
            THREAD_META_SUMMARY_MIGRATION_VERSION,
            source_row_count,
            Some(import_generation),
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    pub fn projection_state_matches(
        &self,
        projection_name: &str,
        projection_version: i64,
        source_row_count: usize,
    ) -> GaryxDbResult<bool> {
        let projection_name = normalize_required("projection_name", projection_name)?;
        let source_row_count = i64::try_from(source_row_count).unwrap_or(i64::MAX);
        let conn = self.read_conn()?;
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

    /// Whether a projection/migration state row exists at the given
    /// version, regardless of its recorded source count. The sqlite
    /// thread-record import gates on existence alone: in steady state new
    /// threads change the key count, and a count-sensitive gate would
    /// re-import on every boot — flowing the stale file archive back over
    /// the SQL truth (#TASK-1864 batch 2 on-device finding). Clearing the
    /// state row is the only event that forces a re-import.
    pub fn projection_state_exists(&self, name: &str, version: i64) -> GaryxDbResult<bool> {
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![name, version],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Drop a projection/migration state row so its one-shot job runs
    /// again on the next eligible boot. Manual recovery hook: clearing
    /// the thread-records import row forces a fresh boot import from the
    /// archived source (review #TASK-1901: a same-key-count rewrite must
    /// not be skipped by the next import).
    pub fn clear_projection_state(&self, name: &str) -> GaryxDbResult<bool> {
        if name == LEGACY_IMPORT_GENERATION_NAME {
            return Err(GaryxDbError::BadRequest(
                "legacy_import_generation is monotonic and cannot be cleared".to_owned(),
            ));
        }
        let conn = self.conn()?;
        let removed = conn.execute(
            "DELETE FROM projection_states WHERE projection_name = ?1",
            params![name],
        )?;
        Ok(removed > 0)
    }

    pub fn record_projection_state(
        &self,
        projection_name: &str,
        projection_version: i64,
        source_row_count: usize,
    ) -> GaryxDbResult<()> {
        let projection_name = normalize_required("projection_name", projection_name)?;
        if projection_name == LEGACY_IMPORT_GENERATION_NAME {
            return Err(GaryxDbError::BadRequest(
                "legacy_import_generation is owned by the boot importer".to_owned(),
            ));
        }
        let source_row_count = i64::try_from(source_row_count).unwrap_or(i64::MAX);
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        record_projection_state_tx(
            &tx,
            &projection_name,
            projection_version,
            source_row_count,
            None,
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_recent_thread(
        &self,
        draft: RecentThreadDraft,
    ) -> GaryxDbResult<RecentThreadRecord> {
        let recorded_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let record = upsert_recent_thread_tx(&tx, draft, &recorded_at)?;
        tx.commit()?;
        Ok(record)
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
        let conn = self.read_conn()?;
        let count: i64 = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM thread_meta) +
                (SELECT COUNT(*) FROM thread_channel_endpoints)",
            [],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn count_thread_meta_rows(&self) -> GaryxDbResult<usize> {
        let conn = self.read_conn()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM thread_meta", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn count_thread_channel_endpoints(&self) -> GaryxDbResult<usize> {
        let conn = self.read_conn()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM thread_channel_endpoints", [], |row| {
                row.get(0)
            })?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn list_thread_channel_endpoints(&self) -> GaryxDbResult<Vec<KnownChannelEndpoint>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT endpoint_key, channel, account_id, binding_key, chat_id,
                    delivery_target_type, delivery_target_id, display_label,
                    thread_id, thread_label, workspace_dir, thread_updated_at,
                    last_inbound_at, last_delivery_at
             FROM thread_channel_endpoints
             ORDER BY endpoint_key ASC",
        )?;
        let rows = stmt.query_map([], known_channel_endpoint_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub(crate) fn get_thread_channel_endpoint(
        &self,
        endpoint_key: &str,
    ) -> GaryxDbResult<Option<KnownChannelEndpoint>> {
        let endpoint_key = normalize_required("endpoint_key", endpoint_key)?;
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT endpoint_key, channel, account_id, binding_key, chat_id,
                        delivery_target_type, delivery_target_id, display_label,
                        thread_id, thread_label, workspace_dir, thread_updated_at,
                        last_inbound_at, last_delivery_at
                   FROM thread_channel_endpoints
                  WHERE endpoint_key = ?1",
                params![endpoint_key],
                known_channel_endpoint_from_row,
            )
            .optional()?)
    }

    /// Per-thread persisted delivery contexts from the thread_meta projection.
    pub fn list_thread_delivery_contexts(
        &self,
    ) -> GaryxDbResult<Vec<(String, String, Option<String>)>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT thread_id, last_delivery_context_json, last_delivery_updated_at
             FROM thread_meta
             WHERE last_delivery_context_json IS NOT NULL
             ORDER BY thread_id ASC",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
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
        let conn = self.read_conn()?;
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
        let conn = self.read_conn()?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let sql = "SELECT thread_id, workspace_dir, thread_type, thread_label, agent_id,
                          provider_type, created_at, updated_at, message_count,
                          last_user_message, last_assistant_message, last_message_preview,
                          recent_run_id, active_run_id, worktree_json,
                          last_delivery_context_json, last_delivery_updated_at,
                          default_list_hidden, excluded_from_recent, sort_updated_at_us,
                          search_text, provider_key, selected_model,
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
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT thread_id, workspace_dir, thread_type, thread_label, agent_id,
                    provider_type, created_at, updated_at, message_count,
                    last_user_message, last_assistant_message, last_message_preview,
                    recent_run_id, active_run_id, worktree_json,
                    last_delivery_context_json, last_delivery_updated_at,
                    default_list_hidden, excluded_from_recent, sort_updated_at_us,
                    search_text, provider_key, selected_model,
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

    pub(crate) fn list_thread_summaries_keyset_page(
        &self,
        filter: ThreadSummaryTaskFilter,
        workspace_dir: Option<&str>,
        query: Option<&str>,
        limit: usize,
        before: Option<(i64, &str)>,
        expected_store_incarnation: Option<&str>,
    ) -> GaryxDbResult<ThreadSummaryDbPage> {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let store_incarnation_id = read_store_incarnation_id(&tx)?;
        if expected_store_incarnation.is_some_and(|expected| expected != store_incarnation_id) {
            return Err(GaryxDbError::BadRequest(
                "cursor does not belong to the current store incarnation".to_owned(),
            ));
        }

        let mut bind = Vec::with_capacity(6);
        if let Some(workspace_dir) = workspace_dir {
            bind.push(SqlValue::Text(workspace_dir.to_owned()));
        }
        if let Some(query) = query {
            bind.push(SqlValue::Text(query.to_owned()));
        }
        if let Some((sort_updated_at_us, thread_id)) = before {
            bind.push(SqlValue::Integer(sort_updated_at_us));
            bind.push(SqlValue::Text(thread_id.to_owned()));
        }
        let fetch_limit = limit.saturating_add(1);
        bind.push(SqlValue::Integer(
            i64::try_from(fetch_limit).unwrap_or(i64::MAX),
        ));

        let sql = filter.page_sql(workspace_dir.is_some(), query.is_some(), before.is_some());
        let mut stmt = tx.prepare(sql)?;
        let rows = stmt.query_map(params_from_iter(bind.iter()), thread_summary_row_from_row)?;
        let mut records = Vec::with_capacity(fetch_limit);
        for row in rows {
            records.push(row?);
        }
        drop(stmt);
        tx.commit()?;

        let has_more = records.len() > limit;
        if has_more {
            records.truncate(limit);
        }
        Ok(ThreadSummaryDbPage {
            records,
            has_more,
            store_incarnation_id,
        })
    }

    /// Test-fixture seeding only: production thread_meta rows derive in
    /// the same transaction as the record write
    /// (`write_thread_record_with_projections`).
    #[cfg(test)]
    pub fn replace_thread_meta_projection(
        &self,
        draft: ThreadMetaProjectionDraft,
    ) -> GaryxDbResult<()> {
        let recorded_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        replace_thread_meta_projection_tx(&tx, draft, &recorded_at)?;
        tx.commit()?;
        Ok(())
    }

    /// Single-transaction write of a thread record plus its derived
    /// projections (#TASK-1864 batch 2, D2): the record and the five
    /// projection tables commit or roll back together, so projection drift
    /// is structurally impossible. `projections: None` writes the record
    /// only (non-thread keys such as `meta::`/`cron::`/`tool::`).
    pub fn write_thread_record_with_projections(
        &self,
        key: &str,
        body: &str,
        updated_at: Option<&str>,
        projections: Option<ThreadRecordProjections>,
    ) -> GaryxDbResult<()> {
        let recorded_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        write_thread_record_with_projections_tx(
            &tx,
            key,
            body,
            updated_at,
            projections,
            &recorded_at,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// All-or-nothing write of MULTIPLE thread records plus their derived
    /// projections in one transaction (#TASK-2099 root final review):
    /// endpoint binding mutations touch the previous owner, the target,
    /// and the known-endpoint registry together — either every record and
    /// projection commits or none do, so a mid-mutation storage failure
    /// can never lose the active binding.
    pub fn write_thread_records_with_projections_atomic(
        &self,
        entries: Vec<ThreadRecordWrite>,
    ) -> GaryxDbResult<()> {
        let recorded_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        for entry in entries {
            write_thread_record_with_projections_tx(
                &tx,
                &entry.key,
                &entry.body,
                entry.updated_at.as_deref(),
                entry.projections,
                &recorded_at,
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Single-transaction delete of a thread record, all its projection
    /// rows, pin, and favorite. Returns whether the record existed.
    pub(crate) fn delete_thread_record_with_projections(&self, key: &str) -> GaryxDbResult<bool> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_block_test_db_mutation(TestDbMutationPoint::DeleteThreadRecord);
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::DeleteThreadRecord)?;
        let key = normalize_required("key", key)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let removed = tx.execute("DELETE FROM thread_records WHERE key = ?1", params![key])? > 0;
        remove_thread_meta_projection_tx(&tx, &key)?;
        remove_task_projection_tx(&tx, &key)?;
        remove_recent_thread_tx(&tx, &key)?;
        let removed_pin =
            tx.execute("DELETE FROM thread_pins WHERE thread_id = ?1", params![key])? > 0;
        bump_thread_pins_revision_if_changed_tx(&tx, removed_pin)?;
        tx.execute(
            "DELETE FROM thread_favorites WHERE thread_id = ?1",
            params![key],
        )?;
        // Ordinary delete has no durable tombstone. Every successful thread
        // deletion advances the fence even when there was no favorite row,
        // so a pre-delete orphan write cannot land after record recreation.
        if removed && is_thread_key(&key) {
            bump_thread_favorites_revision_tx(&tx)?;
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Point read of a record body from the reader connection (WAL snapshot
    /// read — never queued behind the writer).
    pub fn get_thread_record_body(&self, key: &str) -> GaryxDbResult<Option<String>> {
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT body FROM thread_records WHERE key = ?1",
                params![key.trim()],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn thread_record_exists(&self, key: &str) -> GaryxDbResult<bool> {
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM thread_records WHERE key = ?1",
                params![key.trim()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Count record keys by prefix with the same exact case-sensitive
    /// prefix semantics as `list_thread_record_keys`.
    pub fn count_thread_record_keys(&self, prefix: Option<&str>) -> GaryxDbResult<usize> {
        match prefix.map(str::trim).filter(|value| !value.is_empty()) {
            Some(prefix) => {
                // LIKE is ASCII case-insensitive in SQLite; count exact
                // matches in Rust over the narrowed set (same reasoning as
                // list_thread_record_keys, review #TASK-1896).
                Ok(self.list_thread_record_keys(Some(prefix))?.len())
            }
            None => {
                let conn = self.read_conn()?;
                let count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM thread_records", [], |row| row.get(0))?;
                Ok(usize::try_from(count).unwrap_or(usize::MAX))
            }
        }
    }

    pub fn list_thread_record_keys(&self, prefix: Option<&str>) -> GaryxDbResult<Vec<String>> {
        let conn = self.read_conn()?;
        let mut keys = Vec::new();
        match prefix.map(str::trim).filter(|value| !value.is_empty()) {
            Some(prefix) => {
                // LIKE narrows the scan but is ASCII case-insensitive in
                // SQLite; the starts_with post-filter restores the exact
                // case-sensitive prefix semantics of the File/InMemory
                // stores (review #TASK-1896).
                let pattern = format!("{}%", escape_like_pattern(prefix));
                let mut stmt = conn.prepare(
                    "SELECT key FROM thread_records WHERE key LIKE ?1 ESCAPE '\\' ORDER BY key",
                )?;
                let rows = stmt.query_map(params![pattern], |row| row.get::<_, String>(0))?;
                for row in rows {
                    let key: String = row?;
                    if key.starts_with(prefix) {
                        keys.push(key);
                    }
                }
            }
            None => {
                let mut stmt = conn.prepare("SELECT key FROM thread_records ORDER BY key")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                for row in rows {
                    keys.push(row?);
                }
            }
        }
        Ok(keys)
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
        let conn = self.read_conn()?;
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
        let conn = self.read_conn()?;
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
}

impl ReadOnlyGaryxDb {
    pub(crate) fn open(path: impl AsRef<Path>) -> GaryxDbResult<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "query_only", "ON")?;
        Ok(Self { conn })
    }

    pub(crate) fn list_active_recent_thread_ids(
        &mut self,
        limit: usize,
    ) -> GaryxDbResult<ActiveRecentThreadPage> {
        list_active_recent_thread_ids(&mut self.conn, limit)
    }
}

fn list_active_recent_thread_ids(
    conn: &mut Connection,
    limit: usize,
) -> GaryxDbResult<ActiveRecentThreadPage> {
    const ACTIVE_RECENT_THREAD_PREDICATE: &str = "thread_id GLOB 'thread::*' AND (run_state = 'running' OR COALESCE(TRIM(active_run_id), '') <> '')";

    // Count and page share one WAL snapshot, matching the recent-thread page
    // contract. The predicate stays in SQL: restart wake-all is a conditional
    // thread query and must not enumerate record bodies or filter a full table
    // in application code.
    let tx = conn.transaction()?;
    let total_sql =
        format!("SELECT COUNT(*) FROM recent_threads WHERE {ACTIVE_RECENT_THREAD_PREDICATE}");
    let total: i64 = tx.query_row(&total_sql, [], |row| row.get(0))?;
    let total = usize::try_from(total).unwrap_or(usize::MAX);

    let page_sql = format!(
        "SELECT thread_id
           FROM recent_threads
          WHERE {ACTIVE_RECENT_THREAD_PREDICATE}
          ORDER BY activity_seq DESC
          LIMIT ?1"
    );
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut stmt = tx.prepare(&page_sql)?;
    let rows = stmt.query_map([limit], |row| row.get(0))?;
    let mut thread_ids = Vec::new();
    for row in rows {
        thread_ids.push(row?);
    }
    drop(stmt);
    tx.commit()?;

    Ok(ActiveRecentThreadPage { thread_ids, total })
}

fn legacy_import_generation_row_tx(tx: &Transaction<'_>) -> GaryxDbResult<Option<i64>> {
    let row = tx
        .query_row(
            "SELECT projection_version, source_row_count
               FROM projection_states
              WHERE projection_name = ?1",
            params![LEGACY_IMPORT_GENERATION_NAME],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((version, generation)) = row else {
        return Ok(None);
    };
    if version != LEGACY_IMPORT_GENERATION_VERSION || generation < 0 {
        return Err(GaryxDbError::Configuration(format!(
            "invalid legacy import generation row: version={version}, generation={generation}"
        )));
    }
    Ok(Some(generation))
}

fn legacy_import_compat_generation_tx(tx: &Transaction<'_>) -> GaryxDbResult<i64> {
    let imported = tx
        .query_row(
            "SELECT 1 FROM projection_states
              WHERE projection_name = ?1 AND projection_version = ?2",
            params![
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(if imported { 1 } else { 0 })
}

fn record_projection_state_tx(
    tx: &Transaction<'_>,
    projection_name: &str,
    projection_version: i64,
    source_row_count: i64,
    based_on_import_generation: Option<i64>,
) -> GaryxDbResult<()> {
    tx.execute(
        "INSERT INTO projection_states (
            projection_name, projection_version, source_row_count, projected_at,
            based_on_import_generation
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(projection_name) DO UPDATE SET
            projection_version = excluded.projection_version,
            source_row_count = excluded.source_row_count,
            projected_at = excluded.projected_at,
            based_on_import_generation = excluded.based_on_import_generation",
        params![
            projection_name,
            projection_version,
            source_row_count,
            now_string(),
            based_on_import_generation,
        ],
    )?;
    Ok(())
}

fn initialize_connection(conn: &Connection) -> GaryxDbResult<()> {
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
            excluded_from_recent INTEGER NOT NULL DEFAULT 0,
            sort_updated_at_us INTEGER NOT NULL DEFAULT 0,
            search_text TEXT NOT NULL DEFAULT '',
            provider_key TEXT,
            selected_model TEXT,
            selected_model_reasoning_effort TEXT,
            selected_model_service_tier TEXT,
            sdk_session_id TEXT,
            projection_version INTEGER NOT NULL DEFAULT 5,
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
            WHERE default_list_hidden = 0 AND thread_type <> 'task';
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

/// Destructive upgrade cleanup for the removed Workflow product. Old runs,
/// task-backed runs, and child threads are deleted rather than decoded or
/// adapted; no compatibility representation survives normal startup.
fn purge_retired_workflow_state(conn: &Connection) -> GaryxDbResult<()> {
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

fn sqlite_table_exists(conn: &Connection, table_name: &str) -> GaryxDbResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
        params![table_name],
        |row| row.get(0),
    )?)
}

fn insert_thread_id(thread_ids: &mut BTreeSet<String>, value: &str) {
    let value = value.trim();
    if is_thread_key(value) {
        thread_ids.insert(value.to_owned());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrozenRecentMembershipRow {
    thread_id: String,
    last_active_at: String,
}

/// S5 canonical normalization. Side-chat identity is intentionally narrow:
/// only a top-level or metadata `source == "side_chat"` hides the record.
/// Parent markers and retired exclusion flags alone never classify a side
/// chat. The retired flags are still stripped from all object-shaped records.
fn normalize_recent_membership_canonical_record(data: &mut Value) -> bool {
    let side_chat = data.get("source").and_then(Value::as_str) == Some("side_chat")
        || data
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("source"))
            .and_then(Value::as_str)
            == Some("side_chat");
    let Some(object) = data.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    if side_chat && object.get("hidden") != Some(&Value::Bool(true)) {
        object.insert("hidden".to_owned(), Value::Bool(true));
        changed = true;
    }
    for key in ["exclude_from_recent", "excludeFromRecent"] {
        changed |= object.remove(key).is_some();
    }
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        for key in ["exclude_from_recent", "excludeFromRecent"] {
            changed |= metadata.remove(key).is_some();
        }
    }
    changed
}

fn recent_membership_timestamp(value: &str) -> (i64, u32) {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|timestamp| (timestamp.timestamp(), timestamp.timestamp_subsec_nanos()))
        .unwrap_or((0, 0))
}

fn compare_recent_membership_order(
    left_timestamp: &str,
    left_thread_id: &str,
    right_timestamp: &str,
    right_thread_id: &str,
) -> CmpOrdering {
    recent_membership_timestamp(left_timestamp)
        .cmp(&recent_membership_timestamp(right_timestamp))
        .then_with(|| left_thread_id.as_bytes().cmp(right_thread_id.as_bytes()))
}

/// Cutover-only insert. Unlike the runtime upsert, this never allocates a
/// sequence: all new members intentionally share zero until step d assigns the
/// frozen H-based contiguous range with the activity indexes absent.
fn insert_recent_membership_placeholder_tx(
    tx: &Transaction<'_>,
    draft: &RecentThreadDraft,
    recorded_at: &str,
) -> GaryxDbResult<()> {
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
    let inserted = tx.execute(
        "INSERT INTO recent_threads (
            thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
            message_count, last_message_preview, recent_run_id, active_run_id, run_state,
            updated_at, last_active_at, activity_seq, recorded_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, ?14)",
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
    if inserted != 1 {
        return Err(GaryxDbError::Configuration(
            "recent membership cutover could not insert a target row".to_owned(),
        ));
    }
    Ok(())
}

/// Enforce all six directions of canonical-visible/thread-meta/recent parity
/// before the durable marker commits. These are SQL EXCEPT checks rather than
/// counts, so equal-sized but different member sets cannot pass.
fn assert_recent_membership_parity_tx(tx: &Transaction<'_>) -> GaryxDbResult<()> {
    let checks = [
        (
            "canonical visible minus thread_meta visible",
            "SELECT record.key
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND NOT EXISTS (
                    SELECT 1 FROM archived_threads AS archived
                     WHERE archived.thread_id = record.key
                )
                AND COALESCE(json_type(record.body, '$.hidden'), '') <> 'true'
             EXCEPT
             SELECT thread_id FROM thread_meta WHERE default_list_hidden = 0
             LIMIT 1",
        ),
        (
            "thread_meta visible minus canonical visible",
            "SELECT thread_id FROM thread_meta WHERE default_list_hidden = 0
             EXCEPT
             SELECT record.key
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND NOT EXISTS (
                    SELECT 1 FROM archived_threads AS archived
                     WHERE archived.thread_id = record.key
                )
                AND COALESCE(json_type(record.body, '$.hidden'), '') <> 'true'
             LIMIT 1",
        ),
        (
            "canonical visible minus recent",
            "SELECT record.key
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND NOT EXISTS (
                    SELECT 1 FROM archived_threads AS archived
                     WHERE archived.thread_id = record.key
                )
                AND COALESCE(json_type(record.body, '$.hidden'), '') <> 'true'
             EXCEPT
             SELECT thread_id FROM recent_threads
             LIMIT 1",
        ),
        (
            "recent minus canonical visible",
            "SELECT thread_id FROM recent_threads
             EXCEPT
             SELECT record.key
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND NOT EXISTS (
                    SELECT 1 FROM archived_threads AS archived
                     WHERE archived.thread_id = record.key
                )
                AND COALESCE(json_type(record.body, '$.hidden'), '') <> 'true'
             LIMIT 1",
        ),
        (
            "thread_meta visible minus recent",
            "SELECT thread_id FROM thread_meta WHERE default_list_hidden = 0
             EXCEPT
             SELECT thread_id FROM recent_threads
             LIMIT 1",
        ),
        (
            "recent minus thread_meta visible",
            "SELECT thread_id FROM recent_threads
             EXCEPT
             SELECT thread_id FROM thread_meta WHERE default_list_hidden = 0
             LIMIT 1",
        ),
    ];
    for (label, sql) in checks {
        if let Some(thread_id) = tx
            .query_row(sql, [], |row| row.get::<_, String>(0))
            .optional()?
        {
            return Err(GaryxDbError::Configuration(format!(
                "recent membership parity failed ({label}): {thread_id}"
            )));
        }
    }
    Ok(())
}

fn is_retired_workflow_executor_json(raw: &str) -> bool {
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

fn object_marks_retired_workflow(object: &serde_json::Map<String, Value>) -> bool {
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

fn read_thread_pins_tx(conn: &Connection) -> GaryxDbResult<Vec<PinnedThreadRecord>> {
    let mut stmt = conn.prepare(
        "SELECT thread_id, pinned_at, sort_order
           FROM thread_pins
          ORDER BY sort_order ASC, pinned_at DESC, thread_id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(PinnedThreadRecord {
            thread_id: row.get(0)?,
            pinned_at: row.get(1)?,
            sort_order: row.get(2)?,
        })
    })?;
    let mut pins = Vec::new();
    for row in rows {
        pins.push(row?);
    }
    Ok(pins)
}

fn read_thread_pins_revision_tx(conn: &Connection) -> GaryxDbResult<i64> {
    Ok(conn.query_row(
        "SELECT pins_revision FROM thread_pins_meta WHERE id = 1",
        [],
        |row| row.get(0),
    )?)
}

fn read_thread_pins_page_tx(conn: &Connection) -> GaryxDbResult<ThreadPinsPage> {
    Ok(ThreadPinsPage {
        pins: read_thread_pins_tx(conn)?,
        revision: read_thread_pins_revision_tx(conn)?,
    })
}

/// Shared revision boundary for every runtime mutation of `thread_pins`.
/// Callers pass the mutation's affected-row result while still inside the
/// same transaction; no-op idempotent operations deliberately do not bump.
fn bump_thread_pins_revision_if_changed_tx(conn: &Connection, changed: bool) -> GaryxDbResult<()> {
    if !changed {
        return Ok(());
    }
    let updated = conn.execute(
        "UPDATE thread_pins_meta
            SET pins_revision = pins_revision + 1
          WHERE id = 1",
        [],
    )?;
    if updated != 1 {
        return Err(GaryxDbError::Configuration(
            "thread_pins_meta singleton is missing".to_owned(),
        ));
    }
    Ok(())
}

fn read_thread_favorites_tx(conn: &Connection) -> GaryxDbResult<Vec<FavoriteThreadRecord>> {
    let mut stmt = conn.prepare(
        "SELECT thread_id, favorited_at
           FROM thread_favorites
          ORDER BY favorited_at DESC, thread_id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(FavoriteThreadRecord {
            thread_id: row.get(0)?,
            favorited_at: row.get(1)?,
        })
    })?;
    let mut favorites = Vec::new();
    for row in rows {
        favorites.push(row?);
    }
    Ok(favorites)
}

fn read_thread_favorites_revision_tx(conn: &Connection) -> GaryxDbResult<i64> {
    Ok(conn.query_row(
        "SELECT favorites_revision FROM thread_favorites_meta WHERE id = 1",
        [],
        |row| row.get(0),
    )?)
}

fn read_thread_favorites_page_with_rows_tx(
    conn: &Connection,
    favorites: Vec<FavoriteThreadRecord>,
) -> GaryxDbResult<ThreadFavoritesPage> {
    Ok(ThreadFavoritesPage {
        favorites,
        revision: read_thread_favorites_revision_tx(conn)?,
        store_incarnation_id: read_store_incarnation_id(conn)?,
    })
}

fn read_thread_favorites_page_tx(conn: &Connection) -> GaryxDbResult<ThreadFavoritesPage> {
    read_thread_favorites_page_with_rows_tx(conn, read_thread_favorites_tx(conn)?)
}

fn bump_thread_favorites_revision_tx(conn: &Connection) -> GaryxDbResult<()> {
    let updated = conn.execute(
        "UPDATE thread_favorites_meta
            SET favorites_revision = favorites_revision + 1
          WHERE id = 1",
        [],
    )?;
    if updated != 1 {
        return Err(GaryxDbError::Configuration(
            "thread_favorites_meta singleton is missing".to_owned(),
        ));
    }
    Ok(())
}

fn bump_thread_favorites_revision_if_changed_tx(
    conn: &Connection,
    changed: bool,
) -> GaryxDbResult<()> {
    if changed {
        bump_thread_favorites_revision_tx(conn)?;
    }
    Ok(())
}

fn thread_record_exists_tx(conn: &Connection, thread_id: &str) -> GaryxDbResult<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM thread_records WHERE key = ?1",
            params![thread_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
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

fn normalize_thread_pin_order(ordered_ids: Vec<String>) -> GaryxDbResult<Vec<String>> {
    if ordered_ids.is_empty() {
        return Err(GaryxDbError::BadRequest(
            "thread_ids must be a non-empty array".to_owned(),
        ));
    }
    let mut normalized = Vec::with_capacity(ordered_ids.len());
    let mut seen = BTreeSet::new();
    for thread_id in ordered_ids {
        let thread_id = normalize_thread_id(&thread_id)?;
        if !seen.insert(thread_id.clone()) {
            return Err(GaryxDbError::BadRequest(format!(
                "duplicate thread_id: {thread_id}"
            )));
        }
        normalized.push(thread_id);
    }
    Ok(normalized)
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
        excluded_from_recent: row.get::<_, i64>(18)? != 0,
        sort_updated_at_us: row.get(19)?,
        search_text: row.get(20)?,
        provider_key: row.get(21)?,
        selected_model: row.get(22)?,
        selected_model_reasoning_effort: row.get(23)?,
        selected_model_service_tier: row.get(24)?,
        sdk_session_id: row.get(25)?,
        projection_version: row.get(26)?,
        projected_at: row.get(27)?,
    })
}

fn thread_summary_row_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadSummaryRow> {
    let worktree_json: Option<String> = row.get(14)?;
    Ok(ThreadSummaryRow {
        thread_id: row.get(0)?,
        title: row.get(1)?,
        workspace_dir: row.get(2)?,
        thread_type: row.get(3)?,
        provider_type: row.get(4)?,
        agent_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        message_count: row.get::<_, i64>(8)?.clamp(0, i64::from(u32::MAX)) as u32,
        last_user_message: row.get(9)?,
        last_assistant_message: row.get(10)?,
        last_message_preview: row.get(11)?,
        recent_run_id: row.get(12)?,
        active_run_id: row.get(13)?,
        worktree: worktree_json.and_then(|value| serde_json::from_str(&value).ok()),
        excluded_from_recent: row.get::<_, i64>(15)? != 0,
        sort_updated_at_us: row.get(16)?,
    })
}

/// Projection writes derived from one thread record, applied inside the
/// same transaction as the record upsert (#TASK-1864 batch 2, D2). Each
/// `Some` upserts that projection; `None` removes it.
pub struct ThreadRecordProjections {
    pub thread_meta: Option<ThreadMetaProjectionDraft>,
    pub task: Option<TaskProjectionDraft>,
    pub recent: Option<RecentThreadDraft>,
}

/// One record write inside an atomic multi-record batch.
pub struct ThreadRecordWrite {
    pub key: String,
    pub body: String,
    pub updated_at: Option<String>,
    pub projections: Option<ThreadRecordProjections>,
}

fn write_thread_record_with_projections_tx(
    tx: &Transaction<'_>,
    key: &str,
    body: &str,
    updated_at: Option<&str>,
    projections: Option<ThreadRecordProjections>,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let key = normalize_required("key", key)?;
    // Archived threads reject writes inside the same transaction that
    // would persist them — a tombstone committed by a racing archive
    // can never be overtaken by a write that passed an earlier check.
    if garyx_router::is_thread_key(&key) {
        let archived: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM archived_threads WHERE thread_id = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        if archived.is_some() {
            return Err(GaryxDbError::ThreadArchived(key));
        }
    }
    tx.execute(
        "INSERT INTO thread_records (key, body, updated_at, recorded_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET
            body = excluded.body,
            updated_at = excluded.updated_at,
            recorded_at = excluded.recorded_at",
        params![key, body, updated_at, recorded_at],
    )?;
    if let Some(projections) = projections {
        match projections.thread_meta {
            Some(draft) => replace_thread_meta_projection_tx(tx, draft, recorded_at)?,
            None => {
                remove_thread_meta_projection_tx(tx, &key)?;
            }
        }
        match projections.task {
            Some(mut draft) => {
                draft.thread_id = normalize_thread_id(&draft.thread_id)?;
                task_forest::upsert_task_projection(tx, &draft, recorded_at)?;
            }
            None => {
                remove_task_projection_tx(tx, &key)?;
            }
        }
        match projections.recent {
            Some(draft) => {
                upsert_recent_thread_tx(tx, draft, recorded_at)?;
            }
            None => {
                remove_recent_thread_tx(tx, &key)?;
            }
        }
    }
    Ok(())
}

/// Escape `%`/`_`/`\` so a caller-supplied prefix matches literally in a
/// LIKE pattern (used with `ESCAPE '\'`).
fn escape_like_pattern(prefix: &str) -> String {
    prefix
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn upsert_recent_thread_tx(
    tx: &Transaction<'_>,
    draft: RecentThreadDraft,
    recorded_at: &str,
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
    let recorded_at = recorded_at.to_owned();
    let activity_seq = allocate_recent_thread_activity_seq_tx(tx)?;

    tx.execute(
        "INSERT INTO recent_threads (
            thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
            message_count, last_message_preview, recent_run_id, active_run_id, run_state,
            updated_at, last_active_at, activity_seq, recorded_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
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
            activity_seq = excluded.activity_seq,
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
            activity_seq,
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
        activity_seq,
        recorded_at,
    })
}

fn allocate_recent_thread_activity_seq_tx(tx: &Transaction<'_>) -> GaryxDbResult<i64> {
    let current: i64 = tx.query_row(
        "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let next = current
        .checked_add(1)
        .filter(|value| *value < MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE)
        .ok_or_else(|| {
            GaryxDbError::Configuration(
                "recent thread activity sequence space is exhausted".to_owned(),
            )
        })?;
    let updated = tx.execute(
        "UPDATE recent_threads_meta SET activity_seq = ?1 WHERE id = 1",
        params![next],
    )?;
    if updated != 1 {
        return Err(GaryxDbError::Configuration(
            "recent_threads_meta singleton is missing".to_owned(),
        ));
    }
    Ok(next)
}

fn remove_recent_thread_tx(conn: &Connection, thread_id: &str) -> GaryxDbResult<bool> {
    let removed = conn.execute(
        "DELETE FROM recent_threads WHERE thread_id = ?1",
        params![thread_id],
    )?;
    Ok(removed > 0)
}

fn remove_task_projection_tx(conn: &Connection, thread_id: &str) -> GaryxDbResult<bool> {
    let removed = conn.execute(
        "DELETE FROM task_projection WHERE thread_id = ?1",
        params![thread_id],
    )?;
    Ok(removed > 0)
}

fn replace_thread_meta_projection_tx(
    tx: &Transaction<'_>,
    draft: ThreadMetaProjectionDraft,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let thread_id = normalize_thread_id(&draft.thread_id)?;
    remove_thread_meta_projection_tx(tx, &thread_id)?;
    let mut thread_meta = draft.thread_meta;
    thread_meta.thread_id = thread_id.clone();
    upsert_thread_meta(tx, &thread_meta, recorded_at)?;
    for mut endpoint in draft.channel_endpoints {
        endpoint.thread_id = Some(thread_id.clone());
        upsert_thread_channel_endpoint(tx, &endpoint, recorded_at)?;
    }
    Ok(())
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
    let excluded_from_recent = if meta.excluded_from_recent { 1 } else { 0 };
    let sort_updated_at_us = meta.sort_updated_at_us;
    let search_text = meta.search_text.clone();
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
            excluded_from_recent, sort_updated_at_us, search_text,
            provider_key, selected_model, selected_model_reasoning_effort,
            selected_model_service_tier, sdk_session_id,
            projection_version, projected_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)
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
            excluded_from_recent = excluded.excluded_from_recent,
            sort_updated_at_us = excluded.sort_updated_at_us,
            search_text = excluded.search_text,
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
            excluded_from_recent,
            sort_updated_at_us,
            search_text,
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

fn normalize_automation_thread_run_mode(value: &str) -> GaryxDbResult<String> {
    let mode = normalize_required("mode", value)?;
    match mode.as_str() {
        "generated_thread" | "target_thread" => Ok(mode),
        _ => Err(GaryxDbError::BadRequest(
            "mode must be generated_thread or target_thread".to_owned(),
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

fn ensure_thread_pins_sort_order_column(conn: &Connection) -> GaryxDbResult<()> {
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

fn ensure_recent_threads_activity_seq_column(conn: &Connection) -> GaryxDbResult<()> {
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

fn ensure_recent_threads_meta_row(conn: &Connection) -> GaryxDbResult<()> {
    conn.execute(
        "INSERT INTO recent_threads_meta (id, activity_seq)
         VALUES (1, 0)
         ON CONFLICT(id) DO NOTHING",
        [],
    )?;
    Ok(())
}

fn ensure_thread_pins_meta_row(conn: &Connection) -> GaryxDbResult<()> {
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

fn ensure_thread_favorites_meta_row(conn: &Connection) -> GaryxDbResult<()> {
    conn.execute(
        "INSERT INTO thread_favorites_meta (id, favorites_revision)
         VALUES (1, 0)
         ON CONFLICT(id) DO NOTHING",
        [],
    )?;
    Ok(())
}

fn ensure_store_incarnation_row(conn: &Connection) -> GaryxDbResult<()> {
    conn.execute(
        "INSERT INTO garyx_store_meta (id, store_incarnation_id)
         VALUES (1, ?1)
         ON CONFLICT(id) DO NOTHING",
        params![Uuid::new_v4().to_string()],
    )?;
    // Treat corruption as a startup failure rather than silently rotating the
    // CAS domain during an ordinary reopen.
    read_store_incarnation_id(conn).map(|_| ())
}

fn read_store_incarnation_id(conn: &Connection) -> GaryxDbResult<String> {
    let raw: String = conn
        .query_row(
            "SELECT store_incarnation_id FROM garyx_store_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            GaryxDbError::Configuration("garyx_store_meta singleton is missing".to_owned())
        })?;
    Uuid::parse_str(&raw)
        .map(|uuid| uuid.to_string())
        .map_err(|_| {
            GaryxDbError::Configuration("store_incarnation_id is not a valid UUID".to_owned())
        })
}

fn rotate_store_incarnation_tx(conn: &Connection) -> GaryxDbResult<String> {
    let next = Uuid::new_v4().to_string();
    let updated = conn.execute(
        "UPDATE garyx_store_meta SET store_incarnation_id = ?1 WHERE id = 1",
        params![next],
    )?;
    if updated != 1 {
        return Err(GaryxDbError::Configuration(
            "garyx_store_meta singleton is missing".to_owned(),
        ));
    }
    Ok(next)
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

fn ensure_capsules_favorited_at_column(conn: &Connection) -> GaryxDbResult<()> {
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

fn ensure_projection_state_import_generation_column(conn: &Connection) -> GaryxDbResult<()> {
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
    if !columns.contains("excluded_from_recent") {
        conn.execute(
            "ALTER TABLE thread_meta
             ADD COLUMN excluded_from_recent INTEGER NOT NULL DEFAULT 0",
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

/// Restore the single-owner endpoint schema after versions that stored one
/// row per `(endpoint_key, thread_id)`. `CREATE TABLE IF NOT EXISTS` cannot
/// change that composite primary key, so current `ON CONFLICT(endpoint_key)`
/// writes otherwise fail at prepare time. The endpoint table is derived state:
/// rebuild it atomically and clear the holder-dedup marker so the existing
/// post-import startup migration repopulates it from canonical thread records.
fn ensure_thread_channel_endpoint_single_holder_schema(conn: &Connection) -> GaryxDbResult<()> {
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

fn recent_thread_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecentThreadRecord> {
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
        activity_seq: row.get(13)?,
        recorded_at: row.get(14)?,
    })
}

fn known_channel_endpoint_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<KnownChannelEndpoint> {
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
        favorited_at: row.get(12)?,
    })
}

fn capsule_by_id(conn: &Connection, id: &str) -> GaryxDbResult<Option<CapsuleRecord>> {
    Ok(conn
        .query_row(
            "SELECT id, title, description, thread_id, run_id, agent_id, provider_type,
                    html_sha256, byte_size, revision, created_at, updated_at, favorited_at
             FROM capsules
             WHERE id = ?1",
            params![id],
            capsule_from_row,
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use serde_json::json;

    /// A read query slow enough (tens of ms) to make lock serialization
    /// visible to the wall clock.
    fn run_slow_read(conn: &Connection) -> u128 {
        let started = std::time::Instant::now();
        let _: i64 = conn
            .query_row(
                "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 3000000) SELECT count(*) FROM cnt",
                [],
                |row| row.get(0),
            )
            .expect("slow read");
        started.elapsed().as_millis()
    }

    #[test]
    fn concurrent_reads_run_in_parallel_across_the_pool() {
        let dir = tempfile::tempdir().expect("temp dir");
        let service = std::sync::Arc::new(
            GaryxDbService::open(dir.path().join("garyx-db.sqlite3")).expect("db opens"),
        );

        // Calibrate a single read on this machine.
        let single_ms = {
            let conn = service.read_conn().expect("read conn");
            run_slow_read(&conn).max(1)
        };

        let readers = 4u128;
        let started = std::time::Instant::now();
        let handles: Vec<_> = (0..readers)
            .map(|_| {
                let service = std::sync::Arc::clone(&service);
                std::thread::spawn(move || {
                    let conn = service.read_conn().expect("read conn");
                    run_slow_read(&conn);
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("reader thread");
        }
        let wall_ms = started.elapsed().as_millis().max(1);

        // One shared read connection serializes the four reads
        // (wall ≈ 4× single); a pool must let them overlap. The 3× bound
        // leaves headroom for scheduling noise while still failing hard on
        // full serialization.
        assert!(
            wall_ms < single_ms * readers * 3 / 4,
            "concurrent reads serialized behind one connection: wall={wall_ms}ms single={single_ms}ms readers={readers}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn blocking_entry_keeps_the_runtime_responsive() {
        // One runtime worker: if database work runs ON the worker (the old
        // direct-call shape), the heartbeat below cannot tick until the DB
        // call finishes. Through `run_blocking` the worker stays free.
        let dir = tempfile::tempdir().expect("temp dir");
        let service = std::sync::Arc::new(
            GaryxDbService::open(dir.path().join("garyx-db.sqlite3")).expect("db opens"),
        );

        let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let heartbeat = {
            let ticks = std::sync::Arc::clone(&ticks);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        };

        service
            .run_blocking(|db| {
                let conn = db.read_conn()?;
                run_slow_read(&conn);
                Ok(())
            })
            .await
            .expect("blocking read");

        heartbeat.abort();
        let observed = ticks.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            observed >= 3,
            "runtime worker was starved during database work: {observed} heartbeat ticks"
        );
    }

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
    fn file_store_incarnation_is_uuid_stable_on_reopen_and_rotates_only_explicitly() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let service = GaryxDbService::open(&path).expect("first open");
        let first = service.store_incarnation_id().expect("first identity");
        assert_eq!(Uuid::parse_str(&first).unwrap().to_string(), first);
        drop(service);

        let reopened = GaryxDbService::open(&path).expect("ordinary reopen");
        assert_eq!(reopened.store_incarnation_id().unwrap(), first);
        let rotated = reopened
            .rotate_store_incarnation()
            .expect("explicit offline rotation")
            .store_incarnation_id;
        assert_ne!(rotated, first);
        drop(reopened);

        let after_rotation = GaryxDbService::open(&path).expect("reopen after rotation");
        assert_eq!(after_rotation.store_incarnation_id().unwrap(), rotated);
    }

    #[test]
    fn data_dir_lock_precedes_schema_initialization_is_cloexec_and_times_out_boundedly() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        Connection::open(&path)
            .expect("seed raw database")
            .execute("CREATE TABLE untouched(value INTEGER)", [])
            .expect("seed sentinel schema");

        let owner = DataDirLock::acquire(&path, Duration::ZERO).expect("own data dir");
        assert!(owner.close_on_exec().expect("CLOEXEC query"));
        let started = Instant::now();
        let error = GaryxDbService::open_with_lock_wait(&path, Duration::from_millis(80))
            .err()
            .expect("second gateway must time out");
        assert!(matches!(error, GaryxDbError::DataDirLocked { .. }));
        assert!(
            started.elapsed() >= Duration::from_millis(70),
            "lock wait returned before its bounded deadline"
        );

        let raw = Connection::open(&path).expect("inspect untouched database");
        assert!(!sqlite_table_exists(&raw, "garyx_store_meta").unwrap());
        assert!(sqlite_table_exists(&raw, "untouched").unwrap());
        drop(raw);
        drop(owner);

        let service = GaryxDbService::open_with_lock_wait(&path, Duration::ZERO)
            .expect("lock release permits startup");
        assert!(service.store_incarnation_id().is_ok());
    }

    #[test]
    fn data_dir_lock_waiter_continues_after_old_gateway_releases_for_restart_fallback() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let old_gateway = GaryxDbService::open(&path).expect("old gateway owns lock");
        let waiter_path = path.clone();
        let waiter = std::thread::spawn(move || {
            GaryxDbService::open_with_lock_wait(waiter_path, Duration::from_secs(2))
        });

        std::thread::sleep(Duration::from_millis(100));
        assert!(!waiter.is_finished(), "new gateway skipped the held lock");
        drop(old_gateway);
        let new_gateway = waiter
            .join()
            .expect("waiter thread")
            .expect("new gateway takes released lock");
        assert!(new_gateway.store_incarnation_id().is_ok());
    }

    #[test]
    fn pre_r5_parent_handoff_has_continue_and_fail_closed_branches() {
        let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let release = alive.clone();
        let exiting_parent = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            release.store(false, std::sync::atomic::Ordering::Release);
        });
        wait_for_parent_exit(4242, Duration::from_secs(1), || {
            alive.load(std::sync::atomic::Ordering::Acquire)
        })
        .expect("startup continues after parent exits");
        exiting_parent.join().unwrap();

        let error = wait_for_parent_exit(4243, Duration::from_millis(70), || true)
            .expect_err("live parent at cap must fail closed");
        assert!(matches!(
            error,
            GaryxDbError::ParentHandoffTimedOut {
                parent_pid: 4243,
                ..
            }
        ));

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let raw = Connection::open(&path).expect("raw pre-R5 database");
        raw.execute("CREATE TABLE untouched(value INTEGER)", [])
            .unwrap();
        drop(raw);
        let lock = DataDirLock::acquire(&path, Duration::ZERO).expect("new binary lock");
        let barrier = wait_for_parent_exit(4244, Duration::from_millis(60), || true);
        assert!(barrier.is_err());
        drop(lock);
        let raw = Connection::open(&path).expect("inspect after failed handoff");
        assert!(sqlite_table_exists(&raw, "untouched").unwrap());
        assert!(
            !sqlite_table_exists(&raw, "garyx_store_meta").unwrap(),
            "fail-closed parent timeout must precede destructive/schema initialization"
        );
        drop(raw);
        DataDirLock::acquire(&path, Duration::ZERO)
            .expect("failed child released the data-dir lock");
    }

    #[test]
    fn open_path_wires_parent_handoff_between_data_lock_and_database_open() {
        let source = include_str!("mod.rs");
        let open_path = source
            .split_once("fn open_with_lock_wait(")
            .expect("open_with_lock_wait definition")
            .1
            .split_once("pub fn memory()")
            .expect("memory constructor follows open path")
            .0;
        let lock = open_path
            .find("DataDirLock::acquire(path, lock_wait)?")
            .expect("open path acquires data lock");
        let handoff = open_path
            .find("wait_for_pre_r5_parent_handoff()?")
            .expect("open path invokes pre-R5 handoff barrier");
        let connection = open_path
            .find("Connection::open(path)?")
            .expect("open path opens SQLite connection");
        assert!(
            lock < handoff && handoff < connection,
            "startup ordering must remain lock -> parent handoff -> SQLite open"
        );
    }

    #[cfg(unix)]
    #[test]
    fn parent_executable_resolution_failure_is_fail_closed() {
        let error = parent_has_same_executable_name_with(4242, |_| {
            Err(GaryxDbError::Configuration(
                "synthetic ps failure".to_owned(),
            ))
        })
        .expect_err("an unknown parent executable must abort startup");
        assert!(matches!(error, GaryxDbError::Configuration(_)));
    }

    #[test]
    fn read_only_handle_queries_during_a_writer_lock_and_rejects_writes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let service = GaryxDbService::open(&path).expect("create database");
        service
            .upsert_recent_thread(RecentThreadDraft {
                thread_id: "thread::read-only-snapshot".to_owned(),
                title: "Read only snapshot".to_owned(),
                workspace_dir: None,
                thread_type: "chat".to_owned(),
                provider_type: None,
                agent_id: None,
                message_count: 0,
                last_message_preview: String::new(),
                recent_run_id: None,
                active_run_id: Some("run::active".to_owned()),
                run_state: "running".to_owned(),
                updated_at: None,
                last_active_at: "2026-07-14T00:00:00Z".to_owned(),
            })
            .expect("seed recent projection");

        let writer = Connection::open(&path).expect("writer connection");
        writer
            .execute_batch("BEGIN IMMEDIATE;")
            .expect("hold the database write lock");

        let mut read_only = ReadOnlyGaryxDb::open(&path).expect("open read-only handle");
        let query_only: i64 = read_only
            .conn
            .query_row("PRAGMA query_only", [], |row| row.get(0))
            .expect("query_only pragma");
        assert_eq!(query_only, 1);
        let page = read_only
            .list_active_recent_thread_ids(16)
            .expect("WAL reader remains available during a write transaction");
        assert_eq!(page.thread_ids, vec!["thread::read-only-snapshot"]);

        writer.execute_batch("COMMIT;").expect("release write lock");
        let error = read_only
            .conn
            .execute("DELETE FROM recent_threads", [])
            .expect_err("read-only handle must reject writes");
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::ReadOnly),
            "unexpected write error: {error}"
        );
    }

    fn sample_recent_draft(thread_id: &str) -> RecentThreadDraft {
        RecentThreadDraft {
            thread_id: thread_id.to_owned(),
            title: "Sample".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 1,
            last_message_preview: "hello".to_owned(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: None,
            last_active_at: "2026-07-08T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn recent_activity_schema_initializes_before_writes_and_reopens_stably() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        {
            let conn = Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                "CREATE TABLE recent_threads (
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
                 INSERT INTO recent_threads (
                     thread_id, last_active_at, recorded_at
                 ) VALUES (
                     'thread::legacy-before-seq',
                     '2026-07-01T00:00:00Z',
                     '2026-07-01T00:00:00Z'
                 );",
            )
            .expect("seed legacy recent table");
        }

        let db = GaryxDbService::open(&path).expect("open upgraded db");
        let conn = db.conn().expect("writer");
        let meta: i64 = conn
            .query_row(
                "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("meta initialized during schema open");
        let legacy_seq: i64 = conn
            .query_row(
                "SELECT activity_seq FROM recent_threads
                  WHERE thread_id = 'thread::legacy-before-seq'",
                [],
                |row| row.get(0),
            )
            .expect("legacy column added during schema open");
        assert_eq!((meta, legacy_seq), (0, 0));
        drop(conn);
        drop(db);

        let reopened = GaryxDbService::open(&path).expect("reopen upgraded db");
        let conn = reopened.conn().expect("writer after reopen");
        assert_eq!(
            conn.query_row(
                "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0,
            "ordinary reopen must not move the activity high-water mark"
        );
    }

    #[test]
    fn recent_activity_backfill_preserves_old_order_and_is_truly_one_shot() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        {
            let conn = Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                "CREATE TABLE recent_threads (
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
                 INSERT INTO recent_threads (
                     thread_id, last_active_at, recorded_at
                 ) VALUES
                     ('thread::z-old', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z'),
                     ('thread::b-tie', '2026-07-02T00:00:00Z', '2026-07-02T00:00:00Z'),
                     ('thread::a-tie', '2026-07-02T00:00:00Z', '2026-07-02T00:00:00Z');",
            )
            .expect("seed legacy order");
        }

        let db = GaryxDbService::open(&path).expect("open upgraded db");
        db.conn()
            .unwrap()
            .execute(
                "UPDATE recent_threads_meta SET activity_seq = 50 WHERE id = 1",
                [],
            )
            .unwrap();
        let first = db
            .migrate_recent_thread_activity_seq_v1()
            .expect("backfill activity sequence");
        assert_eq!(first.source_row_count, 3);
        assert_eq!(first.updated_row_count, 3);
        assert!(!first.already_completed);

        let rows = db
            .list_recent_threads(10, 0)
            .expect("list migrated recent rows");
        assert_eq!(
            rows.iter()
                .map(|row| (row.thread_id.as_str(), row.activity_seq))
                .collect::<Vec<_>>(),
            vec![
                ("thread::a-tie", 53),
                ("thread::b-tie", 52),
                ("thread::z-old", 51),
            ],
            "descending seq must exactly preserve the former timestamp/id order"
        );
        let conn = db.conn().expect("writer");
        assert_eq!(
            conn.query_row(
                "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            53,
            "backfill must floor against and then advance the existing meta"
        );
        assert!(
            conn.execute(
                "INSERT INTO recent_threads (
                     thread_id, last_active_at, activity_seq, recorded_at
                 ) VALUES (
                     'thread::duplicate-seq', '2026-07-03T00:00:00Z', 53,
                     '2026-07-03T00:00:00Z'
                 )",
                [],
            )
            .is_err(),
            "the post-backfill activity sequence index must be unique"
        );
        drop(conn);

        let second = db
            .migrate_recent_thread_activity_seq_v1()
            .expect("one-shot rerun");
        assert!(second.already_completed);
        assert_eq!(second.source_row_count, 3);
        assert_eq!(second.updated_row_count, 0);
        assert_eq!(
            db.list_recent_threads(10, 0)
                .unwrap()
                .iter()
                .map(|row| row.activity_seq)
                .collect::<Vec<_>>(),
            vec![53, 52, 51]
        );
        drop(db);

        let reopened = GaryxDbService::open(&path).expect("reopen migrated db");
        assert!(
            reopened
                .migrate_recent_thread_activity_seq_v1()
                .unwrap()
                .already_completed
        );
        assert_eq!(
            reopened.list_recent_threads(10, 0).unwrap()[0].activity_seq,
            53
        );
    }

    #[test]
    fn recent_activity_allocator_is_transactional_strict_and_safe_integer_bounded() {
        let db = std::sync::Arc::new(GaryxDbService::memory().expect("memory db"));

        {
            let mut conn = db.conn().expect("writer");
            let tx = conn.transaction().expect("transaction");
            let record = upsert_recent_thread_tx(
                &tx,
                sample_recent_draft("thread::rolled-back-seq"),
                "2026-07-16T00:00:00Z",
            )
            .expect("upsert inside uncommitted transaction");
            assert_eq!(record.activity_seq, 1);
            drop(tx);
        }
        assert!(db.list_recent_threads(10, 0).unwrap().is_empty());
        assert_eq!(
            db.conn()
                .unwrap()
                .query_row(
                    "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
            "allocator and projection upsert must roll back together"
        );

        let handles = (0..24)
            .map(|index| {
                let db = std::sync::Arc::clone(&db);
                std::thread::spawn(move || {
                    db.upsert_recent_thread(sample_recent_draft(&format!(
                        "thread::concurrent-seq-{index:02}"
                    )))
                    .expect("concurrent upsert")
                    .activity_seq
                })
            })
            .collect::<Vec<_>>();
        let mut allocated = handles
            .into_iter()
            .map(|handle| handle.join().expect("allocator thread"))
            .collect::<Vec<_>>();
        allocated.sort_unstable();
        assert_eq!(allocated, (1..=24).collect::<Vec<_>>());

        let first = db
            .upsert_recent_thread(sample_recent_draft("thread::moves-to-head"))
            .unwrap();
        let second = db
            .upsert_recent_thread(sample_recent_draft("thread::other-head"))
            .unwrap();
        let moved = db
            .upsert_recent_thread(sample_recent_draft("thread::moves-to-head"))
            .unwrap();
        assert!(first.activity_seq < second.activity_seq);
        assert!(second.activity_seq < moved.activity_seq);
        assert_eq!(
            db.list_recent_threads(2, 0).unwrap()[0].thread_id,
            "thread::moves-to-head",
            "every read-modify-write upsert gets a fresh monotonic ordering key"
        );

        let conn = db.conn().expect("writer");
        assert!(
            conn.execute(
                "UPDATE recent_threads_meta SET activity_seq = 9007199254740991 WHERE id = 1",
                [],
            )
            .is_err(),
            "meta must reject values that are not exactly representable as desktop integers"
        );
        assert!(
            conn.execute(
                "UPDATE recent_threads SET activity_seq = 9007199254740991
                  WHERE thread_id = 'thread::moves-to-head'",
                [],
            )
            .is_err(),
            "rows must enforce the same exclusive safe-integer bound"
        );
    }

    #[test]
    fn recovery_generation_never_resets_activity_meta_or_one_shot_marker() {
        let db = GaryxDbService::memory().expect("memory db");
        db.migrate_recent_thread_activity_seq_v1()
            .expect("mark empty backfill complete");
        let older = db
            .upsert_recent_thread(sample_recent_draft("thread::before-recovery-older"))
            .unwrap();
        let old_head = db
            .upsert_recent_thread(sample_recent_draft("thread::before-recovery-head"))
            .unwrap();
        assert_eq!((older.activity_seq, old_head.activity_seq), (1, 2));

        assert_eq!(db.commit_legacy_import(0, false).unwrap(), 1);
        db.record_legacy_archive_retirement().unwrap();
        db.clear_projection_state(crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME)
            .unwrap();
        assert_eq!(db.commit_legacy_import(2, true).unwrap(), 2);
        assert!(
            db.projection_state_exists(
                RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME,
                RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION,
            )
            .unwrap(),
            "recovery generation changes must not clear the independent seq marker"
        );

        let recovered = db
            .upsert_recent_thread(sample_recent_draft("thread::recovery-import"))
            .unwrap();
        assert_eq!(recovered.activity_seq, 3);
        assert!(
            db.migrate_recent_thread_activity_seq_v1()
                .unwrap()
                .already_completed
        );
        let old_cursor_page = db
            .list_recent_threads_keyset_page(
                RecentThreadTaskFilter::Include,
                10,
                Some(old_head.activity_seq),
            )
            .expect("old cursor remains valid");
        assert_eq!(
            old_cursor_page
                .records
                .iter()
                .map(|row| (row.thread_id.as_str(), row.activity_seq))
                .collect::<Vec<_>>(),
            vec![("thread::before-recovery-older", 1)]
        );
    }

    #[test]
    fn recent_membership_cutover_rebuilds_exact_membership_and_preserves_retained_order() {
        let db = GaryxDbService::memory().expect("memory db");
        prepare_recent_membership_prerequisites(&db);

        for (thread_id, body) in [
            (
                "thread::retained-z",
                json!({"thread_id": "thread::retained-z", "label": "Retained Z"}),
            ),
            (
                "thread::retained-a",
                json!({"thread_id": "thread::retained-a", "label": "Retained A"}),
            ),
            (
                "thread::new-missing",
                json!({"thread_id": "thread::new-missing", "label": "No time"}),
            ),
            (
                "thread::new-between",
                json!({
                    "thread_id": "thread::new-between",
                    "label": "Between",
                    "updated_at": "2026-07-02T00:00:00Z",
                    "automation_thread_mode": "generated_thread"
                }),
            ),
            (
                "thread::new-same",
                json!({
                    "thread_id": "thread::new-same",
                    "label": "Same timestamp",
                    "updated_at": "2026-07-03T00:00:00Z",
                    "exclude_from_recent": true,
                    "excludeFromRecent": true,
                    "metadata": {
                        "exclude_from_recent": true,
                        "excludeFromRecent": true
                    }
                }),
            ),
            (
                "thread::new-latest",
                json!({
                    "thread_id": "thread::new-latest",
                    "label": "Latest",
                    "updated_at": "2026-07-04T00:00:00Z"
                }),
            ),
            (
                "thread::hidden",
                json!({"thread_id": "thread::hidden", "hidden": true}),
            ),
            (
                "thread::side-chat",
                json!({
                    "thread_id": "thread::side-chat",
                    "source": "side_chat",
                    "side_chat_parent_thread_id": "thread::retained-z",
                    "exclude_from_recent": true,
                    "metadata": {"excludeFromRecent": true}
                }),
            ),
        ] {
            seed_recent_membership_canonical(&db, thread_id, body);
        }
        seed_recent_membership_row(&db, "thread::retained-z", "2026-07-03T00:00:00Z", 10);
        seed_recent_membership_row(&db, "thread::retained-a", "2026-07-01T00:00:00Z", 20);
        // If this orphan were included in the insertion count, new-between
        // would move from bucket 1 to bucket 2 (after retained-a).
        seed_recent_membership_row(&db, "thread::orphan-slot", "2026-07-01T12:00:00Z", 30);
        seed_recent_membership_row(&db, "thread::hidden", "2026-07-05T00:00:00Z", 40);
        {
            let conn = db.conn().unwrap();
            conn.execute(
                "UPDATE recent_threads_meta SET activity_seq = 100 WHERE id = 1",
                [],
            )
            .unwrap();
            // Mode-only stale state proves every canonical live row is
            // rederived even when its body needs no normalization.
            conn.execute(
                "INSERT INTO thread_meta (
                    thread_id, excluded_from_recent, projected_at
                 ) VALUES ('thread::new-between', 1, '2026-07-17T00:00:00Z')",
                [],
            )
            .unwrap();
            // The unconditional whole-table zeroing must include rows outside
            // the canonical live universe without making them visible.
            conn.execute(
                "INSERT INTO thread_meta (
                    thread_id, default_list_hidden, excluded_from_recent, projected_at
                 ) VALUES ('thread::meta-only-hidden', 1, 1, '2026-07-17T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let incarnation = db.store_incarnation_id().unwrap();
        assert!(matches!(
            db.set_thread_favorite("thread::retained-z", true, 0, &incarnation)
                .unwrap(),
            FavoriteThreadResult::Updated { .. }
        ));
        assert!(matches!(
            db.set_thread_favorite("thread::retained-a", true, 1, &incarnation)
                .unwrap(),
            FavoriteThreadResult::Updated { .. }
        ));
        assert_eq!(
            db.thread_favorites_snapshot()
                .unwrap()
                .recent_threads
                .iter()
                .map(|row| row.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::retained-a", "thread::retained-z"]
        );

        let first = db
            .migrate_recent_membership_v2()
            .expect("membership cutover");
        assert_eq!(first.source_row_count, 8);
        assert!(!first.already_completed);
        assert_eq!(
            db.store_incarnation_id().unwrap(),
            incarnation,
            "sequence-only cutover must not rotate favorites CAS identity"
        );
        let ascending = recent_membership_rows_ascending(&db);
        assert_eq!(
            ascending,
            vec![
                ("thread::new-missing".to_owned(), 101),
                ("thread::retained-z".to_owned(), 102),
                ("thread::new-between".to_owned(), 103),
                ("thread::new-same".to_owned(), 104),
                ("thread::retained-a".to_owned(), 105),
                ("thread::new-latest".to_owned(), 106),
            ],
            "retained order, count insertion, missing-time bottom, and same-time id tie"
        );
        assert_eq!(
            db.thread_favorites_snapshot()
                .unwrap()
                .recent_threads
                .iter()
                .map(|row| row.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::retained-a", "thread::retained-z"],
            "favorites retain their pre-cutover relative display order"
        );

        let conn = db.conn().unwrap();
        let (meta_high_water, row_high_water): (i64, i64) = conn
            .query_row(
                "SELECT
                    (SELECT activity_seq FROM recent_threads_meta WHERE id = 1),
                    (SELECT MAX(activity_seq) FROM recent_threads)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((meta_high_water, row_high_water), (106, 106));
        let index_rows = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master
                      WHERE type = 'index'
                        AND name IN (
                            'idx_recent_threads_activity_seq',
                            'idx_recent_threads_task_activity_seq',
                            'idx_recent_threads_non_task_activity_seq'
                        )
                      ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(index_rows.len(), 3);
        let unique: i64 = conn
            .query_row(
                "SELECT [unique] FROM pragma_index_list('recent_threads')
                  WHERE name = 'idx_recent_threads_activity_seq'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(unique, 1);
        for path in [
            "$.exclude_from_recent",
            "$.excludeFromRecent",
            "$.metadata.exclude_from_recent",
            "$.metadata.excludeFromRecent",
        ] {
            let residual: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM thread_records
                      WHERE json_type(body, ?1) IS NOT NULL",
                    params![path],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(residual, 0, "residual canonical path {path}");
        }
        let excluded_residual: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM thread_meta WHERE excluded_from_recent != 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(excluded_residual, 0);
        let side_chat_body: String = conn
            .query_row(
                "SELECT body FROM thread_records WHERE key = 'thread::side-chat'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&side_chat_body).unwrap()["hidden"],
            true
        );
        drop(conn);
        let mut parity_conn = db.conn().unwrap();
        let parity_tx = parity_conn.transaction().unwrap();
        assert_recent_membership_parity_tx(&parity_tx).unwrap();
        parity_tx.commit().unwrap();
        drop(parity_conn);

        let second = db
            .migrate_recent_membership_v2()
            .expect("idempotent cutover");
        assert!(second.already_completed);
        assert_eq!(second.updated_row_count, 0);
        assert_eq!(recent_membership_rows_ascending(&db), ascending);
    }

    #[test]
    fn recent_membership_cutover_reruns_once_per_import_generation() {
        let db = GaryxDbService::memory().expect("memory db");
        prepare_recent_membership_prerequisites(&db);
        seed_recent_membership_canonical(
            &db,
            "thread::generation-zero",
            json!({
                "thread_id": "thread::generation-zero",
                "automation_thread_mode": "generated_thread",
                "exclude_from_recent": true
            }),
        );
        assert!(!db.migrate_recent_membership_v2().unwrap().already_completed);
        assert!(db.migrate_recent_membership_v2().unwrap().already_completed);

        seed_recent_membership_canonical(
            &db,
            "thread::generation-one",
            json!({
                "thread_id": "thread::generation-one",
                "automation_thread_mode": "generated_thread",
                "metadata": {"excludeFromRecent": true}
            }),
        );
        db.conn()
            .unwrap()
            .execute(
                "INSERT INTO thread_meta (
                    thread_id, excluded_from_recent, projected_at
                 ) VALUES ('thread::generation-one', 1, '2026-07-17T00:00:00Z')",
                [],
            )
            .unwrap();
        assert_eq!(db.commit_legacy_import(2, false).unwrap(), 1);
        assert!(
            !db.migrate_thread_meta_summary_v1()
                .unwrap()
                .already_completed
        );
        let generation_one = db.migrate_recent_membership_v2().unwrap();
        assert!(!generation_one.already_completed);
        assert!(db.migrate_recent_membership_v2().unwrap().already_completed);

        let conn = db.conn().unwrap();
        let based_on: i64 = conn
            .query_row(
                "SELECT based_on_import_generation
                   FROM projection_states
                  WHERE projection_name = ?1",
                params![RECENT_MEMBERSHIP_MIGRATION_NAME],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(based_on, 1);
        drop(conn);
        assert_eq!(
            recent_membership_rows_ascending(&db)
                .into_iter()
                .map(|(thread_id, _)| thread_id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "thread::generation-zero".to_owned(),
                "thread::generation-one".to_owned(),
            ])
        );
    }

    #[test]
    fn recent_membership_cutover_high_water_uses_larger_existing_row_sequence() {
        let db = GaryxDbService::memory().expect("memory db");
        prepare_recent_membership_prerequisites(&db);
        seed_recent_membership_canonical(
            &db,
            "thread::row-high-retained",
            json!({"thread_id": "thread::row-high-retained"}),
        );
        seed_recent_membership_canonical(
            &db,
            "thread::row-high-new",
            json!({
                "thread_id": "thread::row-high-new",
                "updated_at": "2026-07-18T00:00:00Z"
            }),
        );
        seed_recent_membership_row(&db, "thread::row-high-retained", "2026-07-17T00:00:00Z", 50);
        db.conn()
            .unwrap()
            .execute(
                "UPDATE recent_threads_meta SET activity_seq = 10 WHERE id = 1",
                [],
            )
            .unwrap();

        db.migrate_recent_membership_v2().unwrap();
        assert_eq!(
            recent_membership_rows_ascending(&db),
            vec![
                ("thread::row-high-retained".to_owned(), 51),
                ("thread::row-high-new".to_owned(), 52),
            ]
        );
        let meta: i64 = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(meta, 52);
    }

    #[test]
    fn recent_membership_cutover_safe_integer_failure_rolls_back_and_retries() {
        let db = GaryxDbService::memory().expect("memory db");
        prepare_recent_membership_prerequisites(&db);
        seed_recent_membership_canonical(
            &db,
            "thread::at-sequence-limit",
            json!({
                "thread_id": "thread::at-sequence-limit",
                "exclude_from_recent": true
            }),
        );
        db.conn()
            .unwrap()
            .execute(
                "UPDATE recent_threads_meta SET activity_seq = ?1 WHERE id = 1",
                params![MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE - 1],
            )
            .unwrap();

        assert!(db.migrate_recent_membership_v2().is_err());
        let conn = db.conn().unwrap();
        let body: String = conn
            .query_row(
                "SELECT body FROM thread_records WHERE key = 'thread::at-sequence-limit'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            serde_json::from_str::<Value>(&body)
                .unwrap()
                .get("exclude_from_recent")
                .is_some()
        );
        let marker_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM projection_states WHERE projection_name = ?1",
                params![RECENT_MEMBERSHIP_MIGRATION_NAME],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker_count, 0);
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type = 'index'
                    AND name IN (
                        'idx_recent_threads_activity_seq',
                        'idx_recent_threads_task_activity_seq',
                        'idx_recent_threads_non_task_activity_seq'
                    )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 3, "failed cutover restores dropped indexes");
        conn.execute(
            "UPDATE recent_threads_meta SET activity_seq = 0 WHERE id = 1",
            [],
        )
        .unwrap();
        drop(conn);
        assert!(!db.migrate_recent_membership_v2().unwrap().already_completed);
    }

    #[test]
    fn recent_membership_cutover_registration_requires_summary_then_activity() {
        let db = GaryxDbService::memory().expect("memory db");
        assert!(db.migrate_recent_membership_v2().is_err());
        db.migrate_thread_meta_summary_v1().unwrap();
        assert!(db.migrate_recent_membership_v2().is_err());
        db.migrate_recent_thread_activity_seq_v1().unwrap();
        assert!(!db.migrate_recent_membership_v2().unwrap().already_completed);

        let source = include_str!("mod.rs");
        let registrations = source
            .split_once("pub(crate) fn run_thread_data_startup_migrations")
            .unwrap()
            .1
            .split_once("pub(crate) fn migrate_recent_thread_activity_seq_v1")
            .unwrap()
            .0;
        let summary = registrations
            .find("migrate_thread_meta_summary_v1")
            .unwrap();
        let activity = registrations
            .find("migrate_recent_thread_activity_seq_v1")
            .unwrap();
        let membership = registrations.find("migrate_recent_membership_v2").unwrap();
        assert!(summary < activity && activity < membership);
    }

    #[test]
    fn recent_membership_side_chat_selector_is_exact_and_strips_flags() {
        let mut cases = [
            ("top source", json!({"source": "side_chat"}), true),
            (
                "metadata source",
                json!({"metadata": {"source": "side_chat"}}),
                true,
            ),
            (
                "both sources",
                json!({"source": "side_chat", "metadata": {"source": "side_chat"}}),
                true,
            ),
            (
                "parent only",
                json!({"side_chat_parent_thread_id": "thread::parent"}),
                false,
            ),
            (
                "exclusion only",
                json!({
                    "exclude_from_recent": true,
                    "excludeFromRecent": true,
                    "metadata": {
                        "exclude_from_recent": true,
                        "excludeFromRecent": true
                    }
                }),
                false,
            ),
            (
                "malformed payload",
                json!({"source": 42, "metadata": ["side_chat"]}),
                false,
            ),
        ];
        for (label, data, expected_hidden) in &mut cases {
            normalize_recent_membership_canonical_record(data);
            assert_eq!(
                data.get("hidden").and_then(Value::as_bool),
                (*expected_hidden).then_some(true),
                "{label}"
            );
            if let Some(object) = data.as_object() {
                assert!(object.get("exclude_from_recent").is_none(), "{label}");
                assert!(object.get("excludeFromRecent").is_none(), "{label}");
                if let Some(metadata) = object.get("metadata").and_then(Value::as_object) {
                    assert!(metadata.get("exclude_from_recent").is_none(), "{label}");
                    assert!(metadata.get("excludeFromRecent").is_none(), "{label}");
                }
            }
        }
    }

    fn prepare_recent_membership_prerequisites(service: &GaryxDbService) {
        service
            .migrate_thread_meta_summary_v1()
            .expect("summary prerequisite");
        service
            .migrate_recent_thread_activity_seq_v1()
            .expect("activity prerequisite");
    }

    fn seed_recent_membership_canonical(service: &GaryxDbService, thread_id: &str, body: Value) {
        service
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO thread_records (key, body, updated_at, recorded_at)
                 VALUES (?1, ?2, NULL, '2026-07-17T00:00:00Z')",
                params![thread_id, body.to_string()],
            )
            .unwrap();
    }

    fn seed_recent_membership_row(
        service: &GaryxDbService,
        thread_id: &str,
        last_active_at: &str,
        activity_seq: i64,
    ) {
        service
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO recent_threads (
                    thread_id, title, thread_type, last_active_at, activity_seq, recorded_at
                 ) VALUES (?1, ?1, 'chat', ?2, ?3, '2026-07-17T00:00:00Z')",
                params![thread_id, last_active_at, activity_seq],
            )
            .unwrap();
    }

    fn recent_membership_rows_ascending(service: &GaryxDbService) -> Vec<(String, i64)> {
        let conn = service.conn().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT thread_id, activity_seq
                   FROM recent_threads
                  ORDER BY activity_seq ASC",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn seed_favorite_thread(service: &GaryxDbService, thread_id: &str, recent: bool) {
        service
            .write_thread_record_with_projections(
                thread_id,
                &json!({"thread_id": thread_id}).to_string(),
                Some("2026-07-16T00:00:00Z"),
                None,
            )
            .expect("seed favorite thread record");
        if recent {
            service
                .upsert_recent_thread(sample_recent_draft(thread_id))
                .expect("seed favorite recent projection");
        }
    }

    fn seed_summary_favorite_tx(
        tx: &Transaction<'_>,
        thread_id: &str,
        favorited_at: &str,
        hidden: bool,
        excluded_from_recent: bool,
    ) {
        tx.execute(
            "INSERT INTO thread_favorites (thread_id, favorited_at) VALUES (?1, ?2)",
            params![thread_id, favorited_at],
        )
        .expect("seed favorite membership");
        tx.execute(
            "INSERT INTO thread_meta (
                thread_id, thread_label, default_list_hidden, excluded_from_recent,
                sort_updated_at_us, search_text, projected_at
             ) VALUES (?1, ?2, ?3, ?4, 0, '', '2026-07-17T00:00:00Z')",
            params![
                thread_id,
                format!("Title for {thread_id}"),
                if hidden { 1 } else { 0 },
                if excluded_from_recent { 1 } else { 0 },
            ],
        )
        .expect("seed favorite summary");
    }

    fn seed_summary_recent_tx(tx: &Transaction<'_>, thread_id: &str, activity_seq: i64) {
        tx.execute(
            "INSERT INTO recent_threads (
                thread_id, title, thread_type, message_count, last_message_preview,
                run_state, last_active_at, activity_seq, recorded_at
             ) VALUES (?1, ?2, 'chat', 0, '', 'idle',
                       '2026-07-17T00:00:00Z', ?3, '2026-07-17T00:00:00Z')",
            params![thread_id, format!("Title for {thread_id}"), activity_seq],
        )
        .expect("seed recent favorite");
    }

    fn seed_task_kind_migration_row(
        service: &GaryxDbService,
        thread_id: &str,
        body: &str,
        has_task_projection: bool,
    ) {
        let conn = service.conn().expect("conn");
        conn.execute(
            "INSERT INTO thread_records (key, body, updated_at, recorded_at)
             VALUES (?1, ?2, '2026-07-01T00:00:00Z', '2026-07-01T00:00:01Z')",
            params![thread_id, body],
        )
        .expect("seed thread record");
        conn.execute(
            "INSERT INTO recent_threads (
                thread_id, title, thread_type, last_active_at, recorded_at
             ) VALUES (?1, 'Legacy title', 'chat',
                       '2026-07-01T00:00:00Z', '2026-07-01T00:00:01Z')",
            params![thread_id],
        )
        .expect("seed recent row");
        conn.execute(
            "INSERT INTO thread_meta (
                thread_id, thread_type, thread_label, updated_at, projected_at
             ) VALUES (?1, 'chat', 'Legacy title',
                       '2026-07-01T00:00:00Z', '2026-07-01T00:00:01Z')",
            params![thread_id],
        )
        .expect("seed meta row");
        if has_task_projection {
            conn.execute(
                "INSERT INTO task_projection (
                    thread_id, number, status, title, creator_json, creator_id,
                    updated_by_json, created_at, updated_at, source_updated_at,
                    source_events_len, projected_at
                 ) VALUES (
                    ?1, 41, 'todo', 'Legacy task',
                    '{\"kind\":\"agent\",\"agent_id\":\"test-agent\"}',
                    'test-agent',
                    '{\"kind\":\"agent\",\"agent_id\":\"test-agent\"}',
                    '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z',
                    '2026-07-01T00:00:00Z', 1, '2026-07-01T00:00:01Z'
                 )",
                params![thread_id],
            )
            .expect("seed task projection");
        }
    }

    fn raw_legacy_import_generation(service: &GaryxDbService) -> Option<i64> {
        service
            .conn()
            .expect("conn")
            .query_row(
                "SELECT source_row_count FROM projection_states
                  WHERE projection_name = ?1",
                params![LEGACY_IMPORT_GENERATION_NAME],
                |row| row.get(0),
            )
            .optional()
            .expect("generation query")
    }

    fn seed_pre_generation_cutover_markers(service: &GaryxDbService) {
        let conn = service.conn().expect("conn");
        for (name, version) in [
            (
                RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
            ),
            (
                ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
                ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION,
            ),
        ] {
            conn.execute(
                "INSERT INTO projection_states (
                    projection_name, projection_version, source_row_count, projected_at
                 ) VALUES (?1, ?2, 0, '2026-07-15T00:00:00Z')",
                params![name, version],
            )
            .expect("seed pre-generation cutover marker");
        }
    }

    fn seed_retired_thread_message_routes_table(service: &GaryxDbService) {
        service
            .conn()
            .expect("conn")
            .execute_batch(
                "CREATE TABLE thread_message_routes (message_id TEXT NOT NULL);
                 INSERT INTO thread_message_routes (message_id) VALUES ('legacy-message');",
            )
            .expect("seed retired message routes table");
    }

    #[test]
    fn drop_thread_message_routes_migration_is_atomic_and_one_shot() {
        let service = GaryxDbService::memory().expect("memory db");
        seed_retired_thread_message_routes_table(&service);

        let failed = service.drop_thread_message_routes_v1_inner(|_| {
            Err(GaryxDbError::Configuration(
                "injected post-drop failure".to_owned(),
            ))
        });
        assert!(failed.is_err());
        assert!(
            sqlite_table_exists(
                &service.conn().expect("conn after rollback"),
                "thread_message_routes"
            )
            .expect("table check after rollback"),
            "the table drop must roll back when marker recording cannot commit"
        );
        assert!(
            !service
                .projection_state_exists(
                    DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME,
                    DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION,
                )
                .expect("marker after rollback")
        );

        let first = service
            .drop_thread_message_routes_v1()
            .expect("first migration");
        assert_eq!(first.source_row_count, 1);
        assert_eq!(first.updated_row_count, 1);
        assert!(!first.already_completed);
        assert!(
            !sqlite_table_exists(
                &service.conn().expect("conn after migration"),
                "thread_message_routes"
            )
            .expect("table check after migration")
        );

        seed_retired_thread_message_routes_table(&service);
        let second = service
            .drop_thread_message_routes_v1()
            .expect("completed migration skips");
        assert!(second.already_completed);
        assert_eq!(second.updated_row_count, 0);
        assert!(
            sqlite_table_exists(
                &service.conn().expect("conn after skipped rerun"),
                "thread_message_routes"
            )
            .expect("table check after skipped rerun"),
            "an existing marker must prevent the migration from running again"
        );
    }

    #[test]
    fn drop_thread_message_routes_migration_tolerates_missing_table() {
        let service = GaryxDbService::memory().expect("memory db");
        let summary = service
            .drop_thread_message_routes_v1()
            .expect("missing table is a no-op");
        assert_eq!(summary.source_row_count, 0);
        assert_eq!(summary.updated_row_count, 0);
        assert!(!summary.already_completed);
        assert!(
            service
                .projection_state_exists(
                    DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME,
                    DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION,
                )
                .expect("migration marker")
        );
    }

    #[test]
    fn legacy_import_generation_commit_is_atomic_monotonic_and_recovery_clears_retirement() {
        let service = GaryxDbService::memory().expect("memory db");
        let fresh_incarnation = service.store_incarnation_id().unwrap();
        service.fail_test_db_call(TestDbFaultPoint::LegacyImportCommit, 1);
        assert!(service.commit_legacy_import(0, false).is_err());
        assert_eq!(service.legacy_import_marker_pair().unwrap(), (false, false));
        assert_eq!(raw_legacy_import_generation(&service), None);

        assert_eq!(service.commit_legacy_import(0, false).unwrap(), 1);
        assert_eq!(service.legacy_import_marker_pair().unwrap(), (true, false));
        assert_eq!(raw_legacy_import_generation(&service), Some(1));
        assert_eq!(service.store_incarnation_id().unwrap(), fresh_incarnation);
        service.record_legacy_archive_retirement().unwrap();
        assert_eq!(service.legacy_import_marker_pair().unwrap(), (true, true));
        let generation_one_incarnation = service.store_incarnation_id().unwrap();

        assert!(
            service
                .clear_projection_state(crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME)
                .unwrap()
        );
        assert_eq!(service.legacy_import_marker_pair().unwrap(), (false, true));
        service.fail_test_db_call(TestDbFaultPoint::LegacyImportAfterIncarnationRotation, 1);
        assert!(service.commit_legacy_import(3, true).is_err());
        assert_eq!(
            service.store_incarnation_id().unwrap(),
            generation_one_incarnation,
            "a crash after rotation but before commit must roll the identity back"
        );
        assert_eq!(service.legacy_import_marker_pair().unwrap(), (false, true));
        assert_eq!(raw_legacy_import_generation(&service), Some(1));
        assert_eq!(service.commit_legacy_import(3, true).unwrap(), 2);
        assert_eq!(service.legacy_import_marker_pair().unwrap(), (true, false));
        assert_eq!(raw_legacy_import_generation(&service), Some(2));
        assert_ne!(
            service.store_incarnation_id().unwrap(),
            generation_one_incarnation,
            "a committed recovery must rotate exactly with its marker transaction"
        );
        seed_favorite_thread(&service, "thread::recovered-store", false);
        assert!(matches!(
            service
                .set_thread_favorite(
                    "thread::recovered-store",
                    true,
                    0,
                    &generation_one_incarnation,
                )
                .expect("old incarnation write is classified"),
            FavoriteThreadResult::WrongIncarnation(ref page)
                if page.revision == 0 && page.favorites.is_empty()
        ));
        assert!(
            service
                .clear_projection_state(LEGACY_IMPORT_GENERATION_NAME)
                .is_err(),
            "the generation owner can never be deleted"
        );
        assert_eq!(raw_legacy_import_generation(&service), Some(2));
    }

    #[test]
    fn pre_generation_cutover_markers_seed_one_without_rerun_then_generation_two_reruns_once() {
        let service = GaryxDbService::memory().expect("memory db");
        service
            .record_projection_state(
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
                1,
            )
            .unwrap();
        seed_pre_generation_cutover_markers(&service);
        seed_task_kind_migration_row(
            &service,
            "thread::pre-generation-task",
            r#"{"thread_id":"thread::pre-generation-task","thread_title_source":"task"}"#,
            false,
        );

        service.run_thread_data_startup_migrations().unwrap();
        assert_eq!(raw_legacy_import_generation(&service), Some(1));
        let before_recovery: Value = serde_json::from_str(
            &service
                .get_thread_record_body("thread::pre-generation-task")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            before_recovery.get("thread_kind").is_none(),
            "pre-generation markers are pinned to generation 1 and must not rerun"
        );

        service.record_legacy_archive_retirement().unwrap();
        service
            .clear_projection_state(crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME)
            .unwrap();
        assert_eq!(service.commit_legacy_import(1, true).unwrap(), 2);
        service.run_thread_data_startup_migrations().unwrap();
        let after_recovery: Value = serde_json::from_str(
            &service
                .get_thread_record_body("thread::pre-generation-task")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(after_recovery["thread_kind"], "task");
        assert_eq!(raw_legacy_import_generation(&service), Some(2));
        assert!(
            service
                .migrate_recent_task_thread_kind_v1()
                .unwrap()
                .already_completed
        );
        assert!(
            service
                .migrate_endpoint_holder_dedup_v1()
                .unwrap()
                .already_completed
        );
    }

    #[test]
    fn lazy_generation_seed_failure_aborts_without_marker_movement() {
        let service = GaryxDbService::memory().expect("memory db");
        service
            .record_projection_state(
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
                0,
            )
            .unwrap();
        seed_pre_generation_cutover_markers(&service);
        service.fail_test_db_call(TestDbFaultPoint::LegacyGenerationSeedWrite, 1);

        assert!(service.run_thread_data_startup_migrations().is_err());
        assert_eq!(raw_legacy_import_generation(&service), None);
        assert_eq!(service.legacy_import_marker_pair().unwrap(), (true, false));
        let conn = service.conn().expect("conn");
        let unchanged: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM projection_states
                  WHERE projection_name IN (?1, ?2)
                    AND based_on_import_generation IS NULL",
                params![
                    RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                    ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
                ],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(unchanged, 2);
    }

    #[test]
    fn recent_task_thread_kind_migration_updates_canonical_and_type_projections() {
        let service = GaryxDbService::memory().expect("memory db");
        seed_task_kind_migration_row(
            &service,
            "thread::legacy-overlay",
            r#"{"thread_id":"thread::legacy-overlay","label":"Overlay title","updated_at":"2026-07-01T00:00:00Z","task":{"number":41}}"#,
            true,
        );
        seed_task_kind_migration_row(
            &service,
            "thread::legacy-title-source",
            r#"{"thread_id":"thread::legacy-title-source","label":"Retained title","thread_title_source":"task","updated_at":"2026-07-01T00:00:00Z"}"#,
            false,
        );
        seed_task_kind_migration_row(
            &service,
            "thread::already-durable",
            r#"{"thread_id":"thread::already-durable","label":"Durable title","thread_kind":"task","updated_at":"2026-07-01T00:00:00Z"}"#,
            false,
        );
        seed_task_kind_migration_row(
            &service,
            "thread::prefix-only",
            r##"{"thread_id":"thread::prefix-only","label":"#TASK-99 ordinary chat","updated_at":"2026-07-01T00:00:00Z"}"##,
            false,
        );

        let summary = service
            .migrate_recent_task_thread_kind_v1()
            .expect("migration succeeds");
        assert_eq!(summary.source_row_count, 3);
        assert_eq!(summary.updated_row_count, 2);
        assert!(!summary.already_completed);

        for thread_id in [
            "thread::legacy-overlay",
            "thread::legacy-title-source",
            "thread::already-durable",
        ] {
            let body = service
                .get_thread_record_body(thread_id)
                .expect("read body")
                .expect("body exists");
            let body: Value = serde_json::from_str(&body).expect("valid body");
            assert_eq!(body["thread_kind"], "task", "{thread_id}");
            assert_eq!(body["updated_at"], "2026-07-01T00:00:00Z");
        }
        let prefix_body: Value = serde_json::from_str(
            &service
                .get_thread_record_body("thread::prefix-only")
                .expect("read prefix body")
                .expect("prefix body exists"),
        )
        .expect("valid prefix body");
        assert!(prefix_body.get("thread_kind").is_none());

        let conn = service.conn().expect("conn");
        for thread_id in [
            "thread::legacy-overlay",
            "thread::legacy-title-source",
            "thread::already-durable",
        ] {
            let recent: (String, String, String) = conn
                .query_row(
                    "SELECT thread_type, title, last_active_at
                       FROM recent_threads WHERE thread_id = ?1",
                    params![thread_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("recent row");
            assert_eq!(recent.0, "task", "{thread_id}");
            assert_eq!(recent.1, "Legacy title");
            assert_eq!(recent.2, "2026-07-01T00:00:00Z");
            let meta_type: String = conn
                .query_row(
                    "SELECT thread_type FROM thread_meta WHERE thread_id = ?1",
                    params![thread_id],
                    |row| row.get(0),
                )
                .expect("meta row");
            assert_eq!(meta_type, "task", "{thread_id}");
        }
        let prefix_type: String = conn
            .query_row(
                "SELECT thread_type FROM recent_threads WHERE thread_id = 'thread::prefix-only'",
                [],
                |row| row.get(0),
            )
            .expect("prefix recent row");
        assert_eq!(prefix_type, "chat");
        drop(conn);
        assert!(
            service
                .projection_state_matches(
                    RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                    RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
                    3,
                )
                .expect("marker")
        );
    }

    #[test]
    fn recent_task_thread_kind_migration_records_zero_and_never_reruns() {
        let service = GaryxDbService::memory().expect("memory db");
        let first = service
            .migrate_recent_task_thread_kind_v1()
            .expect("zero-row migration succeeds");
        assert_eq!(first.source_row_count, 0);
        assert_eq!(first.updated_row_count, 0);
        assert!(!first.already_completed);

        seed_task_kind_migration_row(
            &service,
            "thread::late-legacy-task",
            r#"{"thread_id":"thread::late-legacy-task","thread_title_source":"task"}"#,
            false,
        );
        let second = service
            .migrate_recent_task_thread_kind_v1()
            .expect("completed migration skips");
        assert_eq!(second.source_row_count, 0);
        assert_eq!(second.updated_row_count, 0);
        assert!(second.already_completed);
        let body: Value = serde_json::from_str(
            &service
                .get_thread_record_body("thread::late-legacy-task")
                .expect("read body")
                .expect("body exists"),
        )
        .expect("valid body");
        assert!(body.get("thread_kind").is_none());
    }

    #[test]
    fn recent_task_thread_kind_migration_is_atomic_on_projection_failure() {
        let service = GaryxDbService::memory().expect("memory db");
        seed_task_kind_migration_row(
            &service,
            "thread::atomic-legacy-task",
            r#"{"thread_id":"thread::atomic-legacy-task","thread_title_source":"task"}"#,
            false,
        );
        service
            .conn()
            .expect("conn")
            .execute_batch(
                "CREATE TRIGGER fail_task_kind_projection
                 BEFORE UPDATE OF thread_type ON recent_threads
                 WHEN NEW.thread_type = 'task'
                 BEGIN
                     SELECT RAISE(ABORT, 'forced task-kind projection failure');
                 END;",
            )
            .expect("install failure trigger");

        assert!(
            service.migrate_recent_task_thread_kind_v1().is_err(),
            "projection failure must abort the migration"
        );
        let body: Value = serde_json::from_str(
            &service
                .get_thread_record_body("thread::atomic-legacy-task")
                .expect("read body")
                .expect("body exists"),
        )
        .expect("valid body");
        assert!(body.get("thread_kind").is_none());
        let conn = service.conn().expect("conn");
        let recent_type: String = conn
            .query_row(
                "SELECT thread_type FROM recent_threads
                  WHERE thread_id = 'thread::atomic-legacy-task'",
                [],
                |row| row.get(0),
            )
            .expect("recent type");
        assert_eq!(recent_type, "chat");
        drop(conn);
        assert!(
            !service
                .projection_state_exists(
                    RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                    RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
                )
                .expect("marker lookup")
        );
    }

    fn seed_endpoint_holder_record(
        service: &GaryxDbService,
        thread_id: &str,
        updated_at: &str,
        bindings: Value,
    ) {
        let body = json!({
            "thread_id": thread_id,
            "label": format!("Title for {thread_id}"),
            "workspace_dir": "/workspace/test",
            "updated_at": updated_at,
            "channel_bindings": bindings,
        });
        service
            .write_thread_record_with_projections(
                thread_id,
                &serde_json::to_string(&body).expect("record json"),
                Some(updated_at),
                None,
            )
            .expect("seed holder record");
    }

    fn test_binding(binding_key: &str, label: &str) -> Value {
        json!({
            "channel": "telegram",
            "account_id": "main",
            "binding_key": binding_key,
            "chat_id": binding_key,
            "delivery_target_type": "chat_id",
            "delivery_target_id": binding_key,
            "display_label": label,
            "last_inbound_at": "2026-07-01T00:00:00Z",
        })
    }

    #[test]
    fn endpoint_holder_dedup_migration_keeps_preferred_holder_and_syncs_projection() {
        let service = GaryxDbService::memory().expect("memory db");
        seed_endpoint_holder_record(
            &service,
            "thread::holder-old",
            "2026-07-01T00:00:00Z",
            json!([
                test_binding("1000000001", "Old duplicate"),
                test_binding("1000000002", "Old unique"),
            ]),
        );
        seed_endpoint_holder_record(
            &service,
            "thread::holder-new",
            "2026-07-02T00:00:00Z",
            json!([test_binding("1000000001", "New duplicate")]),
        );
        service
            .conn()
            .expect("conn")
            .execute(
                "INSERT INTO thread_channel_endpoints (
                    endpoint_key, channel, account_id, binding_key, chat_id,
                    thread_id, projected_at
                 ) VALUES (
                    'telegram::main::1000000001', 'telegram', 'main',
                    '1000000001', '1000000001', 'thread::holder-old',
                    '2026-07-01T00:00:00Z'
                 )",
                [],
            )
            .expect("seed stale projection owner");

        let summary = service
            .migrate_endpoint_holder_dedup_v1()
            .expect("dedup migration");
        assert_eq!(summary.source_row_count, 3);
        assert_eq!(summary.updated_row_count, 1);
        assert!(!summary.already_completed);

        let old: Value = serde_json::from_str(
            &service
                .get_thread_record_body("thread::holder-old")
                .expect("old body")
                .expect("old record"),
        )
        .expect("old json");
        let new: Value = serde_json::from_str(
            &service
                .get_thread_record_body("thread::holder-new")
                .expect("new body")
                .expect("new record"),
        )
        .expect("new json");
        assert_eq!(old["updated_at"], "2026-07-01T00:00:00Z");
        assert_eq!(new["updated_at"], "2026-07-02T00:00:00Z");
        let old_bindings = garyx_router::bindings_from_value(&old);
        let new_bindings = garyx_router::bindings_from_value(&new);
        assert_eq!(old_bindings.len(), 1);
        assert_eq!(old_bindings[0].binding_key, "1000000002");
        assert_eq!(new_bindings.len(), 1);
        assert_eq!(new_bindings[0].binding_key, "1000000001");

        let projected = service
            .list_thread_channel_endpoints()
            .expect("endpoint projection");
        let duplicate = projected
            .iter()
            .find(|row| row.endpoint_key == "telegram::main::1000000001")
            .expect("deduplicated endpoint");
        assert_eq!(duplicate.thread_id.as_deref(), Some("thread::holder-new"));
        assert_eq!(duplicate.display_label, "New duplicate");
        let unique = projected
            .iter()
            .find(|row| row.endpoint_key == "telegram::main::1000000002")
            .expect("unique endpoint");
        assert_eq!(unique.thread_id.as_deref(), Some("thread::holder-old"));

        let second = service
            .migrate_endpoint_holder_dedup_v1()
            .expect("idempotent rerun");
        assert!(second.already_completed);
        assert_eq!(second.source_row_count, 3);
        assert_eq!(second.updated_row_count, 0);
    }

    #[test]
    fn endpoint_holder_dedup_migration_records_zero_and_does_not_rerun() {
        let service = GaryxDbService::memory().expect("memory db");
        let first = service
            .migrate_endpoint_holder_dedup_v1()
            .expect("zero migration");
        assert_eq!(first.source_row_count, 0);
        assert!(!first.already_completed);

        seed_endpoint_holder_record(
            &service,
            "thread::late-holder-a",
            "2026-07-01T00:00:00Z",
            json!([test_binding("1000000003", "Late A")]),
        );
        seed_endpoint_holder_record(
            &service,
            "thread::late-holder-b",
            "2026-07-02T00:00:00Z",
            json!([test_binding("1000000003", "Late B")]),
        );
        let second = service
            .migrate_endpoint_holder_dedup_v1()
            .expect("completed migration skips");
        assert!(second.already_completed);
        assert_eq!(second.source_row_count, 0);
        for thread_id in ["thread::late-holder-a", "thread::late-holder-b"] {
            let body: Value = serde_json::from_str(
                &service
                    .get_thread_record_body(thread_id)
                    .expect("body read")
                    .expect("body exists"),
            )
            .expect("body json");
            assert_eq!(garyx_router::bindings_from_value(&body).len(), 1);
        }
    }

    #[test]
    fn endpoint_holder_dedup_migration_is_atomic_on_projection_failure() {
        let service = GaryxDbService::memory().expect("memory db");
        for (thread_id, updated_at) in [
            ("thread::atomic-holder-a", "2026-07-01T00:00:00Z"),
            ("thread::atomic-holder-b", "2026-07-02T00:00:00Z"),
        ] {
            seed_endpoint_holder_record(
                &service,
                thread_id,
                updated_at,
                json!([test_binding("1000000004", "Atomic")]),
            );
        }
        service
            .conn()
            .expect("conn")
            .execute_batch(
                "CREATE TRIGGER fail_endpoint_dedup_projection
                 BEFORE INSERT ON thread_channel_endpoints
                 BEGIN
                     SELECT RAISE(ABORT, 'forced endpoint projection failure');
                 END;",
            )
            .expect("failure trigger");

        assert!(service.migrate_endpoint_holder_dedup_v1().is_err());
        for thread_id in ["thread::atomic-holder-a", "thread::atomic-holder-b"] {
            let body: Value = serde_json::from_str(
                &service
                    .get_thread_record_body(thread_id)
                    .expect("body read")
                    .expect("body exists"),
            )
            .expect("body json");
            assert_eq!(garyx_router::bindings_from_value(&body).len(), 1);
        }
        assert!(
            !service
                .projection_state_exists(
                    ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
                    ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION,
                )
                .expect("marker lookup")
        );
    }

    #[test]
    fn thread_record_write_read_list_delete_round_trip() {
        let dir = tempfile::tempdir().expect("temp dir");
        let service = GaryxDbService::open(dir.path().join("garyx-db.sqlite3")).expect("db opens");

        service
            .write_thread_record_with_projections(
                "thread::alpha",
                r#"{"thread_id":"thread::alpha"}"#,
                Some("2026-07-08T00:00:00Z"),
                None,
            )
            .expect("write record");
        service
            .write_thread_record_with_projections(
                "meta::known_channel_endpoints",
                r#"{"endpoints":[]}"#,
                None,
                None,
            )
            .expect("write meta record");

        // Reads go through the dedicated reader connection.
        assert_eq!(
            service
                .get_thread_record_body("thread::alpha")
                .expect("get"),
            Some(r#"{"thread_id":"thread::alpha"}"#.to_owned())
        );
        assert!(
            service
                .thread_record_exists("thread::alpha")
                .expect("exists")
        );
        assert!(
            !service
                .thread_record_exists("thread::missing")
                .expect("exists missing")
        );
        assert_eq!(
            service
                .list_thread_record_keys(Some("thread::"))
                .expect("list"),
            vec!["thread::alpha".to_owned()]
        );
        assert_eq!(
            service
                .list_thread_record_keys(None)
                .expect("list all")
                .len(),
            2
        );

        // Overwrite replaces the body.
        service
            .write_thread_record_with_projections(
                "thread::alpha",
                r#"{"thread_id":"thread::alpha","label":"v2"}"#,
                None,
                None,
            )
            .expect("overwrite");
        assert!(
            service
                .get_thread_record_body("thread::alpha")
                .expect("get v2")
                .expect("body")
                .contains("v2")
        );

        assert!(
            service
                .delete_thread_record_with_projections("thread::alpha")
                .expect("delete")
        );
        assert!(
            !service
                .delete_thread_record_with_projections("thread::alpha")
                .expect("delete again")
        );
        assert_eq!(
            service
                .get_thread_record_body("thread::alpha")
                .expect("get after delete"),
            None
        );
    }

    #[test]
    fn thread_record_key_prefix_listing_is_case_sensitive() {
        // SQLite LIKE is ASCII case-insensitive; the store contract
        // (File/InMemory starts_with) is case-sensitive (#TASK-1896).
        let service = GaryxDbService::memory().expect("memory db");
        for key in ["thread::lower", "Thread::upper"] {
            service
                .write_thread_record_with_projections(key, "{}", None, None)
                .expect("write");
        }
        assert_eq!(
            service
                .list_thread_record_keys(Some("thread::"))
                .expect("list"),
            vec!["thread::lower".to_owned()]
        );
        assert_eq!(
            service
                .list_thread_record_keys(Some("Thread::"))
                .expect("list upper"),
            vec!["Thread::upper".to_owned()]
        );
    }

    #[test]
    fn thread_record_write_derives_projections_in_the_same_transaction() {
        let service = GaryxDbService::memory().expect("memory db");
        let thread_id = "thread::projected";

        service
            .write_thread_record_with_projections(
                thread_id,
                r#"{"thread_id":"thread::projected"}"#,
                None,
                Some(ThreadRecordProjections {
                    thread_meta: None,
                    task: None,
                    recent: Some(sample_recent_draft(thread_id)),
                }),
            )
            .expect("write with recent projection");
        let recent = service
            .list_recent_threads(10, 0)
            .expect("list recent")
            .into_iter()
            .find(|row| row.thread_id == thread_id);
        assert!(recent.is_some(), "recent projection row must exist");

        // A rewrite with `recent: None` removes the projection row in the
        // same transaction as the record update.
        service
            .write_thread_record_with_projections(
                thread_id,
                r#"{"thread_id":"thread::projected","hidden":true}"#,
                None,
                Some(ThreadRecordProjections {
                    thread_meta: None,
                    task: None,
                    recent: None,
                }),
            )
            .expect("write removing recent projection");
        let recent = service
            .list_recent_threads(10, 0)
            .expect("list recent")
            .into_iter()
            .find(|row| row.thread_id == thread_id);
        assert!(recent.is_none(), "recent projection row must be removed");
        assert!(
            service.thread_record_exists(thread_id).expect("exists"),
            "record itself survives projection removal"
        );

        // Deleting the record clears every projection row and the pin
        // with it, in the same transaction.
        service
            .write_thread_record_with_projections(
                thread_id,
                r#"{"thread_id":"thread::projected"}"#,
                None,
                Some(ThreadRecordProjections {
                    thread_meta: None,
                    task: None,
                    recent: Some(sample_recent_draft(thread_id)),
                }),
            )
            .expect("write again");
        service.pin_thread(thread_id).expect("pin");
        service
            .delete_thread_record_with_projections(thread_id)
            .expect("delete");
        assert!(
            !service
                .list_recent_threads(10, 0)
                .expect("list recent")
                .iter()
                .any(|row| row.thread_id == thread_id),
            "projection rows must not survive record deletion"
        );
        assert!(
            !service
                .list_pinned_threads()
                .expect("list pins")
                .pins
                .iter()
                .any(|pin| pin.thread_id == thread_id),
            "the pin must be removed in the delete transaction"
        );
    }

    #[test]
    fn thread_record_write_rolls_back_atomically_on_projection_failure() {
        let service = GaryxDbService::memory().expect("memory db");
        let thread_id = "thread::atomic";

        // An invalid projection draft (blank run_state) fails inside the
        // transaction; the record write must roll back with it.
        let mut bad_recent = sample_recent_draft(thread_id);
        bad_recent.run_state = "  ".to_owned();
        let result = service.write_thread_record_with_projections(
            thread_id,
            r#"{"thread_id":"thread::atomic"}"#,
            None,
            Some(ThreadRecordProjections {
                thread_meta: None,
                task: None,
                recent: Some(bad_recent),
            }),
        );
        assert!(result.is_err(), "invalid projection draft must error");
        assert!(
            !service.thread_record_exists(thread_id).expect("exists"),
            "record write must roll back when a projection write fails"
        );
    }

    #[test]
    fn clear_stale_active_runs_settles_by_recent_run_presence() {
        // Review #TASK-1927: an orphan with no committed run must settle to
        // idle (matching the retired reconcile's derivation), while one
        // with history settles to completed.
        let service = GaryxDbService::memory().expect("memory db");
        for (thread_id, recent) in [
            ("thread::orphan-no-history", None),
            ("thread::orphan-with-history", Some("run::done")),
        ] {
            service
                .upsert_recent_thread(RecentThreadDraft {
                    thread_id: thread_id.to_owned(),
                    title: "Orphan".to_owned(),
                    workspace_dir: None,
                    thread_type: "chat".to_owned(),
                    provider_type: None,
                    agent_id: None,
                    message_count: 1,
                    last_message_preview: String::new(),
                    recent_run_id: recent.map(str::to_owned),
                    active_run_id: Some("run::stale".to_owned()),
                    run_state: "running".to_owned(),
                    updated_at: None,
                    last_active_at: "2026-07-08T00:00:00Z".to_owned(),
                })
                .expect("seed row");
        }

        let before = service
            .list_recent_threads(10, 0)
            .unwrap()
            .into_iter()
            .map(|row| (row.thread_id, row.activity_seq))
            .collect::<std::collections::BTreeMap<_, _>>();
        let meta_before: i64 = service
            .conn()
            .unwrap()
            .query_row(
                "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();

        service.clear_stale_active_runs().expect("clear orphans");

        let rows = service.list_recent_threads(10, 0).expect("list");
        assert_eq!(
            rows.iter()
                .map(|row| (row.thread_id.clone(), row.activity_seq))
                .collect::<std::collections::BTreeMap<_, _>>(),
            before,
            "pre-bind orphan settlement must not move rows in activity order"
        );
        assert_eq!(
            service
                .conn()
                .unwrap()
                .query_row(
                    "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            meta_before,
            "pre-bind orphan settlement must not allocate a sequence"
        );
        let state_of = |id: &str| {
            rows.iter()
                .find(|row| row.thread_id == id)
                .map(|row| (row.active_run_id.clone(), row.run_state.clone()))
                .expect("row")
        };
        assert_eq!(
            state_of("thread::orphan-no-history"),
            (None, "idle".to_owned())
        );
        assert_eq!(
            state_of("thread::orphan-with-history"),
            (None, "completed".to_owned())
        );
    }

    #[test]
    fn memory_db_still_works_without_wal() {
        let service = GaryxDbService::memory().expect("memory db");
        service.pin_thread("thread::mem-check").expect("pin");
        let page = service.list_pinned_threads().expect("list");
        assert_eq!(page.pins.len(), 1);
        assert_eq!(page.pins[0].thread_id, "thread::mem-check");
    }

    #[test]
    fn startup_migrations_purge_legacy_workflow_tables_records_and_projections() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        drop(GaryxDbService::open(&path).expect("create current schema"));

        {
            let conn = Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                r#"
                CREATE TABLE workflow_runs (
                    workflow_id TEXT PRIMARY KEY
                );
                CREATE TABLE workflow_child_runs (thread_id TEXT NOT NULL);
                CREATE TABLE workflow_events (event_seq INTEGER PRIMARY KEY);

                INSERT INTO workflow_runs (workflow_id)
                VALUES ('thread::legacy-workflow-run');
                INSERT INTO workflow_child_runs (thread_id)
                VALUES ('thread::legacy-workflow-child');
                INSERT INTO workflow_events (event_seq) VALUES (1);

                INSERT INTO thread_records (key, body, recorded_at) VALUES
                  ('thread::legacy-workflow-run',
                   '{"thread_kind":"workflow_run","workflow_run_id":"thread::legacy-workflow-run"}',
                   '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-task',
                   '{"task":{"executor":{"type":"workflow","workflow_id":"unit"}}}',
                   '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-child',
                   '{"source":"workflow","workflow_child_run_id":"child::legacy"}',
                   '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-metadata',
                   '{"metadata":{"workflow_thread":true,"workflow_run_id":"thread::legacy-workflow-metadata"}}',
                   '2026-07-01T00:00:00.000Z'),
                  ('thread::ordinary',
                   '{"label":"Discuss the ordinary deployment workflow"}',
                   '2026-07-01T00:00:00.000Z');

                INSERT INTO task_projection (
                    thread_id, number, status, title, creator_json, creator_id,
                    updated_by_json, executor_json, created_at, updated_at,
                    source_updated_at, source_events_len, projected_at
                ) VALUES (
                    'thread::legacy-workflow-task', 71, 'done', 'Legacy task',
                    '{"kind":"agent","agent_id":"legacy"}', 'legacy',
                    '{"kind":"agent","agent_id":"legacy"}',
                    '{"type":"workflow","workflow_id":"unit"}',
                    '2026-07-01T00:00:00.000Z', '2026-07-01T00:00:00.000Z',
                    '2026-07-01T00:00:00.000Z', 0, '2026-07-01T00:00:00.000Z'
                );

                INSERT INTO thread_meta (thread_id, thread_type, projected_at) VALUES
                  ('thread::legacy-workflow-run', 'workflow_run', '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-task', 'workflow_run', '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-child', 'chat', '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-metadata', 'chat', '2026-07-01T00:00:00.000Z'),
                  ('thread::ordinary', 'chat', '2026-07-01T00:00:00.000Z');

                INSERT INTO recent_threads (
                    thread_id, title, thread_type, message_count, last_message_preview,
                    run_state, last_active_at, recorded_at
                ) VALUES (
                    'thread::legacy-workflow-run', 'Legacy run', 'workflow_run', 0, '',
                    'idle', '2026-07-01T00:00:00.000Z', '2026-07-01T00:00:00.000Z'
                );
                INSERT INTO thread_pins (thread_id, pinned_at)
                VALUES ('thread::legacy-workflow-task', '2026-07-01T00:00:00.000Z');
                INSERT INTO thread_favorites (thread_id, favorited_at)
                VALUES ('thread::legacy-workflow-task', '2026-07-01T00:00:00.000Z');
                INSERT INTO archived_threads (thread_id, archived_at)
                VALUES ('thread::legacy-workflow-child', '2026-07-01T00:00:00.000Z');
                INSERT INTO thread_channel_endpoints (
                    endpoint_key, channel, account_id, binding_key, thread_id, projected_at
                ) VALUES (
                    'test::main::legacy', 'test', 'main', 'legacy',
                    'thread::legacy-workflow-child', '2026-07-01T00:00:00.000Z'
                );
                INSERT INTO automation_thread_runs (
                    automation_id, run_id, thread_id, mode, status, started_at, recorded_at
                ) VALUES (
                    'automation::legacy', 'run::legacy', 'thread::legacy-workflow-run',
                    'generated_thread', 'done', '2026-07-01T00:00:00.000Z',
                    '2026-07-01T00:00:00.000Z'
                );
                INSERT INTO capsules (
                    id, title, description, thread_id, html_sha256, byte_size,
                    revision, created_at, updated_at
                ) VALUES (
                    'capsule::legacy', 'Legacy capsule', '',
                    'thread::legacy-workflow-child', 'abc123', 1, 1,
                    '2026-07-01T00:00:00.000Z', '2026-07-01T00:00:00.000Z'
                );
                "#,
            )
            .expect("seed legacy workflow state");
        }

        let db = GaryxDbService::open(&path).expect("open migrated db");
        db.run_thread_data_startup_migrations()
            .expect("run destructive startup migrations");
        assert_eq!(
            db.list_pinned_threads()
                .expect("pins after cleanup")
                .revision,
            1,
            "startup cleanup must bump the collection exactly once"
        );
        assert_eq!(
            db.list_thread_favorites()
                .expect("favorites after cleanup")
                .revision,
            1,
            "startup cleanup must bump favorites exactly once when changed"
        );
        for table in ["workflow_runs", "workflow_child_runs", "workflow_events"] {
            assert!(!sqlite_table_exists(&db.conn().expect("conn"), table).expect("table check"));
        }
        for thread_id in [
            "thread::legacy-workflow-run",
            "thread::legacy-workflow-task",
            "thread::legacy-workflow-child",
            "thread::legacy-workflow-metadata",
        ] {
            assert_eq!(
                db.get_thread_record_body(thread_id).expect("record lookup"),
                None,
                "retired record survived: {thread_id}"
            );
        }
        assert!(
            db.get_thread_record_body("thread::ordinary")
                .expect("ordinary record")
                .is_some(),
            "plain-English workflow text must not delete an ordinary thread"
        );

        let conn = db.conn().expect("conn");
        for (table, column) in [
            ("task_projection", "thread_id"),
            ("thread_meta", "thread_id"),
            ("recent_threads", "thread_id"),
            ("thread_pins", "thread_id"),
            ("thread_favorites", "thread_id"),
            ("archived_threads", "thread_id"),
            ("thread_channel_endpoints", "thread_id"),
            ("automation_thread_runs", "thread_id"),
        ] {
            let sql = format!(
                "SELECT COUNT(*) FROM {table} WHERE {column} LIKE 'thread::legacy-workflow%'"
            );
            let count: i64 = conn.query_row(&sql, [], |row| row.get(0)).expect("count");
            assert_eq!(count, 0, "retired projection survived in {table}");
        }
        let capsule_thread_id: Option<String> = conn
            .query_row(
                "SELECT thread_id FROM capsules WHERE id = 'capsule::legacy'",
                [],
                |row| row.get(0),
            )
            .expect("capsule reference");
        assert_eq!(capsule_thread_id, None);
        drop(conn);
        drop(db);

        let reopened = GaryxDbService::open(&path).expect("cleanup is idempotent");
        reopened
            .run_thread_data_startup_migrations()
            .expect("rerun startup migrations");
        assert_eq!(
            reopened
                .list_pinned_threads()
                .expect("pins after idempotent cleanup")
                .revision,
            1,
            "a second startup cleanup must not bump an unchanged collection"
        );
        assert_eq!(
            reopened
                .list_thread_favorites()
                .expect("favorites after idempotent cleanup")
                .revision,
            1,
            "a no-op startup purge must preserve favorites revision"
        );
        assert!(
            reopened
                .get_thread_record_body("thread::ordinary")
                .expect("ordinary record after reopen")
                .is_some()
        );
    }

    #[tokio::test]
    async fn run_blocking_round_trips_reads_and_writes() {
        let service = std::sync::Arc::new(GaryxDbService::memory().expect("memory db"));

        let page = service
            .run_blocking(|db| db.pin_thread("thread::async-entry"))
            .await
            .expect("async pin");
        assert_eq!(page.pins[0].thread_id, "thread::async-entry");

        let page = service
            .run_blocking(|db| db.list_pinned_threads())
            .await
            .expect("async list");
        assert_eq!(page.pins.len(), 1);
        assert_eq!(page.pins[0].thread_id, "thread::async-entry");
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

        let rows = db.list_thread_meta().expect("list legacy meta");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].thread_id, "thread::legacy");
        assert_eq!(rows[0].created_at, None);
        assert_eq!(rows[0].message_count, 0);
        assert_eq!(rows[0].last_message_preview, None);
        assert_eq!(rows[0].projection_version, 2);
    }

    #[test]
    fn opening_composite_endpoint_pk_db_restores_single_holder_upserts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        {
            let conn = Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                r#"
                CREATE TABLE thread_records (
                    key TEXT PRIMARY KEY,
                    body TEXT NOT NULL,
                    updated_at TEXT,
                    recorded_at TEXT NOT NULL
                ) STRICT;

                CREATE TABLE projection_states (
                    projection_name TEXT PRIMARY KEY,
                    projection_version INTEGER NOT NULL,
                    source_row_count INTEGER NOT NULL,
                    projected_at TEXT NOT NULL
                ) STRICT;

                CREATE TABLE thread_channel_endpoints (
                    endpoint_key TEXT NOT NULL,
                    channel TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    binding_key TEXT NOT NULL,
                    chat_id TEXT NOT NULL DEFAULT '',
                    delivery_target_type TEXT NOT NULL DEFAULT 'chat_id',
                    delivery_target_id TEXT NOT NULL DEFAULT '',
                    display_label TEXT NOT NULL DEFAULT '',
                    thread_id TEXT NOT NULL,
                    thread_label TEXT,
                    workspace_dir TEXT,
                    thread_updated_at TEXT,
                    last_inbound_at TEXT,
                    last_delivery_at TEXT,
                    projected_at TEXT NOT NULL,
                    PRIMARY KEY (endpoint_key, thread_id)
                ) STRICT;

                INSERT INTO thread_records (key, body, updated_at, recorded_at)
                VALUES (
                    'thread::legacy-holder',
                    '{"thread_id":"thread::legacy-holder","updated_at":"2026-07-01T00:00:00Z","channel_bindings":[{"channel":"api","account_id":"main","binding_key":"client-1","chat_id":"client-1"}]}',
                    '2026-07-01T00:00:00Z',
                    '2026-07-01T00:00:00Z'
                );

                INSERT INTO projection_states (
                    projection_name, projection_version, source_row_count, projected_at
                ) VALUES (
                    'endpoint_holder_dedup_v1', 1, 1, '2026-07-01T00:00:00Z'
                );

                INSERT INTO thread_channel_endpoints (
                    endpoint_key, channel, account_id, binding_key, chat_id,
                    thread_id, projected_at
                ) VALUES (
                    'api::main::client-1', 'api', 'main', 'client-1', 'client-1',
                    'thread::legacy-holder', '2026-07-01T00:00:00Z'
                );
                "#,
            )
            .expect("legacy composite endpoint schema");
        }

        let db = GaryxDbService::open(&path).expect("open migrated db");
        db.run_thread_data_startup_migrations()
            .expect("run startup migrations");
        let rederived = db
            .list_thread_channel_endpoints()
            .expect("list rederived endpoints");
        assert_eq!(rederived.len(), 1);
        assert_eq!(
            rederived[0].thread_id.as_deref(),
            Some("thread::legacy-holder")
        );

        db.replace_thread_meta_projection(ThreadMetaProjectionDraft {
            thread_id: "thread::current-holder".to_owned(),
            thread_meta: ThreadMetaDraft {
                thread_id: "thread::current-holder".to_owned(),
                thread_type: "chat".to_owned(),
                ..Default::default()
            },
            channel_endpoints: vec![KnownChannelEndpoint {
                endpoint_key: "api::main::client-1".to_owned(),
                channel: "api".to_owned(),
                account_id: "main".to_owned(),
                binding_key: "client-1".to_owned(),
                chat_id: "client-1".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "client-1".to_owned(),
                display_label: "Test Client".to_owned(),
                thread_id: Some("thread::current-holder".to_owned()),
                ..Default::default()
            }],
        })
        .expect("single-holder endpoint upsert");

        let endpoints = db
            .list_thread_channel_endpoints()
            .expect("list migrated endpoints");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(
            endpoints[0].thread_id.as_deref(),
            Some("thread::current-holder")
        );
    }

    #[test]
    fn fresh_thread_pins_schema_has_sort_order_revision_and_zero_row_marker() {
        let db = GaryxDbService::memory().expect("db opens");
        let column = db
            .conn()
            .expect("connection")
            .query_row(
                "SELECT \"notnull\", dflt_value
                   FROM pragma_table_info('thread_pins')
                  WHERE name = 'sort_order'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .expect("sort_order column");
        assert_eq!(column, (1, Some("0".to_owned())));
        assert_eq!(db.list_pinned_threads().expect("fresh page").revision, 0);

        let summary = db
            .migrate_thread_pin_sort_order_v1()
            .expect("zero-row migration");
        assert_eq!(summary.source_row_count, 0);
        assert_eq!(summary.updated_row_count, 0);
        assert!(!summary.already_completed);
        assert!(
            db.projection_state_exists(
                THREAD_PIN_SORT_ORDER_MIGRATION_NAME,
                THREAD_PIN_SORT_ORDER_MIGRATION_VERSION,
            )
            .expect("migration marker")
        );
    }

    #[test]
    fn legacy_thread_pin_backfill_preserves_display_order_and_runs_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        {
            let conn = Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                "CREATE TABLE thread_pins (
                     thread_id TEXT PRIMARY KEY,
                     pinned_at TEXT NOT NULL
                 ) STRICT;
                 INSERT INTO thread_pins (thread_id, pinned_at) VALUES
                   ('thread::oldest', '2026-01-01T00:00:01.000Z'),
                   ('thread::same-b', '2026-01-01T00:00:03.000Z'),
                   ('thread::same-a', '2026-01-01T00:00:03.000Z'),
                   ('thread::middle', '2026-01-01T00:00:02.000Z');",
            )
            .expect("legacy pins");
        }

        let db = GaryxDbService::open(&path).expect("open legacy db");
        let summary = db
            .migrate_thread_pin_sort_order_v1()
            .expect("backfill legacy pins");
        assert_eq!(summary.source_row_count, 4);
        assert_eq!(summary.updated_row_count, 4);
        assert!(!summary.already_completed);
        let page = db.list_pinned_threads().expect("backfilled page");
        assert_eq!(page.revision, 0);
        assert_eq!(
            page.pins
                .iter()
                .map(|pin| (pin.thread_id.as_str(), pin.sort_order))
                .collect::<Vec<_>>(),
            vec![
                ("thread::same-a", 0),
                ("thread::same-b", 1),
                ("thread::middle", 2),
                ("thread::oldest", 3),
            ]
        );

        db.conn()
            .expect("connection")
            .execute(
                "UPDATE thread_pins SET sort_order = 99 WHERE thread_id = 'thread::same-a'",
                [],
            )
            .expect("tamper after marker");
        drop(db);

        let reopened = GaryxDbService::open(&path).expect("second boot");
        let second = reopened
            .migrate_thread_pin_sort_order_v1()
            .expect("migration stays one-shot");
        assert!(second.already_completed);
        assert_eq!(second.updated_row_count, 0);
        let retained: i64 = reopened
            .conn()
            .expect("connection")
            .query_row(
                "SELECT sort_order FROM thread_pins WHERE thread_id = 'thread::same-a'",
                [],
                |row| row.get(0),
            )
            .expect("retained sort order");
        assert_eq!(retained, 99, "the marker must prevent a second backfill");
    }

    #[test]
    fn failed_thread_pin_backfill_rolls_back_and_retries_cleanly() {
        let db = GaryxDbService::memory().expect("db opens");
        db.conn()
            .expect("connection")
            .execute_batch(
                "INSERT INTO thread_pins (thread_id, pinned_at) VALUES
                   ('thread::older', '2026-01-01T00:00:01.000Z'),
                   ('thread::newer', '2026-01-01T00:00:02.000Z');",
            )
            .expect("seed pins");

        let result = db.migrate_thread_pin_sort_order_v1_inner(|_| {
            Err(GaryxDbError::Configuration(
                "injected migration failure".to_owned(),
            ))
        });
        assert!(matches!(result, Err(GaryxDbError::Configuration(_))));
        assert!(
            !db.projection_state_exists(
                THREAD_PIN_SORT_ORDER_MIGRATION_NAME,
                THREAD_PIN_SORT_ORDER_MIGRATION_VERSION,
            )
            .expect("marker lookup")
        );
        let rolled_back = db.list_pinned_threads().expect("rolled-back page");
        assert!(rolled_back.pins.iter().all(|pin| pin.sort_order == 0));

        db.migrate_thread_pin_sort_order_v1()
            .expect("retry migration");
        assert_eq!(
            db.list_pinned_threads()
                .expect("retried page")
                .pins
                .iter()
                .map(|pin| (pin.thread_id.as_str(), pin.sort_order))
                .collect::<Vec<_>>(),
            vec![("thread::newer", 0), ("thread::older", 1)]
        );
    }

    #[test]
    fn thread_pins_page_is_one_wal_snapshot_across_pins_and_revision() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let reader = GaryxDbService::open(&path).expect("reader opens");
        reader.pin_thread("thread::first").expect("first pin");
        // A raw SQLite connection is intentional: the production invariant
        // forbids a second GaryxDbService for the same data dir, while this
        // test still needs a commit between the page's two snapshot reads.
        let writer = Connection::open(&path).expect("test-only raw writer");

        let snapshot = reader
            .list_pinned_threads_inner(|| {
                writer.execute_batch(
                    "BEGIN IMMEDIATE;
                     INSERT INTO thread_pins (thread_id, pinned_at, sort_order)
                     VALUES ('thread::second', '2026-07-16T00:00:00Z', -2);
                     UPDATE thread_pins_meta SET pins_revision = pins_revision + 1 WHERE id = 1;
                     COMMIT;",
                )?;
                Ok(())
            })
            .expect("snapshot page");
        assert_eq!(snapshot.revision, 1);
        assert_eq!(
            snapshot
                .pins
                .iter()
                .map(|pin| pin.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::first"]
        );

        let current = reader.list_pinned_threads().expect("current page");
        assert_eq!(current.revision, 2);
        assert_eq!(
            current
                .pins
                .iter()
                .map(|pin| pin.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::second", "thread::first"]
        );
    }

    #[test]
    fn pin_unpin_and_idempotent_repin_use_atomic_pages_and_exact_revisions() {
        use std::time::Duration;

        let db = GaryxDbService::memory().expect("db opens");
        let first = db.pin_thread("thread::older").expect("pin older");
        assert_eq!(first.revision, 1);
        let first_pin = first.pins[0].clone();
        std::thread::sleep(Duration::from_millis(2));
        let second = db.pin_thread("thread::newer").expect("pin newer");
        assert_eq!(second.revision, 2);
        assert_eq!(
            second
                .pins
                .iter()
                .map(|pin| pin.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::newer", "thread::older"]
        );
        std::thread::sleep(Duration::from_millis(2));
        let repinned = db.pin_thread("thread::older").expect("repin older");
        assert_eq!(repinned.revision, 2);
        let preserved = repinned
            .pins
            .iter()
            .find(|pin| pin.thread_id == "thread::older")
            .expect("repinned record");
        assert_eq!(preserved.pinned_at, first_pin.pinned_at);
        assert_eq!(preserved.sort_order, first_pin.sort_order);

        let (removed, unpinned) = db.unpin_thread("thread::older").expect("unpin older");
        assert!(removed);
        assert_eq!(unpinned.revision, 3);
        assert_eq!(
            unpinned
                .pins
                .iter()
                .map(|pin| pin.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::newer"]
        );
        assert!(
            !db.unpin_thread("thread::older")
                .expect("unpin older again")
                .0
        );
        assert_eq!(db.list_pinned_threads().expect("final page").revision, 3);
    }

    #[test]
    fn thread_favorites_schema_initializes_singleton_before_startup_cleanup_and_reopens_stably() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let db = GaryxDbService::open(&path).expect("database");
        assert_eq!(db.list_thread_favorites().unwrap().revision, 0);
        let conn = db.conn().expect("writer");
        assert!(sqlite_table_exists(&conn, "thread_favorites").unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM thread_favorites_meta WHERE id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
        drop(conn);
        seed_favorite_thread(&db, "thread::reopen-favorite", false);
        let incarnation = db.store_incarnation_id().unwrap();
        db.set_thread_favorite("thread::reopen-favorite", true, 0, &incarnation)
            .expect("favorite");
        drop(db);

        let reopened = GaryxDbService::open(&path).expect("reopen");
        let page = reopened.list_thread_favorites().expect("reopened page");
        assert_eq!(page.revision, 1);
        assert_eq!(page.favorites.len(), 1);
        assert_eq!(page.store_incarnation_id, incarnation);
    }

    #[test]
    fn thread_favorites_cas_fences_identity_revision_and_bumps_every_accepted_noop() {
        let db = GaryxDbService::memory().expect("database");
        seed_favorite_thread(&db, "thread::favorite-cas", false);
        let incarnation = db.store_incarnation_id().unwrap();
        let initial = db.list_thread_favorites().expect("initial page");
        assert_eq!(initial.revision, 0);
        assert!(initial.favorites.is_empty());

        let wrong = db
            .set_thread_favorite("thread::favorite-cas", true, 0, &Uuid::new_v4().to_string())
            .expect("wrong incarnation response");
        assert!(matches!(
            wrong,
            FavoriteThreadResult::WrongIncarnation(ref page) if page.revision == 0
        ));

        let first = db
            .set_thread_favorite("thread::favorite-cas", true, 0, &incarnation)
            .expect("favorite");
        let FavoriteThreadResult::Updated {
            changed: true,
            page: first,
        } = first
        else {
            panic!("expected changed favorite")
        };
        assert_eq!(first.revision, 1);
        assert_eq!(first.favorites.len(), 1);
        let favorited_at = first.favorites[0].favorited_at.clone();

        let repeated = db
            .set_thread_favorite("thread::favorite-cas", true, 1, &incarnation)
            .expect("repeat favorite");
        let FavoriteThreadResult::Updated {
            changed: false,
            page: repeated,
        } = repeated
        else {
            panic!("expected accepted no-op favorite")
        };
        assert_eq!(repeated.revision, 2);
        assert_eq!(repeated.favorites[0].favorited_at, favorited_at);

        let conflict = db
            .set_thread_favorite("thread::favorite-cas", false, 1, &incarnation)
            .expect("stale conflict");
        assert!(matches!(
            conflict,
            FavoriteThreadResult::Conflict(ref page)
                if page.revision == 2 && page.favorites.len() == 1
        ));

        let removed = db
            .set_thread_favorite("thread::favorite-cas", false, 2, &incarnation)
            .expect("unfavorite");
        assert!(matches!(
            removed,
            FavoriteThreadResult::Updated {
                changed: true,
                ref page,
            } if page.revision == 3 && page.favorites.is_empty()
        ));
        let repeated_delete = db
            .set_thread_favorite("thread::favorite-cas", false, 3, &incarnation)
            .expect("repeat unfavorite");
        assert!(matches!(
            repeated_delete,
            FavoriteThreadResult::Updated {
                changed: false,
                ref page,
            } if page.revision == 4 && page.favorites.is_empty()
        ));

        let missing = db
            .set_thread_favorite("thread::missing", true, 4, &incarnation)
            .expect("missing page");
        assert!(matches!(
            missing,
            FavoriteThreadResult::NotFound(ref page)
                if page.revision == 4 && page.favorites.is_empty()
        ));
    }

    #[test]
    fn thread_favorites_get_page_is_one_wal_snapshot() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let db = GaryxDbService::open(&path).expect("database");
        seed_favorite_thread(&db, "thread::first-favorite", false);
        seed_favorite_thread(&db, "thread::second-favorite", false);
        let incarnation = db.store_incarnation_id().unwrap();
        db.set_thread_favorite("thread::first-favorite", true, 0, &incarnation)
            .expect("first favorite");
        let writer = Connection::open(&path).expect("test-only raw writer");

        let snapshot = db
            .list_thread_favorites_inner(|| {
                writer.execute_batch(
                    "BEGIN IMMEDIATE;
                     INSERT INTO thread_favorites (thread_id, favorited_at)
                     VALUES ('thread::second-favorite', '2026-07-16T00:00:01Z');
                     UPDATE thread_favorites_meta
                        SET favorites_revision = favorites_revision + 1 WHERE id = 1;
                     COMMIT;",
                )?;
                Ok(())
            })
            .expect("snapshot page");
        assert_eq!(snapshot.revision, 1);
        assert_eq!(
            snapshot
                .favorites
                .iter()
                .map(|favorite| favorite.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::first-favorite"]
        );
        let current = db.list_thread_favorites().expect("current page");
        assert_eq!(current.revision, 2);
        assert_eq!(current.favorites.len(), 2);
    }

    #[test]
    fn favorites_snapshot_membership_revision_and_recent_rows_share_one_wal_snapshot() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let db = GaryxDbService::open(&path).expect("database");
        seed_favorite_thread(&db, "thread::snapshot-first", true);
        seed_favorite_thread(&db, "thread::snapshot-second", true);
        let incarnation = db.store_incarnation_id().unwrap();
        db.set_thread_favorite("thread::snapshot-first", true, 0, &incarnation)
            .expect("first favorite");
        let writer = Connection::open(&path).expect("test-only raw writer");

        let snapshot = db
            .thread_favorites_snapshot_inner(|| {
                writer.execute_batch(
                    "BEGIN IMMEDIATE;
                     INSERT INTO thread_favorites (thread_id, favorited_at)
                     VALUES ('thread::snapshot-second', '2026-07-16T00:00:01Z');
                     UPDATE thread_favorites_meta
                        SET favorites_revision = favorites_revision + 1 WHERE id = 1;
                     COMMIT;",
                )?;
                Ok(())
            })
            .expect("atomic snapshot");
        assert_eq!(snapshot.page.revision, 1);
        assert_eq!(snapshot.page.favorites.len(), 1);
        assert_eq!(snapshot.recent_total, 1);
        assert_eq!(snapshot.recent_threads.len(), 1);
        assert_eq!(
            snapshot.recent_threads[0].thread_id,
            "thread::snapshot-first"
        );

        let current = db.thread_favorites_snapshot().expect("next snapshot");
        assert_eq!(current.page.revision, 2);
        assert_eq!(current.page.favorites.len(), 2);
        assert_eq!(current.recent_total, 2);
    }

    #[test]
    fn favorites_enhanced_membership_and_summaries_share_one_wal_snapshot() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let db = GaryxDbService::open(&path).expect("database");
        {
            let mut conn = db.conn().expect("writer");
            let tx = conn.transaction().expect("seed transaction");
            seed_summary_favorite_tx(
                &tx,
                "thread::enhanced-first",
                "2026-07-17T00:00:00Z",
                false,
                true,
            );
            seed_summary_favorite_tx(
                &tx,
                "thread::enhanced-second",
                "2026-07-17T00:00:01Z",
                false,
                true,
            );
            tx.execute(
                "DELETE FROM thread_favorites WHERE thread_id = 'thread::enhanced-second'",
                [],
            )
            .unwrap();
            tx.commit().expect("seed commit");
        }
        let writer = Connection::open(&path).expect("test-only raw writer");

        let (snapshot, summaries) = db
            .thread_favorites_snapshot_with_options(true, || {
                writer.execute(
                    "INSERT INTO thread_favorites (thread_id, favorited_at)
                     VALUES ('thread::enhanced-second', '2026-07-17T00:00:01Z')",
                    [],
                )?;
                Ok(())
            })
            .expect("atomic enhanced snapshot");
        let (summaries, truncated) = summaries.expect("summary payload");
        assert_eq!(snapshot.page.favorites.len(), 1);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].thread_id, "thread::enhanced-first");
        assert!(!truncated);

        let current = db
            .thread_favorites_snapshot_with_summaries()
            .expect("next enhanced snapshot");
        assert_eq!(current.snapshot.page.favorites.len(), 2);
        assert_eq!(current.summaries.len(), 2);
    }

    #[test]
    fn favorite_cleanup_interleavings_and_unconditional_plain_delete_bump_close_orphan_writes() {
        let archived = GaryxDbService::memory().expect("archive database");
        seed_favorite_thread(&archived, "thread::archive-first", false);
        let incarnation = archived.store_incarnation_id().unwrap();
        assert!(
            archived
                .archive_thread_record("thread::archive-first")
                .expect("archive first")
        );
        assert!(matches!(
            archived
                .set_thread_favorite("thread::archive-first", true, 0, &incarnation)
                .expect("post-archive write"),
            FavoriteThreadResult::NotFound(ref page) if page.revision == 0
        ));

        let favorite_first = GaryxDbService::memory().expect("favorite-first database");
        seed_favorite_thread(&favorite_first, "thread::favorite-first", false);
        let incarnation = favorite_first.store_incarnation_id().unwrap();
        favorite_first
            .set_thread_favorite("thread::favorite-first", true, 0, &incarnation)
            .expect("favorite first");
        favorite_first
            .archive_thread_record("thread::favorite-first")
            .expect("archive cleans favorite");
        assert_eq!(
            favorite_first
                .list_thread_favorites()
                .expect("post archive")
                .revision,
            2
        );
        favorite_first
            .archive_thread_record("thread::favorite-first")
            .expect("repeat archive");
        assert_eq!(
            favorite_first
                .list_thread_favorites()
                .expect("repeat archive")
                .revision,
            2,
            "archive tombstone permits bump-on-change"
        );

        let deleted = GaryxDbService::memory().expect("delete database");
        seed_favorite_thread(&deleted, "thread::delete-recreate", false);
        let incarnation = deleted.store_incarnation_id().unwrap();
        assert!(
            deleted
                .delete_thread_record_with_projections("thread::delete-recreate")
                .expect("plain delete without favorite")
        );
        assert_eq!(deleted.list_thread_favorites().unwrap().revision, 1);
        seed_favorite_thread(&deleted, "thread::delete-recreate", false);
        assert!(matches!(
            deleted
                .set_thread_favorite("thread::delete-recreate", true, 0, &incarnation)
                .expect("orphan pre-delete write"),
            FavoriteThreadResult::Conflict(ref page) if page.revision == 1
        ));
        assert!(
            deleted
                .list_thread_favorites()
                .unwrap()
                .favorites
                .is_empty()
        );

        deleted
            .set_thread_favorite("thread::delete-recreate", true, 1, &incarnation)
            .expect("fresh favorite");
        assert!(
            deleted
                .delete_thread_record_with_projections("thread::delete-recreate")
                .expect("delete with favorite")
        );
        let page = deleted.list_thread_favorites().unwrap();
        assert_eq!(page.revision, 3, "plain delete bumps exactly once");
        assert!(page.favorites.is_empty());
    }

    #[test]
    fn favorites_snapshot_is_atomic_empty_and_capped_with_truncation() {
        let db = GaryxDbService::memory().expect("database");
        let empty = db.thread_favorites_snapshot().expect("empty snapshot");
        assert!(empty.page.favorites.is_empty());
        assert!(empty.recent_threads.is_empty());
        assert_eq!(empty.recent_total, 0);
        assert!(!empty.recent_truncated);

        let conn = db.conn().expect("writer");
        conn.execute_batch(
            "WITH RECURSIVE seq(x) AS (
                 VALUES(0) UNION ALL SELECT x + 1 FROM seq WHERE x < 500
             )
             INSERT INTO thread_records (key, body, updated_at, recorded_at)
             SELECT printf('thread::snapshot-%03d', x), '{}', NULL,
                    '2026-07-16T00:00:00Z' FROM seq;
             WITH RECURSIVE seq(x) AS (
                 VALUES(0) UNION ALL SELECT x + 1 FROM seq WHERE x < 500
             )
             INSERT INTO thread_favorites (thread_id, favorited_at)
             SELECT printf('thread::snapshot-%03d', x),
                    printf('2026-07-16T00:%02d:%02dZ', x / 60, x % 60) FROM seq;
             WITH RECURSIVE seq(x) AS (
                 VALUES(0) UNION ALL SELECT x + 1 FROM seq WHERE x < 500
             )
             INSERT INTO recent_threads (
                 thread_id, title, thread_type, message_count,
                 last_message_preview, run_state, last_active_at, recorded_at
             )
             SELECT printf('thread::snapshot-%03d', x), printf('Favorite %03d', x),
                    'chat', 1, '', 'idle',
                    printf('2026-07-16T00:%02d:%02dZ', x / 60, x % 60),
                    '2026-07-16T00:00:00Z' FROM seq;",
        )
        .expect("seed 501 joined favorites");
        drop(conn);

        let snapshot = db.thread_favorites_snapshot().expect("capped snapshot");
        assert_eq!(snapshot.page.favorites.len(), 501);
        assert_eq!(snapshot.recent_total, 501);
        assert_eq!(snapshot.recent_threads.len(), 500);
        assert!(snapshot.recent_truncated);
    }

    #[test]
    fn favorites_summary_window_caps_501_all_excluded_members() {
        let db = GaryxDbService::memory().expect("database");
        {
            let mut conn = db.conn().expect("writer");
            let tx = conn.transaction().expect("seed transaction");
            for index in 0..=500 {
                let thread_id = format!("thread::excluded-{index:03}");
                seed_summary_favorite_tx(&tx, &thread_id, &format!("{index:03}"), false, true);
            }
            tx.commit().expect("seed commit");
        }

        let enhanced = db
            .thread_favorites_snapshot_with_summaries()
            .expect("enhanced snapshot");
        assert_eq!(enhanced.snapshot.page.favorites.len(), 501);
        assert_eq!(enhanced.snapshot.recent_total, 0);
        assert!(!enhanced.snapshot.recent_truncated);
        assert!(enhanced.summaries_truncated);
        assert_eq!(enhanced.summaries.len(), 500);
        assert_eq!(
            enhanced.summaries.first().unwrap().thread_id,
            "thread::excluded-500"
        );
        assert!(
            enhanced
                .summaries
                .iter()
                .all(|row| row.thread_id != "thread::excluded-000")
        );
    }

    #[test]
    fn favorites_summary_window_appends_only_one_of_two_raw_members_after_499_recent() {
        let db = GaryxDbService::memory().expect("database");
        {
            let mut conn = db.conn().expect("writer");
            let tx = conn.transaction().expect("seed transaction");
            for index in 0..499 {
                let thread_id = format!("thread::recent-{index:03}");
                seed_summary_favorite_tx(&tx, &thread_id, "recent", false, false);
                seed_summary_recent_tx(&tx, &thread_id, i64::from(index) + 1);
            }
            seed_summary_favorite_tx(
                &tx,
                "thread::raw-newer",
                "2026-07-17T00:00:01.000Z",
                false,
                true,
            );
            seed_summary_favorite_tx(
                &tx,
                "thread::raw-older",
                "2026-07-17T00:00:00.000Z",
                false,
                true,
            );
            tx.commit().expect("seed commit");
        }

        let enhanced = db
            .thread_favorites_snapshot_with_summaries()
            .expect("enhanced snapshot");
        assert_eq!(enhanced.snapshot.recent_total, 499);
        assert!(enhanced.summaries_truncated);
        assert_eq!(enhanced.summaries.len(), 500);
        assert_eq!(
            enhanced.summaries.last().unwrap().thread_id,
            "thread::raw-newer"
        );
        assert!(
            enhanced
                .summaries
                .iter()
                .all(|row| row.thread_id != "thread::raw-older")
        );
    }

    #[test]
    fn favorites_hidden_member_occupies_a_summary_window_slot() {
        let db = GaryxDbService::memory().expect("database");
        {
            let mut conn = db.conn().expect("writer");
            let tx = conn.transaction().expect("seed transaction");
            for index in 0..=500 {
                let thread_id = format!("thread::hidden-window-{index:03}");
                seed_summary_favorite_tx(
                    &tx,
                    &thread_id,
                    &format!("{index:03}"),
                    index == 500,
                    true,
                );
            }
            tx.commit().expect("seed commit");
        }

        let enhanced = db
            .thread_favorites_snapshot_with_summaries()
            .expect("enhanced snapshot");
        assert!(enhanced.summaries_truncated);
        assert_eq!(
            enhanced.summaries.len(),
            499,
            "the hidden member consumes one of the 500 window positions"
        );
        assert!(
            enhanced
                .summaries
                .iter()
                .all(|row| row.thread_id != "thread::hidden-window-500")
        );
        assert!(
            enhanced
                .summaries
                .iter()
                .all(|row| row.thread_id != "thread::hidden-window-000"),
            "the first visible member beyond the window must not leak in"
        );
    }

    #[test]
    fn favorites_raw_same_millisecond_tiebreak_selects_ascending_id_at_position_500() {
        let db = GaryxDbService::memory().expect("database");
        {
            let mut conn = db.conn().expect("writer");
            let tx = conn.transaction().expect("seed transaction");
            for index in 0..499 {
                let thread_id = format!("thread::tie-recent-{index:03}");
                seed_summary_favorite_tx(&tx, &thread_id, "recent", false, false);
                seed_summary_recent_tx(&tx, &thread_id, i64::from(index) + 1);
            }
            // Reverse insertion is deliberate: ordering must come from the
            // raw fallback contract, not rowid/insertion order.
            seed_summary_favorite_tx(
                &tx,
                "thread::raw-z",
                "2026-07-17T00:00:00.123Z",
                false,
                true,
            );
            seed_summary_favorite_tx(
                &tx,
                "thread::raw-a",
                "2026-07-17T00:00:00.123Z",
                false,
                true,
            );
            tx.commit().expect("seed commit");
        }

        let enhanced = db
            .thread_favorites_snapshot_with_summaries()
            .expect("enhanced snapshot");
        assert!(enhanced.summaries_truncated);
        assert_eq!(enhanced.summaries.len(), 500);
        assert_eq!(
            enhanced.summaries.last().unwrap().thread_id,
            "thread::raw-a"
        );
        assert!(
            enhanced
                .summaries
                .iter()
                .all(|row| row.thread_id != "thread::raw-z")
        );
    }

    #[test]
    fn reorder_thread_pins_handles_full_subset_unknown_and_stale_requests() {
        let db = GaryxDbService::memory().expect("db opens");
        db.pin_thread("thread::a").expect("pin a");
        db.pin_thread("thread::b").expect("pin b");
        let initial = db.pin_thread("thread::c").expect("pin c");
        assert_eq!(initial.revision, 3);
        let original_metadata = initial
            .pins
            .iter()
            .map(|pin| (pin.thread_id.clone(), pin.pinned_at.clone()))
            .collect::<BTreeSet<_>>();

        let full = match db
            .reorder_thread_pins(
                vec![
                    "thread::a".to_owned(),
                    "thread::c".to_owned(),
                    "thread::b".to_owned(),
                ],
                3,
            )
            .expect("full reorder")
        {
            ReorderThreadPinsResult::Updated(page) => page,
            ReorderThreadPinsResult::Conflict(_) => panic!("fresh CAS conflicted"),
        };
        assert_eq!(full.revision, 4);
        assert_eq!(
            full.pins
                .iter()
                .map(|pin| (pin.thread_id.as_str(), pin.sort_order))
                .collect::<Vec<_>>(),
            vec![("thread::a", 0), ("thread::c", 1), ("thread::b", 2)]
        );

        let subset = match db
            .reorder_thread_pins(vec!["thread::b".to_owned()], 4)
            .expect("subset reorder")
        {
            ReorderThreadPinsResult::Updated(page) => page,
            ReorderThreadPinsResult::Conflict(_) => panic!("fresh CAS conflicted"),
        };
        assert_eq!(subset.revision, 5);
        assert_eq!(
            subset
                .pins
                .iter()
                .map(|pin| pin.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::b", "thread::a", "thread::c"]
        );

        let unknown = match db
            .reorder_thread_pins(
                vec!["thread::unknown".to_owned(), "thread::c".to_owned()],
                5,
            )
            .expect("unknown-id reorder")
        {
            ReorderThreadPinsResult::Updated(page) => page,
            ReorderThreadPinsResult::Conflict(_) => panic!("fresh CAS conflicted"),
        };
        assert_eq!(unknown.revision, 6);
        assert_eq!(
            unknown
                .pins
                .iter()
                .map(|pin| pin.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::c", "thread::b", "thread::a"]
        );
        assert_eq!(
            unknown
                .pins
                .iter()
                .map(|pin| (pin.thread_id.clone(), pin.pinned_at.clone()))
                .collect::<BTreeSet<_>>(),
            original_metadata,
            "reorder must preserve membership and pin metadata"
        );

        let conflict = match db
            .reorder_thread_pins(vec!["thread::a".to_owned()], 5)
            .expect("stale reorder")
        {
            ReorderThreadPinsResult::Conflict(page) => page,
            ReorderThreadPinsResult::Updated(_) => panic!("stale CAS unexpectedly succeeded"),
        };
        assert_eq!(conflict, unknown);
        assert_eq!(db.list_pinned_threads().expect("GET page"), unknown);

        assert!(matches!(
            db.reorder_thread_pins(Vec::new(), 6),
            Err(GaryxDbError::BadRequest(_))
        ));
        assert!(matches!(
            db.reorder_thread_pins(vec!["thread::a".to_owned(), " thread::a ".to_owned()], 6,),
            Err(GaryxDbError::BadRequest(_))
        ));
        assert_eq!(
            db.list_pinned_threads().expect("unchanged page").revision,
            6
        );
    }

    #[test]
    fn archive_and_runtime_delete_each_bump_pin_revision_once() {
        let archived = GaryxDbService::memory().expect("archive db");
        archived
            .pin_thread("thread::archived")
            .expect("archive candidate pin");
        archived
            .archive_thread_record("thread::archived")
            .expect("archive");
        assert_eq!(
            archived
                .list_pinned_threads()
                .expect("archive page")
                .revision,
            2
        );
        archived
            .archive_thread_record("thread::archived")
            .expect("repeat archive");
        assert_eq!(
            archived
                .list_pinned_threads()
                .expect("repeat archive page")
                .revision,
            2
        );

        let deleted = GaryxDbService::memory().expect("delete db");
        deleted
            .pin_thread("thread::deleted")
            .expect("delete candidate pin");
        deleted
            .delete_thread_record_with_projections("thread::deleted")
            .expect("runtime delete");
        assert_eq!(
            deleted.list_pinned_threads().expect("delete page").revision,
            2
        );
        deleted
            .delete_thread_record_with_projections("thread::deleted")
            .expect("repeat delete");
        assert_eq!(
            deleted
                .list_pinned_threads()
                .expect("repeat delete page")
                .revision,
            2
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

    fn capsule_table_columns(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("PRAGMA table_info(capsules)")
            .expect("inspect capsules schema");
        stmt.query_map([], |row| row.get::<_, String>(1))
            .expect("query capsules columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("read capsules columns")
    }

    #[test]
    fn capsules_schema_has_favorite_column_and_reinitialization_is_idempotent() {
        let db = GaryxDbService::memory().expect("db opens");
        let conn = db.conn().expect("db connection");
        assert!(capsule_table_columns(&conn).contains(&"favorited_at".to_owned()));

        initialize_connection(&conn).expect("schema reinitializes");
        let columns = capsule_table_columns(&conn);
        assert_eq!(
            columns
                .iter()
                .filter(|column| column.as_str() == "favorited_at")
                .count(),
            1
        );
    }

    #[test]
    fn capsules_schema_migrates_existing_table_with_favorite_column() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let legacy = Connection::open(&path).expect("open legacy db");
        legacy
            .execute_batch(
                r#"
                CREATE TABLE capsules (
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
                "#,
            )
            .expect("create legacy capsules table");
        drop(legacy);

        let db = GaryxDbService::open(&path).expect("open migrated db");
        let conn = db.conn().expect("db connection");
        assert!(capsule_table_columns(&conn).contains(&"favorited_at".to_owned()));
        initialize_connection(&conn).expect("migrated schema reinitializes");
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
        assert_eq!(created.favorited_at, None);

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
    fn set_capsule_favorite_is_idempotent_metadata_only_point_write() {
        let db = GaryxDbService::memory().expect("db opens");
        let id = Uuid::new_v4().to_string();
        let created = db
            .create_capsule(capsule_draft(&id, "Favorite", "thread::capsules"))
            .expect("create capsule");

        let favorited = db
            .set_capsule_favorite(&id, true)
            .expect("favorite capsule")
            .expect("capsule exists");
        let first_favorited_at = favorited
            .favorited_at
            .clone()
            .expect("favorite timestamp is set");
        assert_eq!(favorited.revision, created.revision);
        assert_eq!(favorited.updated_at, created.updated_at);

        let repeated = db
            .set_capsule_favorite(&id, true)
            .expect("repeat favorite")
            .expect("capsule exists");
        assert_eq!(
            repeated.favorited_at.as_deref(),
            Some(first_favorited_at.as_str())
        );
        assert_eq!(repeated.revision, created.revision);
        assert_eq!(repeated.updated_at, created.updated_at);

        let unfavorited = db
            .set_capsule_favorite(&id, false)
            .expect("unfavorite capsule")
            .expect("capsule exists");
        assert_eq!(unfavorited.favorited_at, None);
        assert_eq!(unfavorited.revision, created.revision);
        assert_eq!(unfavorited.updated_at, created.updated_at);

        let unknown_id = Uuid::new_v4().to_string();
        assert_eq!(
            db.set_capsule_favorite(&unknown_id, true)
                .expect("favorite unknown capsule"),
            None
        );
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
    fn recent_threads_filtered_page_filters_before_pagination() {
        let db = GaryxDbService::memory().expect("db opens");
        for (thread_id, thread_type, timestamp) in [
            ("thread::task-middle", "task", "2026-05-23T12:00:00Z"),
            ("thread::chat-older", "chat", "2026-05-23T13:00:00Z"),
            ("thread::chat-newer", "chat", "2026-05-23T13:00:00Z"),
            ("thread::task-newest", "task", "2026-05-23T14:00:00Z"),
        ] {
            db.upsert_recent_thread(RecentThreadDraft {
                thread_id: thread_id.to_owned(),
                title: thread_id.to_owned(),
                workspace_dir: None,
                thread_type: thread_type.to_owned(),
                provider_type: None,
                agent_id: None,
                message_count: 0,
                last_message_preview: String::new(),
                recent_run_id: None,
                active_run_id: None,
                run_state: "idle".to_owned(),
                updated_at: Some(timestamp.to_owned()),
                last_active_at: timestamp.to_owned(),
            })
            .expect("seed recent row");
        }

        let excluded = db
            .list_recent_threads_page(RecentThreadTaskFilter::Exclude, 2, 0)
            .expect("exclude page");
        assert_eq!(excluded.total, 2);
        assert_eq!(excluded.offset, 0);
        assert!(!excluded.has_more);
        assert_eq!(
            excluded
                .records
                .iter()
                .map(|row| row.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::chat-newer", "thread::chat-older"],
            "task rows ahead of chats must not shorten the filtered page"
        );

        let only_first = db
            .list_recent_threads_page(RecentThreadTaskFilter::Only, 1, 0)
            .expect("only first page");
        assert_eq!(only_first.total, 2);
        assert!(only_first.has_more);
        assert_eq!(only_first.records[0].thread_id, "thread::task-newest");
        let only_second = db
            .list_recent_threads_page(RecentThreadTaskFilter::Only, 1, 1)
            .expect("only second page");
        assert_eq!(only_second.offset, 1);
        assert!(!only_second.has_more);
        assert_eq!(only_second.records[0].thread_id, "thread::task-middle");

        let included = db
            .list_recent_threads_page(RecentThreadTaskFilter::Include, 10, 0)
            .expect("include page");
        assert_eq!(included.total, 4);
        assert_eq!(
            included
                .records
                .iter()
                .map(|row| row.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "thread::task-newest",
                "thread::chat-newer",
                "thread::chat-older",
                "thread::task-middle",
            ]
        );

        let clamped = db
            .list_recent_threads_page(RecentThreadTaskFilter::Exclude, 10, 99)
            .expect("clamped page");
        assert_eq!(clamped.total, 2);
        assert_eq!(clamped.offset, 2);
        assert!(clamped.records.is_empty());
        assert!(!clamped.has_more);
    }

    #[test]
    fn recent_threads_keyset_does_not_skip_after_deletion_and_uses_n_plus_one() {
        let db = GaryxDbService::memory().expect("db opens");
        for thread_id in ["thread::oldest", "thread::middle", "thread::newest"] {
            db.upsert_recent_thread(sample_recent_draft(thread_id))
                .expect("seed recent row");
        }

        let first = db
            .list_recent_threads_keyset_page(RecentThreadTaskFilter::Include, 1, None)
            .expect("first keyset page");
        assert_eq!(first.total, 3);
        assert!(first.has_more, "N+1 must detect a second row");
        assert_eq!(first.records.len(), 1);
        assert_eq!(first.records[0].thread_id, "thread::newest");
        let cursor = first.records[0].activity_seq;

        db.remove_recent_thread("thread::newest")
            .expect("delete already-returned row");
        let second = db
            .list_recent_threads_keyset_page(RecentThreadTaskFilter::Include, 1, Some(cursor))
            .expect("second keyset page");
        assert_eq!(second.total, 2);
        assert!(second.has_more);
        assert_eq!(
            second.records[0].thread_id, "thread::middle",
            "deleting a row above the cursor must not skip the next row"
        );

        let last = db
            .list_recent_threads_keyset_page(
                RecentThreadTaskFilter::Include,
                1,
                Some(second.records[0].activity_seq),
            )
            .expect("last keyset page");
        assert_eq!(last.records[0].thread_id, "thread::oldest");
        assert!(!last.has_more, "exactly N remaining rows has no next page");

        let empty = db
            .list_recent_threads_keyset_page(
                RecentThreadTaskFilter::Include,
                1,
                Some(last.records[0].activity_seq),
            )
            .expect("empty tail page");
        assert!(empty.records.is_empty());
        assert!(!empty.has_more);
    }

    #[test]
    fn recent_threads_keyset_count_and_rows_share_one_wal_snapshot() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let db = GaryxDbService::open(&path).expect("db opens");
        db.upsert_recent_thread(sample_recent_draft("thread::snapshot-before"))
            .expect("seed initial row");

        let page = db
            .list_recent_threads_keyset_page_inner(
                RecentThreadTaskFilter::Include,
                10,
                None,
                || {
                    let writer = Connection::open(&path)?;
                    writer.execute_batch(
                        "BEGIN IMMEDIATE;
                         UPDATE recent_threads_meta SET activity_seq = 2 WHERE id = 1;
                         INSERT INTO recent_threads (
                             thread_id, title, thread_type, last_active_at,
                             activity_seq, recorded_at
                         ) VALUES (
                             'thread::snapshot-after', 'After', 'chat',
                             '2026-07-16T01:00:00Z', 2,
                             '2026-07-16T01:00:00Z'
                         );
                         COMMIT;",
                    )?;
                    Ok(())
                },
            )
            .expect("snapshot keyset page");

        assert_eq!(page.total, 1);
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].thread_id, "thread::snapshot-before");
        assert_eq!(db.count_recent_threads().unwrap(), 2);
    }

    #[test]
    fn recent_threads_filtered_page_uses_one_read_snapshot() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garyx-db.sqlite3");
        let db = GaryxDbService::open(&path).expect("db opens");
        db.upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::snapshot-before".to_owned(),
            title: "Before".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 0,
            last_message_preview: String::new(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-05-23T10:00:00Z".to_owned()),
            last_active_at: "2026-05-23T10:00:00Z".to_owned(),
        })
        .expect("seed initial row");

        let page = db
            .list_recent_threads_page_inner(RecentThreadTaskFilter::Include, 10, 0, || {
                let writer = Connection::open(&path)?;
                writer.execute(
                    "INSERT INTO recent_threads (
                            thread_id, title, thread_type, last_active_at, recorded_at
                         ) VALUES (
                            'thread::snapshot-after', 'After', 'chat',
                            '2026-05-23T11:00:00Z', '2026-05-23T11:00:00Z'
                         )",
                    [],
                )?;
                Ok(())
            })
            .expect("snapshot page");

        assert_eq!(page.total, 1);
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].thread_id, "thread::snapshot-before");
        assert_eq!(
            db.count_recent_threads().expect("post-write count"),
            2,
            "the concurrent commit must exist after the read transaction closes"
        );
    }

    #[test]
    fn recent_threads_filtered_queries_use_partial_order_indexes() {
        let db = GaryxDbService::memory().expect("db opens");
        db.migrate_recent_thread_activity_seq_v1()
            .expect("create activity indexes");
        let conn = db.conn().expect("conn");
        for (predicate, expected_index) in [
            (
                "thread_type = 'task'",
                "idx_recent_threads_task_activity_seq",
            ),
            (
                "thread_type <> 'task'",
                "idx_recent_threads_non_task_activity_seq",
            ),
        ] {
            let sql = format!(
                "EXPLAIN QUERY PLAN
                 SELECT thread_id FROM recent_threads
                  WHERE {predicate}
                  ORDER BY activity_seq DESC
                  LIMIT 10"
            );
            let mut stmt = conn.prepare(&sql).expect("prepare query plan");
            let details = stmt
                .query_map([], |row| row.get::<_, String>(3))
                .expect("query plan")
                .collect::<Result<Vec<_>, _>>()
                .expect("plan rows")
                .join("\n");
            assert!(
                details.contains(expected_index),
                "expected {expected_index} in query plan:\n{details}"
            );
        }
    }

    #[test]
    fn thread_summary_keyset_branches_use_scoped_partial_indexes_without_temp_sort() {
        let db = GaryxDbService::memory().expect("db opens");
        let conn = db.conn().expect("connection");
        for (filter, suffix) in [
            (ThreadSummaryTaskFilter::Include, "visible"),
            (ThreadSummaryTaskFilter::Exclude, "non_task"),
            (ThreadSummaryTaskFilter::Only, "task"),
        ] {
            for scoped in [false, true] {
                for has_cursor in [false, true] {
                    let expected_index = if scoped {
                        format!("idx_thread_meta_summary_workspace_{suffix}")
                    } else {
                        format!("idx_thread_meta_summary_{suffix}")
                    };
                    let mut bind = Vec::new();
                    if scoped {
                        bind.push(SqlValue::Text("/workspace/test".to_owned()));
                    }
                    if has_cursor {
                        bind.push(SqlValue::Integer(1));
                        bind.push(SqlValue::Text("thread::cursor".to_owned()));
                    }
                    bind.push(SqlValue::Integer(31));
                    let sql = format!(
                        "EXPLAIN QUERY PLAN {}",
                        filter.page_sql(scoped, false, has_cursor)
                    );
                    let mut stmt = conn.prepare(&sql).expect("prepare query plan");
                    let details = stmt
                        .query_map(params_from_iter(bind.iter()), |row| row.get::<_, String>(3))
                        .expect("query plan")
                        .collect::<Result<Vec<_>, _>>()
                        .expect("plan rows")
                        .join("\n");
                    assert!(
                        details.contains("USING INDEX") && details.contains(&expected_index),
                        "expected {expected_index} for filter={filter:?} scoped={scoped} cursor={has_cursor}:\n{details}"
                    );
                    assert!(
                        !details.contains("USE TEMP B-TREE"),
                        "keyset branch must be index-ordered:\n{details}"
                    );
                }
            }
        }
    }

    #[test]
    fn thread_meta_summary_cutover_backfills_all_columns_once_and_is_idempotent() {
        let db = GaryxDbService::memory().expect("db opens");
        let records = [
            (
                "thread::summary-cutover-updated",
                json!({
                    "thread_id": "thread::summary-cutover-updated",
                    "label": "Straße",
                    "workspace_dir": "/workspace/Équipe",
                    "agent_id": "Σς",
                    "updated_at": "2026-07-17T01:02:03.500+00:00",
                    "created_at": "2020-01-01T00:00:00Z",
                    "last_assistant_preview": "％＿＼",
                    "exclude_from_recent": "yes"
                }),
            ),
            (
                "thread::summary-cutover-created",
                json!({
                    "thread_id": "thread::summary-cutover-created",
                    "label": "Created only",
                    "created_at": "2026-07-17T01:02:03Z"
                }),
            ),
            (
                "thread::summary-cutover-null",
                json!({"thread_id": "thread::summary-cutover-null"}),
            ),
        ];
        {
            let conn = db.conn().expect("writer");
            for (thread_id, body) in &records {
                conn.execute(
                    "INSERT INTO thread_records (key, body, updated_at, recorded_at)
                     VALUES (?1, ?2, NULL, '2026-07-17T00:00:00Z')",
                    params![thread_id, body.to_string()],
                )
                .expect("seed canonical record");
                conn.execute(
                    "INSERT INTO thread_meta (
                        thread_id, thread_label, excluded_from_recent,
                        sort_updated_at_us, search_text, projected_at
                     ) VALUES (?1, 'stale', 0, -1, 'stale', '2026-07-17T00:00:00Z')",
                    params![thread_id],
                )
                .expect("seed stale projection");
            }
        }

        let first = db
            .migrate_thread_meta_summary_v1()
            .expect("summary cutover");
        assert_eq!(first.source_row_count, 3);
        assert_eq!(first.updated_row_count, 3);
        assert!(!first.already_completed);
        let rows = db.list_thread_meta().expect("backfilled rows");
        let updated = rows
            .iter()
            .find(|row| row.thread_id == "thread::summary-cutover-updated")
            .unwrap();
        assert!(!updated.excluded_from_recent);
        assert_eq!(
            updated.sort_updated_at_us,
            DateTime::parse_from_rfc3339("2026-07-17T01:02:03.500Z")
                .unwrap()
                .timestamp_micros()
        );
        assert_eq!(
            updated.search_text,
            crate::thread_meta_projection::normalize_for_search(
                "Straße\n/workspace/Équipe\nΣς\n％＿＼",
            )
        );
        let created = rows
            .iter()
            .find(|row| row.thread_id == "thread::summary-cutover-created")
            .unwrap();
        assert_eq!(
            created.sort_updated_at_us,
            DateTime::parse_from_rfc3339("2026-07-17T01:02:03Z")
                .unwrap()
                .timestamp_micros()
        );
        let missing = rows
            .iter()
            .find(|row| row.thread_id == "thread::summary-cutover-null")
            .unwrap();
        assert_eq!(missing.sort_updated_at_us, 0);
        assert!(!missing.excluded_from_recent);

        let second = db
            .migrate_thread_meta_summary_v1()
            .expect("idempotent summary cutover");
        assert_eq!(second.source_row_count, 3);
        assert_eq!(second.updated_row_count, 0);
        assert!(second.already_completed);
        assert_eq!(db.list_thread_meta().unwrap(), rows);
    }

    #[test]
    fn thread_meta_projection_round_trip_and_remove() {
        let db = GaryxDbService::memory().expect("db opens");
        let delivery_json = r#"{"channel":"telegram","account_id":"main","chat_id":"42","user_id":"42","delivery_target_type":"chat_id","delivery_target_id":"42"}"#.to_owned();
        db.replace_thread_meta_projection(ThreadMetaProjectionDraft {
            thread_id: "thread::project".to_owned(),
            thread_meta: ThreadMetaDraft {
                thread_id: "thread::project".to_owned(),
                workspace_dir: Some("/work/project".to_owned()),
                thread_type: "chat".to_owned(),
                thread_label: Some("Project Thread".to_owned()),
                agent_id: Some("codex".to_owned()),
                provider_type: Some("codex".to_owned()),
                provider_key: None,
                selected_model: None,
                selected_model_reasoning_effort: None,
                selected_model_service_tier: None,
                sdk_session_id: None,
                created_at: Some("2026-06-03T07:59:00.000Z".to_owned()),
                updated_at: Some("2026-06-03T08:00:00.000Z".to_owned()),
                message_count: 2,
                last_user_message: Some("start review".to_owned()),
                last_assistant_message: Some("done".to_owned()),
                last_message_preview: Some("done".to_owned()),
                recent_run_id: Some("run::project".to_owned()),
                active_run_id: None,
                worktree_json: Some(r#"{"path":"/work/project"}"#.to_owned()),
                last_delivery_context_json: Some(delivery_json.clone()),
                last_delivery_updated_at: Some("2026-06-03T08:00:01.000Z".to_owned()),
                default_list_hidden: false,
                excluded_from_recent: false,
                sort_updated_at_us: 1_780_473_600_000_000,
                search_text: "project thread\n/work/project\ncodex\ndone".to_owned(),
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
                thread_id: Some("thread::project".to_owned()),
                thread_label: Some("Project Thread".to_owned()),
                workspace_dir: Some("/work/project".to_owned()),
                thread_updated_at: Some("2026-06-03T08:00:00.000Z".to_owned()),
                last_inbound_at: Some("2026-06-03T07:59:59.000Z".to_owned()),
                last_delivery_at: Some("2026-06-03T08:00:01.000Z".to_owned()),
            }],
        })
        .expect("project thread meta");

        let meta = db.list_thread_meta().expect("list meta");
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].thread_id, "thread::project");
        assert_eq!(meta[0].thread_type, "chat");
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
        assert_eq!(endpoints[0].thread_id.as_deref(), Some("thread::project"));

        assert!(
            db.remove_thread_meta_projection("thread::project")
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
}
