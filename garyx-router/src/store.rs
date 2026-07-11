use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::endpoint_projection::ChannelEndpointProjection;
use crate::tasks::TaskProjectionReader;

/// Abstract interface for thread storage.
///
/// Implementations can be in-memory, SQLite, etc. All methods are async to
/// support various backends, and every method surfaces backend failures as
/// [`ThreadStoreError`] — a read returning `Ok(None)` really means "absent",
/// and a write returning `Ok(())` really means "persisted".
#[async_trait]
pub trait ThreadStore: Send + Sync {
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
