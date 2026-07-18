use crate::thread_record_normalization::strip_retired_recent_exclusion_fields;
use chrono::{DateTime, SecondsFormat, Utc};
use garyx_router::{KnownChannelEndpoint, ThreadTerminalState, bindings_from_value, is_thread_key};
pub use meetings::*;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, params, params_from_iter,
    types::Value as SqlValue,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
pub use task_forest::{
    CURRENT_TASK_PROJECTION_VERSION, TaskForestNode, TaskForestPage, TaskForestScope,
    TaskProjectionDraft,
};
use uuid::Uuid;

mod automation_runs;
mod capsules;
mod endpoints;
mod favorites;
mod lifecycle;
mod lock;
mod meta;
mod migrations;
mod pins;
mod read_only;
mod recent;
mod records;
mod schema;
mod store_incarnation;
mod summaries;
#[cfg(any(test, feature = "test-seams"))]
mod test_support;
mod workspaces;

pub use automation_runs::*;
pub use capsules::*;
use endpoints::*;
pub use favorites::*;
pub use lifecycle::*;
use lock::*;
pub use meta::*;
pub(crate) use migrations::*;
pub use pins::*;
pub(crate) use read_only::*;
pub use recent::*;
pub use records::*;
pub(crate) use schema::*;
pub use store_incarnation::*;
pub use summaries::*;
#[cfg(any(test, feature = "test-seams"))]
pub use test_support::*;
pub use workspaces::*;

mod meetings;

mod task_forest;

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

const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(5_000);

/// Read-pool size: enough to keep the common concurrent readers (desktop,
/// mobile, a handful of agents) off each other's locks without holding a
/// meaningful number of file handles.
const READ_POOL_SIZE: usize = 4;

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

#[cfg(test)]
mod tests;

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
}
