//! Canonical thread-record storage contract.
//!
//! Layout:
//! - [`ThreadStore`]: the storage contract — point operations plus the two
//!   validated write shapes ([`ThreadRecordPatch`] and the privileged
//!   [`AtomicRecordMerge`] batch).
//! - [`ThreadStoreDomains`]: store-owned runtime domains (run coordination
//!   and SQL projection read seams), split from the storage contract.
//! - [`patch`](self::patch): validated mutation witnesses.
//! - [`channel_bindings`](self::channel_bindings): protected-field guards.
//! - [`contract`]: the executable contract every backend runs in its tests.

mod channel_bindings;
pub mod contract;
mod patch;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::ThreadRunCoordinator;
use crate::endpoint_projection::ChannelEndpointProjection;
use crate::tasks::TaskProjectionReader;

pub use channel_bindings::{
    ChannelBindingsMergeAuthority, ensure_channel_bindings_unchanged, validate_channel_bindings,
};
pub use patch::{AtomicRecordMerge, ThreadPatchResult, ThreadRecordPatch};

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

/// Store-owned runtime domains, split from the storage contract.
///
/// Each accessor hands out a per-store singleton whose lifetime is tied to
/// the store instance — there is no process-global registry — which is why
/// the accessors live on the store's trait family at all. Delegating
/// wrappers must forward every accessor so they share their inner store's
/// truth source; a wrapper that silently keeps the defaults would split
/// the linearization domain or drop the SQL projections.
pub trait ThreadStoreDomains: Send + Sync {
    /// The store-owned run/mutation linearization domain. Production stores
    /// override this with a per-store coordinator; the shared fallback keeps
    /// small test doubles fail-closed without expanding every fixture.
    fn run_coordinator(&self) -> Arc<ThreadRunCoordinator> {
        ThreadRunCoordinator::shared_fallback()
    }

    /// The SQL channel-endpoint projection maintained by this store, when
    /// the backend derives one in the same transaction as record writes
    /// (the SQLite store). `None` means condition queries fall back to
    /// [`crate::endpoint_projection::ScanChannelEndpointProjection`], the
    /// structural equivalent for in-memory stores.
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

/// Abstract interface for canonical thread-record storage.
///
/// Implementations can be in-memory, SQLite, etc. All methods are async to
/// support various backends, and every method surfaces backend failures as
/// [`ThreadStoreError`] — a read returning `Ok(None)` really means "absent",
/// and a write returning `Ok(())` really means "persisted".
///
/// The write contract has exactly three shapes:
/// - [`set`](Self::set): full replace of one record, with the protected
///   `channel_bindings` field required unchanged.
/// - [`patch`](Self::patch): validated top-level mutation of one existing
///   record, proven safe at [`ThreadRecordPatch`] construction time.
/// - [`update_many_atomic`](Self::update_many_atomic): the privileged
///   all-or-nothing multi-record merge — the only shape allowed to change
///   endpoint bindings.
///
/// Two invariants must hold inside every backend's own locking/transaction
/// domain and therefore cannot be centralized here: writes to tombstoned
/// threads fail with [`ThreadStoreError::Archived`] (checked inside the
/// write transaction or write guard, so a racing archive is never
/// overtaken), and `set` rejects `channel_bindings` changes under the same
/// guard. The crate publishes these obligations as runnable assertions in
/// [`contract`]; every backend must run that suite from its tests.
#[async_trait]
pub trait ThreadStore: ThreadStoreDomains {
    /// The canonical durable terminal tombstone, if one exists. There is no
    /// default: a backend or wrapper without tombstone storage must say so
    /// explicitly instead of silently reporting every thread live.
    async fn terminal_state(
        &self,
        thread_id: &str,
    ) -> Result<Option<ThreadTerminalState>, ThreadStoreError>;

    /// Retrieve thread data by key. `Ok(None)` means the thread does not
    /// exist; backend/parse failures are errors, never `None`.
    async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError>;

    /// Store or update thread data as a full replace. Writes to archived
    /// threads are rejected with [`ThreadStoreError::Archived`] instead of
    /// silently succeeding, and a replacement that changes the protected
    /// `channel_bindings` field of an existing record is rejected with
    /// [`ThreadStoreError::ProtectedFieldConflict`].
    async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError>;

    /// Delete a thread, recording the durable `Deleted` tombstone. Returns
    /// `Ok(true)` if the thread existed.
    async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError>;

    /// List all thread ids, optionally filtered by prefix. This is a key
    /// listing only — condition queries belong to SQL projections.
    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError>;

    /// Check if a thread exists.
    async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError>;

    /// Apply a validated top-level patch to an existing record. Missing
    /// records are [`ThreadStoreError::NotFound`]; tombstoned records are
    /// [`ThreadStoreError::Archived`]. Backends that cannot hold their
    /// write guard across re-read and merge fail closed.
    async fn patch(
        &self,
        thread_id: &str,
        patch: ThreadRecordPatch,
    ) -> Result<ThreadPatchResult, ThreadStoreError> {
        drop((thread_id, patch));
        Err(ThreadStoreError::Backend(
            "this thread store backend does not support atomic record patches".to_owned(),
        ))
    }

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
    /// Entry construction is where the binding privilege is enforced:
    /// [`AtomicRecordMerge::new`] rejects the protected `channel_bindings`
    /// field, and binding-carrying entries exist only through
    /// [`AtomicRecordMerge::channel_bindings_merge`] under the
    /// [`ChannelBindingsMergeAuthority`] witness.
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
    #[error("thread record patch is invalid: {0}")]
    InvalidPatch(String),
    #[error("thread '{thread_id}' protected field conflict: {field}")]
    ProtectedFieldConflict { thread_id: String, field: String },
    #[error("thread store backend failed: {0}")]
    Backend(String),
}
