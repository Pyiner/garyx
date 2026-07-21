//! Test-only fixtures shared by downstream crates' writer-contract tests.
//!
//! Compiled solely under the `test-seams` feature, which production builds
//! never enable — downstream crates opt in from `[dev-dependencies]`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::run_admission::ThreadRunCoordinator;
use crate::store::{
    ThreadPatchResult, ThreadRecordPatch, ThreadStore, ThreadStoreDomains, ThreadStoreError,
    ThreadTerminalState,
};

/// A [`ThreadStore`] spy for durable existing-record writer contracts: it
/// records every `patch`'s changed fields and every whole-record `set`.
///
/// The audited writers must stay field-scoped `patch` writers — a writer
/// regressing to whole-record `set` (which clobbers fields written
/// concurrently between its read and its persist) shows up in
/// [`PatchSpyThreadStore::set_thread_ids`], and a patch that grows beyond its
/// reviewed allowlist shows up in
/// [`PatchSpyThreadStore::patched_field_sets`].
#[derive(Default)]
pub struct PatchSpyThreadStore {
    records: Mutex<HashMap<String, Value>>,
    set_thread_ids: Mutex<Vec<String>>,
    patched_fields: Mutex<Vec<Vec<String>>>,
}

impl PatchSpyThreadStore {
    pub fn seeded(thread_id: &str, record: Value) -> Arc<Self> {
        let spy = Self::default();
        spy.records
            .lock()
            .expect("spy records lock")
            .insert(thread_id.to_owned(), record);
        Arc::new(spy)
    }

    pub fn record(&self, thread_id: &str) -> Option<Value> {
        self.records
            .lock()
            .expect("spy records lock")
            .get(thread_id)
            .cloned()
    }

    /// Thread ids that received a whole-record `set`.
    pub fn set_thread_ids(&self) -> Vec<String> {
        self.set_thread_ids.lock().expect("spy set lock").clone()
    }

    /// The changed-field names of each `patch`, in call order.
    pub fn patched_field_sets(&self) -> Vec<Vec<String>> {
        self.patched_fields.lock().expect("spy patch lock").clone()
    }
}

impl ThreadStoreDomains for PatchSpyThreadStore {
    fn run_coordinator(&self) -> Arc<ThreadRunCoordinator> {
        ThreadRunCoordinator::shared_fallback()
    }
}

#[async_trait]
impl ThreadStore for PatchSpyThreadStore {
    async fn terminal_state(
        &self,
        _thread_id: &str,
    ) -> Result<Option<ThreadTerminalState>, ThreadStoreError> {
        Ok(None)
    }

    async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
        Ok(self
            .records
            .lock()
            .expect("spy records lock")
            .get(thread_id)
            .cloned())
    }

    async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError> {
        self.set_thread_ids
            .lock()
            .expect("spy set lock")
            .push(thread_id.to_owned());
        self.records
            .lock()
            .expect("spy records lock")
            .insert(thread_id.to_owned(), data);
        Ok(())
    }

    async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
        Ok(self
            .records
            .lock()
            .expect("spy records lock")
            .remove(thread_id)
            .is_some())
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
        Ok(self
            .records
            .lock()
            .expect("spy records lock")
            .keys()
            .filter(|key| prefix.is_none_or(|prefix| key.starts_with(prefix)))
            .cloned()
            .collect())
    }

    async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
        Ok(self
            .records
            .lock()
            .expect("spy records lock")
            .contains_key(thread_id))
    }

    async fn patch(
        &self,
        thread_id: &str,
        patch: ThreadRecordPatch,
    ) -> Result<ThreadPatchResult, ThreadStoreError> {
        self.patched_fields
            .lock()
            .expect("spy patch lock")
            .push(patch.changed_fields().map(str::to_owned).collect());
        let mut records = self.records.lock().expect("spy records lock");
        let record = records
            .get_mut(thread_id)
            .ok_or_else(|| ThreadStoreError::NotFound(thread_id.to_owned()))?;
        let changed = patch.apply_to(record)?;
        Ok(if changed {
            ThreadPatchResult::Applied
        } else {
            ThreadPatchResult::Unchanged
        })
    }
}
