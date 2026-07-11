use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::store::{AtomicRecordMerge, ThreadStore, ThreadStoreError};

/// In-memory thread storage using a `HashMap` behind a [`RwLock`].
///
/// Suitable for development and single-instance deployments.
pub struct InMemoryThreadStore {
    store: RwLock<HashMap<String, Value>>,
}

impl InMemoryThreadStore {
    pub fn new() -> Self {
        Self {
            store: RwLock::new(HashMap::new()),
        }
    }

    /// Remove all threads.
    pub async fn clear(&self) {
        self.store.write().await.clear();
    }

    /// Return the number of stored threads.
    pub async fn size(&self) -> usize {
        self.store.read().await.len()
    }
}

impl Default for InMemoryThreadStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ThreadStore for InMemoryThreadStore {
    async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
        Ok(self.store.read().await.get(thread_id).cloned())
    }

    async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError> {
        self.store.write().await.insert(thread_id.to_owned(), data);
        Ok(())
    }

    async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
        Ok(self.store.write().await.remove(thread_id).is_some())
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
        let guard = self.store.read().await;
        Ok(match prefix {
            Some(p) => guard.keys().filter(|k| k.starts_with(p)).cloned().collect(),
            None => guard.keys().cloned().collect(),
        })
    }

    async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
        Ok(self.store.read().await.contains_key(thread_id))
    }

    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
        let mut guard = self.store.write().await;
        let entry = guard
            .get_mut(thread_id)
            .ok_or_else(|| ThreadStoreError::NotFound(thread_id.to_owned()))?;

        if let (Some(existing), Some(new_fields)) = (entry.as_object_mut(), updates.as_object()) {
            for (k, v) in new_fields {
                existing.insert(k.clone(), v.clone());
            }
        }
        Ok(())
    }

    async fn update_many_atomic(
        &self,
        entries: Vec<AtomicRecordMerge>,
    ) -> Result<(), ThreadStoreError> {
        // One write guard across validation and application: either every
        // entry applies or none do.
        let mut guard = self.store.write().await;
        for entry in &entries {
            if !entry.create_if_missing && !guard.contains_key(&entry.thread_id) {
                return Err(ThreadStoreError::NotFound(entry.thread_id.clone()));
            }
        }
        for entry in entries {
            let record = guard
                .entry(entry.thread_id)
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if let (Some(existing), Some(new_fields)) =
                (record.as_object_mut(), entry.fields.as_object())
            {
                for (k, v) in new_fields {
                    existing.insert(k.clone(), v.clone());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
