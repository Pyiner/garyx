use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::run_admission::ThreadRunCoordinator;
use crate::store::{AtomicRecordMerge, ThreadStore, ThreadStoreError, ThreadTerminalState};

#[derive(Default)]
struct MemoryState {
    records: HashMap<String, Value>,
    terminal_states: HashMap<String, ThreadTerminalState>,
}

/// In-memory thread storage using a `HashMap` behind a [`RwLock`].
///
/// Suitable for development and single-instance deployments.
pub struct InMemoryThreadStore {
    store: Arc<RwLock<MemoryState>>,
    run_coordinator: Arc<ThreadRunCoordinator>,
}

impl InMemoryThreadStore {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(MemoryState::default())),
            run_coordinator: Arc::new(ThreadRunCoordinator::new()),
        }
    }

    /// Remove all threads.
    pub async fn clear(&self) {
        *self.store.write().await = MemoryState::default();
    }

    /// Return the number of stored threads.
    pub async fn size(&self) -> usize {
        self.store.read().await.records.len()
    }

    #[cfg(test)]
    pub(crate) async fn seed_terminal_state(&self, thread_id: &str, terminal: ThreadTerminalState) {
        let mut state = self.store.write().await;
        state.records.remove(thread_id);
        state.terminal_states.insert(thread_id.to_owned(), terminal);
    }
}

impl Default for InMemoryThreadStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ThreadStore for InMemoryThreadStore {
    fn run_coordinator(&self) -> Arc<ThreadRunCoordinator> {
        self.run_coordinator.clone()
    }

    async fn terminal_state(
        &self,
        thread_id: &str,
    ) -> Result<Option<ThreadTerminalState>, ThreadStoreError> {
        Ok(self
            .store
            .read()
            .await
            .terminal_states
            .get(thread_id)
            .copied())
    }

    async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
        Ok(self.store.read().await.records.get(thread_id).cloned())
    }

    async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError> {
        let mut state = self.store.write().await;
        if state.terminal_states.contains_key(thread_id) {
            return Err(ThreadStoreError::Archived(thread_id.to_owned()));
        }
        state.records.insert(thread_id.to_owned(), data);
        drop(state);
        self.run_coordinator.record_written(thread_id);
        Ok(())
    }

    async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
        let mut reservation = self
            .run_coordinator
            .reserve_delete(self, thread_id)
            .await
            .map_err(|error| ThreadStoreError::Backend(error.to_string()))?;
        self.run_coordinator
            .abort_and_drain_delete(&reservation)
            .await
            .map_err(|error| ThreadStoreError::Backend(error.to_string()))?;
        let prior = reservation.prior_terminal();
        let mut state = self.store.write().await;
        let removed = state.records.remove(thread_id).is_some();
        let upgraded = state.terminal_states.get(thread_id) == Some(&ThreadTerminalState::Archived);
        if removed || upgraded {
            state
                .terminal_states
                .insert(thread_id.to_owned(), ThreadTerminalState::Deleted);
        }
        drop(state);
        if removed || upgraded || prior == Some(ThreadTerminalState::Deleted) {
            reservation.settle_committed(Some(ThreadTerminalState::Deleted));
        } else {
            reservation.settle_decision(None);
        }
        Ok(removed || upgraded)
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
        let guard = self.store.read().await;
        Ok(match prefix {
            Some(p) => guard
                .records
                .keys()
                .filter(|k| k.starts_with(p))
                .cloned()
                .collect(),
            None => guard.records.keys().cloned().collect(),
        })
    }

    async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
        Ok(self.store.read().await.records.contains_key(thread_id))
    }

    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
        let mut guard = self.store.write().await;
        if guard.terminal_states.contains_key(thread_id) {
            return Err(ThreadStoreError::Archived(thread_id.to_owned()));
        }
        let entry = guard
            .records
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
            if guard.terminal_states.contains_key(&entry.thread_id) {
                return Err(ThreadStoreError::Archived(entry.thread_id.clone()));
            }
            if !entry.create_if_missing && !guard.records.contains_key(&entry.thread_id) {
                return Err(ThreadStoreError::NotFound(entry.thread_id.clone()));
            }
        }
        for entry in entries {
            let record = guard
                .records
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
