use async_trait::async_trait;
use serde_json::Value;

/// Abstract interface for thread storage.
///
/// Implementations can be in-memory, Redis, database, etc.
/// All methods are async to support various backends.
#[async_trait]
pub trait ThreadStore: Send + Sync {
    /// Retrieve thread data by key.
    ///
    /// Thread id format: `{agentId}::{type}::{peerId}`
    async fn get(&self, thread_id: &str) -> Option<Value>;

    /// Store or update thread data.
    async fn set(&self, thread_id: &str, data: Value);

    /// Delete a thread. Returns `true` if the thread existed.
    async fn delete(&self, thread_id: &str) -> bool;

    /// List all thread ids, optionally filtered by prefix.
    async fn list_keys(&self, prefix: Option<&str>) -> Vec<String>;

    /// Check if a thread exists.
    async fn exists(&self, thread_id: &str) -> bool;

    /// Update specific fields in a thread without replacing the entire value.
    ///
    /// The default implementation merges top-level keys from `updates` into the
    /// existing thread object. Backends may override for atomic merge support.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the thread does not exist.
    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError>;

    /// Atomically clear `history.active_run_snapshot` only if it belongs to
    /// `run_id`, returning true if a snapshot was cleared.
    ///
    /// Used when a run leaves the active set without reaching its terminal (an
    /// aborted/preempted run, whose persistence task is dropped): the lingering
    /// overlay would otherwise keep the thread projected as `running`. The
    /// `run_id` ownership check ensures a replacement run that has already written
    /// its own snapshot is never clobbered.
    ///
    /// The default implementation is a non-atomic get/check/set; backends that can
    /// take a per-thread write lock should override it so a concurrent writer
    /// cannot race the read-modify-write.
    async fn clear_active_run_snapshot_if_owned(&self, thread_id: &str, run_id: &str) -> bool {
        let Some(mut data) = self.get(thread_id).await else {
            return false;
        };
        if crate::thread_history::active_run_snapshot_run_id(&data).as_deref() != Some(run_id) {
            return false;
        }
        if crate::thread_history::remove_active_run_snapshot(&mut data) {
            self.set(thread_id, data).await;
            true
        } else {
            false
        }
    }
}

/// Errors returned by [`ThreadStore`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ThreadStoreError {
    #[error("thread not found: {0}")]
    NotFound(String),
}
