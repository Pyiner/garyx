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

use crate::garyx_db::{
    GaryxDbResult, GaryxDbService, ThreadRecordProjections, is_retired_workflow_thread_record,
};
use crate::recent_thread_projection::{
    ActiveRunProbe, is_recent_thread_excluded, recent_thread_draft_from_thread_data_with_active_run,
    resolve_active_run_id,
};
use crate::task_projection::task_projection_draft_from_thread_data;
use crate::thread_meta_projection::thread_meta_projection_from_thread_data_with_active_run;
use garyx_router::is_hidden_thread_value;

/// Remove fields that must never reach the truth table: the retired
/// `messages` snapshot (#TASK-1864 batch 1).
pub(crate) fn strip_retired_record_fields(data: &mut Value) {
    if let Some(object) = data.as_object_mut() {
        object.remove("messages");
    }
}

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
    /// semantics as the former wrapper. One blocking hop covers the
    /// tombstone check and the projection cleanup.
    async fn reject_archived_thread_write(&self, thread_id: &str) -> bool {
        if !is_thread_key(thread_id) {
            return false;
        }
        let owned_thread_id = thread_id.to_owned();
        let rejected = self
            .garyx_db
            .run_blocking(move |db| {
                let thread_id = owned_thread_id.as_str();
                match db.is_thread_archived(thread_id) {
                    Ok(true) => {
                        if let Err(error) = db.unpin_thread(thread_id) {
                            warn!(thread_id, error = %error, "failed to unpin archived thread");
                        }
                        if let Err(error) = db.remove_recent_thread(thread_id) {
                            warn!(thread_id, error = %error, "failed to remove archived thread from recent projection");
                        }
                        if let Err(error) = db.remove_thread_meta_projection(thread_id) {
                            warn!(thread_id, error = %error, "failed to remove archived thread meta projection");
                        }
                        if let Err(error) = db.remove_task_projection(thread_id) {
                            warn!(thread_id, error = %error, "failed to remove archived task projection");
                        }
                        Ok(true)
                    }
                    Ok(false) => Ok(false),
                    Err(error) => {
                        warn!(thread_id, error = %error, "failed to check archived thread tombstone before write");
                        Ok(false)
                    }
                }
            })
            .await;
        rejected.unwrap_or(false)
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
        // producer; this strip guards legacy values arriving through the
        // boot-import path.
        strip_retired_record_fields(&mut data);
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
        // Archived threads silently drop writes (review #TASK-1901).
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
        // Archived threads silently drop writes (review #TASK-1901).
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

/// Assemble the SQLite-backed thread store for runtime wiring
/// (#TASK-1864): build the store over `garyx_db` and run the one-shot
/// boot import from the file archive when this machine has not imported
/// yet. SQLite is the only backend — the dual-write mirror and backend
/// selection are retired. The returned store is what
/// `AppStateBuilder::with_thread_store` should receive; the builder
/// deliberately does not re-wrap SQLite backends in the projecting store.
pub async fn assemble_sqlite_thread_store(
    garyx_db: Arc<GaryxDbService>,
    transcript_store: Arc<ThreadTranscriptStore>,
    bridge: &Arc<garyx_bridge::MultiProviderBridge>,
    import_source: Arc<dyn ThreadStore>,
) -> GaryxDbResult<Arc<dyn ThreadStore>> {
    let probe: Arc<dyn ActiveRunProbe> = Arc::new(
        crate::recent_thread_projection::BridgeActiveRunProbe::new(Arc::downgrade(bridge)),
    );
    let sqlite_store = SqliteThreadStore::new(
        Arc::clone(&garyx_db),
        Arc::clone(&transcript_store),
        probe,
    );
    import_thread_records_if_needed(&garyx_db, &import_source, &sqlite_store, &transcript_store)
        .await;
    garyx_db
        .run_blocking(|db| db.run_thread_data_startup_migrations())
        .await?;
    Ok(Arc::new(sqlite_store))
}

pub(crate) const THREAD_RECORDS_IMPORT_NAME: &str = "thread_records_import";
const THREAD_RECORDS_IMPORT_VERSION: i64 = 1;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ThreadRecordImportSummary {
    pub source_keys: usize,
    pub imported: usize,
    pub skipped: usize,
    pub transcripts_backfilled: usize,
}

/// One-shot boot import (#TASK-1864 batch 2, D7): stream every record out
/// of the file archive (FileThreadStore supplies the legacy parsing and
/// scrub for free), backfill transcripts for pre-transcript threads from
/// their `messages` snapshot, strip the snapshot, seed the write-time
/// preview fields and the legacy task-body fallback, then write each
/// record plus its projections in one transaction. Runs once per machine:
/// a matching migration-state row skips it entirely.
pub(crate) async fn import_thread_records_if_needed(
    garyx_db: &Arc<GaryxDbService>,
    source: &Arc<dyn ThreadStore>,
    sqlite_store: &SqliteThreadStore,
    transcript_store: &Arc<ThreadTranscriptStore>,
) -> ThreadRecordImportSummary {
    let source_keys = source.list_keys(None).await;
    // Gate on state-row existence, not the key count: in steady state new
    // threads change the count, and a count-sensitive gate would re-import
    // on every boot, flowing the stale file archive back over the SQL
    // truth. Clearing the state row is the only event that forces a
    // re-import.
    match garyx_db
        .projection_state_exists(THREAD_RECORDS_IMPORT_NAME, THREAD_RECORDS_IMPORT_VERSION)
    {
        Ok(true) => {
            return ThreadRecordImportSummary {
                source_keys: source_keys.len(),
                ..Default::default()
            };
        }
        Ok(false) => {}
        Err(error) => {
            warn!(error = %error, "failed to check thread record import state; importing");
        }
    }

    // Safety interlock: an empty archive with a populated truth table
    // means the archive was retired (moved to backups) — importing it
    // would wipe nothing but recording state over live data is wrong
    // either way. Never "import" emptiness over an existing truth table.
    if source_keys.is_empty() {
        match garyx_db.list_thread_record_keys(None) {
            Ok(existing) if !existing.is_empty() => {
                warn!(
                    existing = existing.len(),
                    "skipping thread-record import: source archive is empty but the truth table is populated"
                );
                return ThreadRecordImportSummary::default();
            }
            _ => {}
        }
    }
    let mut summary = ThreadRecordImportSummary {
        source_keys: source_keys.len(),
        ..Default::default()
    };
    for key in &source_keys {
        let Some(mut data) = source.get(key).await else {
            summary.skipped += 1;
            continue;
        };
        // The file store is only an upgrade archive. Removed product records
        // are destroyed here instead of being imported into SQLite and then
        // requiring a compatibility decoder.
        if is_thread_key(key) && is_retired_workflow_thread_record(&data) {
            summary.skipped += 1;
            if let Err(error) = transcript_store.delete(key).await {
                warn!(key, error = %error, "failed to delete retired workflow transcript during import");
            }
            continue;
        }
        let legacy_messages = data
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if is_thread_key(key) && !legacy_messages.is_empty() {
            // Pre-transcript threads: rebuild the transcript from the
            // legacy snapshot so the D5 fallback readers can retire.
            if !transcript_store.exists(key).await {
                match transcript_store
                    .rewrite_from_messages(key, &legacy_messages)
                    .await
                {
                    Ok(_) => summary.transcripts_backfilled += 1,
                    Err(error) => {
                        warn!(key, error = %error, "failed to backfill transcript from legacy messages");
                    }
                }
            }
            if let Some(object) = data.as_object_mut() {
                // Seed the write-time preview fields the projections read.
                for role in ["user", "assistant"] {
                    if let Some(field) =
                        garyx_models::message_preview::preview_field_for_role(role)
                        && !object.contains_key(field)
                        && let Some(preview) =
                            garyx_models::message_preview::last_message_preview_for_role(
                                legacy_messages.iter(),
                                role,
                            )
                    {
                        object.insert(field.to_owned(), Value::String(preview));
                    }
                }
                // Legacy task-body fallback (#TASK-1864 D5): tasks that
                // predate task.body get it from their seeded first user
                // message, one time, here.
                if let Some(task) = object.get_mut("task").and_then(Value::as_object_mut)
                    && task
                        .get("body")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_none()
                    && let Some(first_user) = legacy_messages.iter().find_map(|message| {
                        if message.get("role").and_then(Value::as_str) != Some("user") {
                            return None;
                        }
                        message
                            .get("content")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                    })
                {
                    task.insert("body".to_owned(), Value::String(first_user));
                }
            }
        }

        // write_record strips the snapshot and derives projections in one
        // transaction.
        sqlite_store.write_record(key, data).await;
        summary.imported += 1;
    }

    if let Err(error) = garyx_db.record_projection_state(
        THREAD_RECORDS_IMPORT_NAME,
        THREAD_RECORDS_IMPORT_VERSION,
        summary.source_keys,
    ) {
        warn!(error = %error, "failed to record thread record import state");
    }
    tracing::info!(
        source_keys = summary.source_keys,
        imported = summary.imported,
        skipped = summary.skipped,
        transcripts_backfilled = summary.transcripts_backfilled,
        "thread record import completed"
    );
    summary
}

/// Contract suite run against every ThreadStore implementation
/// (File / InMemory / Sqlite on memory and file databases): get/set/
/// update/delete/list_keys/exists must agree (#TASK-1864 batch 2).
#[cfg(test)]
mod contract_tests {
    use garyx_models::Principal;
    use garyx_router::{
        CreateTaskInput, FileThreadStore, InMemoryTaskCounterStore, InMemoryThreadStore, TaskService,
    };
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
    async fn cleared_import_state_forces_a_reimport() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let transcript_store = Arc::new(ThreadTranscriptStore::memory());
        let sqlite = SqliteThreadStore::new(
            Arc::clone(&garyx_db),
            Arc::clone(&transcript_store),
            Arc::new(AlwaysActiveRunProbe),
        );
        let source: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        source
            .set("thread::rollback", json!({"thread_id": "thread::rollback", "label": "v1"}))
            .await;

        let summary =
            import_thread_records_if_needed(&garyx_db, &source, &sqlite, &transcript_store).await;
        assert_eq!(summary.imported, 1);

        // A manual recovery rewrites the archive under the same key count…
        source
            .set("thread::rollback", json!({"thread_id": "thread::rollback", "label": "v2"}))
            .await;
        // …and clearing the import-state row (the manual recovery step)
        // forces the re-import (#TASK-1901: same key count must not skip it).
        assert!(
            garyx_db
                .clear_projection_state(THREAD_RECORDS_IMPORT_NAME)
                .expect("clear state")
        );

        let summary =
            import_thread_records_if_needed(&garyx_db, &source, &sqlite, &transcript_store).await;
        assert_eq!(summary.imported, 1, "cleared state must force a re-import");
        assert_eq!(
            sqlite.get("thread::rollback").await.expect("record")["label"],
            "v2",
            "the re-import must pick up the rollback write"
        );
    }

    #[tokio::test]
    async fn boot_import_migrates_the_archive_once() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let transcript_store = Arc::new(ThreadTranscriptStore::memory());
        let sqlite = SqliteThreadStore::new(
            Arc::clone(&garyx_db),
            Arc::clone(&transcript_store),
            Arc::new(AlwaysActiveRunProbe),
        );
        let source: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());

        // A pre-transcript thread: legacy messages, a task without a body.
        source
            .set(
                "thread::legacy",
                json!({
                    "thread_id": "thread::legacy",
                    "updated_at": "2026-07-01T00:00:00Z",
                    "history": {"message_count": 2},
                    "messages": [
                        {"role": "user", "content": "legacy question"},
                        {"role": "assistant", "content": "legacy answer"},
                    ],
                    "task": {"number": 42, "status": "done", "title": "Legacy task"},
                }),
            )
            .await;
        // A thread that already has a transcript: no backfill.
        source
            .set(
                "thread::modern",
                json!({
                    "thread_id": "thread::modern",
                    "messages": [{"role": "user", "content": "already transcribed"}],
                }),
            )
            .await;
        transcript_store
            .append_committed_messages(
                "thread::modern",
                Some("run-1"),
                &[json!({"role": "user", "content": "already transcribed"})],
            )
            .await
            .expect("seed transcript");
        // Non-thread keys import as plain records.
        source
            .set("meta::known_channel_endpoints", json!({"endpoints": []}))
            .await;

        let summary =
            import_thread_records_if_needed(&garyx_db, &source, &sqlite, &transcript_store).await;
        assert_eq!(summary.source_keys, 3);
        assert_eq!(summary.imported, 3);
        assert_eq!(summary.transcripts_backfilled, 1, "only the pre-transcript thread");

        // The legacy thread: snapshot stripped, transcript rebuilt, preview
        // and task body seeded, projections derived.
        let record = sqlite.get("thread::legacy").await.expect("imported record");
        assert!(record.get("messages").is_none());
        assert_eq!(record["last_user_preview"], "legacy question");
        assert_eq!(record["last_assistant_preview"], "legacy answer");
        assert_eq!(record["task"]["body"], "legacy question");
        assert!(transcript_store.exists("thread::legacy").await);
        let tail = transcript_store
            .provider_session_tail("thread::legacy", 10)
            .await
            .expect("tail");
        assert_eq!(tail.len(), 2);
        assert!(
            garyx_db
                .list_recent_threads(10, 0)
                .expect("recent")
                .iter()
                .any(|row| row.thread_id == "thread::legacy"),
            "projections must be derived during import"
        );
        assert!(sqlite.get("meta::known_channel_endpoints").await.is_some());

        // Second run is a no-op: the migration state row gates it.
        let summary =
            import_thread_records_if_needed(&garyx_db, &source, &sqlite, &transcript_store).await;
        assert_eq!(summary.imported, 0);
        assert_eq!(summary.source_keys, 3);
    }

    #[tokio::test]
    async fn assembly_migrates_task_kind_only_after_boot_import() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let transcript_store = Arc::new(ThreadTranscriptStore::memory());
        let source: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let task_service = TaskService::new(
            Arc::clone(&source),
            Arc::new(InMemoryTaskCounterStore::new()),
        );
        let (thread_id, _) = task_service
            .create_task(CreateTaskInput {
                title: Some("Imported legacy task".to_owned()),
                body: Some("Verify startup ordering.".to_owned()),
                assignee: None,
                notification_target: None,
                source: None,
                executor: None,
                start: false,
                actor: Some(Principal::Agent {
                    agent_id: "test-agent".to_owned(),
                }),
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .expect("create source task");
        let mut legacy = source.get(&thread_id).await.expect("source record");
        legacy
            .as_object_mut()
            .expect("record object")
            .remove("thread_kind");
        source.set(&thread_id, legacy).await;

        let bridge = Arc::new(garyx_bridge::MultiProviderBridge::new());
        let store = assemble_sqlite_thread_store(
            Arc::clone(&garyx_db),
            Arc::clone(&transcript_store),
            &bridge,
            source,
        )
        .await
        .expect("assemble store");

        let imported = store.get(&thread_id).await.expect("imported task");
        assert_eq!(imported["thread_kind"], "task");
        let recent = garyx_db
            .list_recent_threads(10, 0)
            .expect("recent rows")
            .into_iter()
            .find(|row| row.thread_id == thread_id)
            .expect("recent task row");
        assert_eq!(recent.thread_type, "task");
        let meta = garyx_db
            .list_thread_meta()
            .expect("meta rows")
            .into_iter()
            .find(|row| row.thread_id == thread_id)
            .expect("meta task row");
        assert_eq!(meta.thread_type, "task");
        assert!(
            garyx_db
                .projection_state_exists(
                    crate::garyx_db::RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                    1,
                )
                .expect("migration marker")
        );
    }

    #[tokio::test]
    async fn boot_import_discards_retired_workflow_records() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let transcript_store = Arc::new(ThreadTranscriptStore::memory());
        let sqlite = SqliteThreadStore::new(
            Arc::clone(&garyx_db),
            Arc::clone(&transcript_store),
            Arc::new(AlwaysActiveRunProbe),
        );
        let source: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());

        source
            .set(
                "thread::legacy-workflow-task",
                json!({
                    "thread_id": "thread::legacy-workflow-task",
                    "task": {
                        "executor": {"type": "workflow", "workflow_id": "unit"}
                    },
                    "messages": [{"role": "user", "content": "retired task"}],
                }),
            )
            .await;
        source
            .set(
                "thread::legacy-workflow-child",
                json!({
                    "thread_id": "thread::legacy-workflow-child",
                    "source": "workflow",
                    "workflow_child_run_id": "child::legacy",
                    "messages": [{"role": "assistant", "content": "retired child"}],
                }),
            )
            .await;
        source
            .set(
                "thread::ordinary",
                json!({
                    "thread_id": "thread::ordinary",
                    "label": "Discuss the ordinary deployment workflow",
                }),
            )
            .await;
        transcript_store
            .append_committed_messages(
                "thread::legacy-workflow-task",
                Some("run::legacy"),
                &[json!({"role": "user", "content": "stale transcript"})],
            )
            .await
            .expect("seed retired transcript");

        let summary =
            import_thread_records_if_needed(&garyx_db, &source, &sqlite, &transcript_store).await;
        assert_eq!(summary.source_keys, 3);
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.skipped, 2);
        assert!(sqlite.get("thread::legacy-workflow-task").await.is_none());
        assert!(sqlite.get("thread::legacy-workflow-child").await.is_none());
        assert!(sqlite.get("thread::ordinary").await.is_some());
        assert!(
            !transcript_store
                .exists("thread::legacy-workflow-task")
                .await
        );
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
                    // Legacy snapshot arriving through the boot-import
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
