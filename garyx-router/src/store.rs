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
}

/// Errors returned by [`ThreadStore`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ThreadStoreError {
    #[error("thread not found: {0}")]
    NotFound(String),
}
