//! SQLite-backed thread record store (#TASK-1864 batch 2).
//!
//! Truth-source inversion for thread records: `thread_records` in
//! garyx-db.sqlite3 holds the canonical record bodies, and the five
//! projection tables are derived inside the same write transaction, so
//! projection drift is structurally impossible (design D2). Transcript
//! content stays in the jsonl transcripts (unchanged).
//!
//! This store folds in the three responsibilities of the former
//! `RecentThreadProjectingStore` wrapper: the per-key write lock, the
//! archived-thread write rejection, and the projection derivation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use garyx_router::{ThreadStore, ThreadStoreError, ThreadTranscriptStore, is_thread_key};
use serde_json::Value;
use tracing::warn;

use crate::garyx_db::{GaryxDbService, ThreadRecordProjections};
use crate::recent_thread_projection::{
    ActiveRunProbe, is_recent_thread_excluded, recent_thread_draft_from_thread_data_with_active_run,
    resolve_active_run_id,
};
use crate::task_projection::task_projection_draft_from_thread_data;
use crate::thread_meta_projection::thread_meta_projection_from_thread_data_with_active_run;
use garyx_router::is_hidden_thread_value;

pub(crate) struct SqliteThreadStore {
    garyx_db: Arc<GaryxDbService>,
    transcript_store: Arc<ThreadTranscriptStore>,
    active_run_probe: Arc<dyn ActiveRunProbe>,
    /// Per-key locks serializing read-merge-write cycles and the projection
    /// derivation for one key, so concurrent writes to the same thread
    /// cannot interleave (folded in from RecentThreadProjectingStore).
    key_locks: StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl SqliteThreadStore {
    pub(crate) fn new(
        garyx_db: Arc<GaryxDbService>,
        transcript_store: Arc<ThreadTranscriptStore>,
        active_run_probe: Arc<dyn ActiveRunProbe>,
    ) -> Self {
        Self {
            garyx_db,
            transcript_store,
            active_run_probe,
            key_locks: StdMutex::new(HashMap::new()),
        }
    }

    fn key_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.key_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(key.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Archived threads reject writes and clear their projections — same
    /// semantics as the former wrapper.
    async fn reject_archived_thread_write(&self, thread_id: &str) -> bool {
        if !is_thread_key(thread_id) {
            return false;
        }
        match self.garyx_db.is_thread_archived(thread_id) {
            Ok(true) => {
                if let Err(error) = self.garyx_db.unpin_thread(thread_id) {
                    warn!(thread_id, error = %error, "failed to unpin archived thread");
                }
                if let Err(error) = self.garyx_db.remove_recent_thread(thread_id) {
                    warn!(thread_id, error = %error, "failed to remove archived thread from recent projection");
                }
                if let Err(error) = self.garyx_db.remove_thread_meta_projection(thread_id) {
                    warn!(thread_id, error = %error, "failed to remove archived thread meta projection");
                }
                if let Err(error) = self.garyx_db.remove_task_projection(thread_id) {
                    warn!(thread_id, error = %error, "failed to remove archived task projection");
                }
                true
            }
            Ok(false) => false,
            Err(error) => {
                warn!(thread_id, error = %error, "failed to check archived thread tombstone before write");
                false
            }
        }
    }

    /// Derive the projection set for one thread record. Non-thread keys get
    /// `None` (record-only write).
    async fn derive_projections(
        &self,
        key: &str,
        data: &Value,
    ) -> Option<ThreadRecordProjections> {
        if !is_thread_key(key) {
            return None;
        }
        let active_run_id =
            resolve_active_run_id(&self.transcript_store, self.active_run_probe.as_ref(), key)
                .await;
        let thread_meta = thread_meta_projection_from_thread_data_with_active_run(
            key,
            data,
            active_run_id.clone(),
        );
        let task = task_projection_draft_from_thread_data(key, data);
        let recent = if is_hidden_thread_value(data) || is_recent_thread_excluded(data) {
            None
        } else {
            recent_thread_draft_from_thread_data_with_active_run(key, data, active_run_id)
        };
        Some(ThreadRecordProjections {
            thread_meta,
            task,
            recent,
        })
    }

    async fn write_record(&self, key: &str, mut data: Value) {
        // Structural invariant of the truth table: bodies never carry the
        // retired `messages` snapshot (#TASK-1864). Batch 1 removed every
        // producer; this strip guards legacy values arriving through
        // import/mirror paths.
        if let Some(object) = data.as_object_mut() {
            object.remove("messages");
        }
        let body = match serde_json::to_string(&data) {
            Ok(body) => body,
            Err(error) => {
                warn!(key, error = %error, "failed to serialize thread record body; dropping write");
                return;
            }
        };
        let updated_at = data
            .get("updated_at")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let projections = self.derive_projections(key, &data).await;
        let garyx_db = Arc::clone(&self.garyx_db);
        let key_owned = key.to_owned();
        let result = garyx_db
            .run_blocking(move |db| {
                db.write_thread_record_with_projections(
                    &key_owned,
                    &body,
                    updated_at.as_deref(),
                    projections,
                )
            })
            .await;
        if let Err(error) = result {
            warn!(key, error = %error, "failed to write thread record");
        }
    }
}

#[async_trait]
impl ThreadStore for SqliteThreadStore {
    async fn get(&self, thread_id: &str) -> Option<Value> {
        let garyx_db = Arc::clone(&self.garyx_db);
        let key = thread_id.to_owned();
        let body = match garyx_db
            .run_blocking(move |db| db.get_thread_record_body(&key))
            .await
        {
            Ok(body) => body?,
            Err(error) => {
                warn!(thread_id, error = %error, "failed to read thread record");
                return None;
            }
        };
        match serde_json::from_str(&body) {
            Ok(value) => Some(value),
            Err(error) => {
                warn!(thread_id, error = %error, "failed to parse thread record body");
                None
            }
        }
    }

    async fn set(&self, thread_id: &str, data: Value) {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        if self.reject_archived_thread_write(thread_id).await {
            return;
        }
        self.write_record(thread_id, data).await;
    }

    async fn delete(&self, thread_id: &str) -> bool {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        let garyx_db = Arc::clone(&self.garyx_db);
        let key = thread_id.to_owned();
        match garyx_db
            .run_blocking(move |db| db.delete_thread_record_with_projections(&key))
            .await
        {
            Ok(removed) => removed,
            Err(error) => {
                warn!(thread_id, error = %error, "failed to delete thread record");
                false
            }
        }
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Vec<String> {
        let garyx_db = Arc::clone(&self.garyx_db);
        let prefix = prefix.map(ToOwned::to_owned);
        match garyx_db
            .run_blocking(move |db| db.list_thread_record_keys(prefix.as_deref()))
            .await
        {
            Ok(keys) => keys,
            Err(error) => {
                warn!(error = %error, "failed to list thread record keys");
                Vec::new()
            }
        }
    }

    async fn exists(&self, thread_id: &str) -> bool {
        let garyx_db = Arc::clone(&self.garyx_db);
        let key = thread_id.to_owned();
        match garyx_db
            .run_blocking(move |db| db.thread_record_exists(&key))
            .await
        {
            Ok(exists) => exists,
            Err(error) => {
                warn!(thread_id, error = %error, "failed to check thread record existence");
                false
            }
        }
    }

    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        if self.reject_archived_thread_write(thread_id).await {
            return Ok(());
        }
        // Read-merge-write under the per-key lock: equivalent to an atomic
        // top-level merge because no other writer for this key can
        // interleave, and the write itself is a single transaction.
        let mut data = self
            .get(thread_id)
            .await
            .ok_or_else(|| ThreadStoreError::NotFound(thread_id.to_owned()))?;
        if let (Some(target), Some(updates)) = (data.as_object_mut(), updates.as_object()) {
            for (key, value) in updates {
                target.insert(key.clone(), value.clone());
            }
        }
        self.write_record(thread_id, data).await;
        Ok(())
    }
}

/// Contract suite run against every ThreadStore implementation
/// (File / InMemory / Sqlite on memory and file databases): get/set/
/// update/delete/list_keys/exists must agree (#TASK-1864 batch 2).
#[cfg(test)]
mod contract_tests {
    use garyx_router::{FileThreadStore, InMemoryThreadStore};
    use serde_json::json;

    use super::*;
    use crate::recent_thread_projection::AlwaysActiveRunProbe;

    fn sqlite_store(garyx_db: Arc<GaryxDbService>) -> SqliteThreadStore {
        SqliteThreadStore::new(
            garyx_db,
            Arc::new(ThreadTranscriptStore::memory()),
            Arc::new(AlwaysActiveRunProbe),
        )
    }

    async fn run_contract(store: &dyn ThreadStore) {
        // Missing key.
        assert_eq!(store.get("thread::missing").await, None);
        assert!(!store.exists("thread::missing").await);
        assert!(!store.delete("thread::missing").await);
        assert!(
            store
                .update("thread::missing", json!({"label": "x"}))
                .await
                .is_err(),
            "update of a missing thread must error"
        );

        // Round trip.
        store
            .set(
                "thread::alpha",
                json!({"thread_id": "thread::alpha", "label": "first"}),
            )
            .await;
        let read = store.get("thread::alpha").await.expect("read back");
        assert_eq!(read["label"], "first");
        assert!(store.exists("thread::alpha").await);

        // Overwrite replaces the whole value.
        store
            .set(
                "thread::alpha",
                json!({"thread_id": "thread::alpha", "generation": 2}),
            )
            .await;
        let read = store.get("thread::alpha").await.expect("read v2");
        assert_eq!(read["generation"], 2);
        assert!(read.get("label").is_none(), "set is a full replace");

        // Update merges top-level keys.
        store
            .update("thread::alpha", json!({"label": "merged", "extra": true}))
            .await
            .expect("update");
        let read = store.get("thread::alpha").await.expect("read merged");
        assert_eq!(read["generation"], 2);
        assert_eq!(read["label"], "merged");
        assert_eq!(read["extra"], true);

        // Non-thread keys are ordinary records.
        store
            .set("meta::known_channel_endpoints", json!({"endpoints": []}))
            .await;
        store
            .set("cron::job-1", json!({"schedule": "daily"}))
            .await;

        // list_keys: all + prefix.
        let mut all = store.list_keys(None).await;
        all.sort();
        assert_eq!(
            all,
            vec![
                "cron::job-1".to_owned(),
                "meta::known_channel_endpoints".to_owned(),
                "thread::alpha".to_owned(),
            ]
        );
        let mut threads = store.list_keys(Some("thread::")).await;
        threads.sort();
        assert_eq!(threads, vec!["thread::alpha".to_owned()]);

        // Delete.
        assert!(store.delete("thread::alpha").await);
        assert!(!store.delete("thread::alpha").await);
        assert_eq!(store.get("thread::alpha").await, None);
        assert!(!store.exists("thread::alpha").await);
    }

    #[tokio::test]
    async fn in_memory_store_satisfies_the_contract() {
        let store = InMemoryThreadStore::new();
        run_contract(&store).await;
    }

    #[tokio::test]
    async fn file_store_satisfies_the_contract() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = FileThreadStore::new(dir.path()).await.expect("file store");
        run_contract(&store).await;
    }

    #[tokio::test]
    async fn sqlite_store_satisfies_the_contract_on_a_memory_database() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let store = sqlite_store(garyx_db);
        run_contract(&store).await;
    }

    #[tokio::test]
    async fn sqlite_store_satisfies_the_contract_on_a_file_database() {
        // File databases exercise the dedicated reader connection.
        let dir = tempfile::tempdir().expect("temp dir");
        let garyx_db = Arc::new(
            GaryxDbService::open(dir.path().join("garyx-db.sqlite3")).expect("db opens"),
        );
        let store = sqlite_store(garyx_db);
        run_contract(&store).await;
    }

    #[tokio::test]
    async fn sqlite_store_strips_legacy_messages_and_derives_projections() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let store = sqlite_store(Arc::clone(&garyx_db));
        let thread_id = "thread::sqlite-projections";

        store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "label": "Projected",
                    "updated_at": "2026-07-08T00:00:00Z",
                    "history": {"message_count": 3, "last_message_at": "2026-07-08T00:00:00Z"},
                    "last_user_preview": "hello preview",
                    // Legacy snapshot arriving through an import/mirror
                    // path must never reach the truth table.
                    "messages": [{"role": "user", "content": "legacy"}],
                }),
            )
            .await;

        let read = store.get(thread_id).await.expect("read back");
        assert!(
            read.get("messages").is_none(),
            "record bodies never carry the retired messages snapshot"
        );

        // Projections were derived in the same write.
        let recent = garyx_db
            .list_recent_threads(10, 0)
            .expect("list recent")
            .into_iter()
            .find(|row| row.thread_id == thread_id)
            .expect("recent projection row");
        assert_eq!(recent.last_message_preview, "hello preview");

        // Hidden rewrite removes the recent row, keeps the record.
        store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "label": "Projected",
                    "hidden": true,
                }),
            )
            .await;
        assert!(
            !garyx_db
                .list_recent_threads(10, 0)
                .expect("list recent")
                .iter()
                .any(|row| row.thread_id == thread_id)
        );
        assert!(store.exists(thread_id).await);

        // Archived threads reject writes entirely.
        garyx_db.mark_thread_archived(thread_id).expect("archive");
        store
            .set(thread_id, json!({"thread_id": thread_id, "label": "after-archive"}))
            .await;
        let read = store.get(thread_id).await.expect("record still readable");
        assert_eq!(
            read["label"], "Projected",
            "archived threads must reject writes"
        );
    }
}
