use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::ThreadRunCoordinator;
use crate::endpoint_projection::ChannelEndpointProjection;
use crate::tasks::TaskProjectionReader;

/// Durable terminal state recorded by the canonical thread tombstone.
///
/// Both variants reject record writes. The distinction is retained for
/// lifecycle result-matrix decisions and diagnostics.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThreadTerminalState {
    Archived,
    Deleted,
}

impl ThreadTerminalState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "archived" => Some(Self::Archived),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

/// Abstract interface for thread storage.
///
/// Implementations can be in-memory, SQLite, etc. All methods are async to
/// support various backends, and every method surfaces backend failures as
/// [`ThreadStoreError`] — a read returning `Ok(None)` really means "absent",
/// and a write returning `Ok(())` really means "persisted".
/// One record's top-level field merge inside an atomic multi-record
/// mutation (see [`ThreadStore::update_many_atomic`]).
#[derive(Debug, Clone)]
pub struct AtomicRecordMerge {
    pub thread_id: String,
    pub fields: Value,
    /// Missing records normally abort the mutation; registry-style
    /// records owned by the mutation itself are created on first write.
    pub create_if_missing: bool,
}

#[async_trait]
pub trait ThreadStore: Send + Sync {
    /// The store-owned run/mutation linearization domain. Production stores
    /// override this with a per-store coordinator; the shared fallback keeps
    /// small test doubles fail-closed without expanding every fixture.
    fn run_coordinator(&self) -> Arc<ThreadRunCoordinator> {
        ThreadRunCoordinator::shared_fallback()
    }

    /// The canonical durable terminal tombstone, if one exists.
    async fn terminal_state(
        &self,
        _thread_id: &str,
    ) -> Result<Option<ThreadTerminalState>, ThreadStoreError> {
        Ok(None)
    }

    /// Whether the canonical record has any durable terminal tombstone.
    async fn is_archived(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
        Ok(self.terminal_state(thread_id).await?.is_some())
    }

    /// Retrieve thread data by key. `Ok(None)` means the thread does not
    /// exist; backend/parse failures are errors, never `None`.
    async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError>;

    /// Store or update thread data. Writes to archived threads are rejected
    /// with [`ThreadStoreError::Archived`] instead of silently succeeding.
    async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError>;

    /// Delete a thread. Returns `Ok(true)` if the thread existed.
    async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError>;

    /// List all thread ids, optionally filtered by prefix.
    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError>;

    /// Check if a thread exists.
    async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError>;

    /// Update specific fields in a thread without replacing the entire value.
    ///
    /// The default contract merges top-level keys from `updates` into the
    /// existing thread object. Backends may override for atomic merge
    /// support.
    ///
    /// # Errors
    ///
    /// Returns [`ThreadStoreError::NotFound`] if the thread does not exist
    /// and [`ThreadStoreError::Archived`] if it is tombstoned.
    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError>;

    /// Apply top-level field merges to SEVERAL records as one
    /// all-or-nothing mutation. Multi-record state transitions (moving an
    /// endpoint binding touches the previous owner, the target, and the
    /// known-endpoint registry) must never commit partially: a storage
    /// failure mid-mutation would otherwise lose the active binding
    /// (#TASK-2099 root final review).
    ///
    /// A missing record aborts the whole mutation with
    /// [`ThreadStoreError::NotFound`] unless the entry sets
    /// `create_if_missing` (the known-endpoint registry record is created
    /// on first bind); vanished thread records are never resurrected as
    /// skeletons.
    ///
    /// Every backend that participates in multi-record mutations MUST
    /// supply a genuinely atomic implementation (the SQLite store commits
    /// every record and its derived projections in one transaction; the
    /// in-memory store applies the batch under a single write guard). The
    /// default REFUSES before touching storage: an API named atomic must
    /// never partially commit, so there is no sequential fallback
    /// (#TASK-2099 root final review).
    async fn update_many_atomic(
        &self,
        entries: Vec<AtomicRecordMerge>,
    ) -> Result<(), ThreadStoreError> {
        drop(entries);
        Err(ThreadStoreError::Backend(
            "this thread store backend does not support atomic multi-record mutations".to_owned(),
        ))
    }

    /// Count keys, optionally filtered by prefix. Backends with SQL
    /// storage override this with a COUNT query; the default lists keys.
    async fn count_keys(&self, prefix: Option<&str>) -> Result<usize, ThreadStoreError> {
        Ok(self.list_keys(prefix).await?.len())
    }

    /// The SQL channel-endpoint projection maintained by this store, when
    /// the backend derives one in the same transaction as record writes
    /// (the SQLite store). `None` means condition queries fall back to
    /// [`crate::endpoint_projection::ScanChannelEndpointProjection`], the
    /// structural equivalent for in-memory stores. Tied to the store's own
    /// lifetime — there is no process-global registry.
    fn channel_endpoint_projection(&self) -> Option<Arc<dyn ChannelEndpointProjection>> {
        None
    }

    /// The SQL task projection maintained by this store, when the backend
    /// derives one in the same transaction as record writes. `None` means
    /// task condition queries fall back to the scan reader.
    fn task_projection(&self) -> Option<Arc<dyn TaskProjectionReader>> {
        None
    }
}

/// Logged fallbacks for call sites whose signatures cannot surface a
/// [`ThreadStoreError`] (fire-and-forget writers, Option-shaped read
/// helpers). Failures stay observable through the log instead of being
/// silently folded into absent values; paths that can propagate should
/// call the fallible methods directly.
#[async_trait]
pub trait ThreadStoreExt: ThreadStore {
    async fn get_logged(&self, thread_id: &str) -> Option<Value> {
        match self.get(thread_id).await {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(thread_id, error = %error, "thread store read failed");
                None
            }
        }
    }

    /// Write that logs failures (including archived-thread rejection) and
    /// reports whether the write persisted.
    async fn set_logged(&self, thread_id: &str, data: Value) -> bool {
        match self.set(thread_id, data).await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(thread_id, error = %error, "thread store write failed");
                false
            }
        }
    }

    async fn delete_logged(&self, thread_id: &str) -> bool {
        match self.delete(thread_id).await {
            Ok(removed) => removed,
            Err(error) => {
                tracing::warn!(thread_id, error = %error, "thread store delete failed");
                false
            }
        }
    }

    async fn list_keys_logged(&self, prefix: Option<&str>) -> Vec<String> {
        match self.list_keys(prefix).await {
            Ok(keys) => keys,
            Err(error) => {
                tracing::warn!(error = %error, "thread store key listing failed");
                Vec::new()
            }
        }
    }

    async fn exists_logged(&self, thread_id: &str) -> bool {
        match self.exists(thread_id).await {
            Ok(exists) => exists,
            Err(error) => {
                tracing::warn!(thread_id, error = %error, "thread store existence check failed");
                false
            }
        }
    }
}

impl<T: ThreadStore + ?Sized> ThreadStoreExt for T {}

/// Errors returned by [`ThreadStore`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ThreadStoreError {
    #[error("thread not found: {0}")]
    NotFound(String),
    /// The thread is archived (tombstoned): writes are rejected so callers
    /// cannot mistake a dropped write for a persisted one.
    #[error("thread is archived: {0}")]
    Archived(String),
    #[error("thread record serialization failed for {thread_id}: {message}")]
    Serialization { thread_id: String, message: String },
    #[error("thread store backend failed: {0}")]
    Backend(String),
}
