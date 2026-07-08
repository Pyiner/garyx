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

/// Remove fields that must never reach the truth table or its file
/// mirror: the retired `messages` snapshot (#TASK-1864 batch 1).
pub(crate) fn strip_retired_record_fields(data: &mut Value) {
    if let Some(object) = data.as_object_mut() {
        object.remove("messages");
    }
}

/// Thread-record storage backend (#TASK-1864 batch 2, D8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStoreBackend {
    /// JSON archive is the truth (default; current production shape).
    File,
    /// SQLite truth with a best-effort file mirror for hot rollback.
    Sqlite,
    /// SQLite truth, no mirror.
    SqliteOnly,
}

/// Resolve the configured backend; `GARYX_THREAD_STORE` overrides config
/// so a rollback never requires editing the config file. Unknown values
/// fall back to `File` with a warning.
pub fn resolve_thread_store_backend(config: &garyx_models::config::GaryxConfig) -> ThreadStoreBackend {
    let raw = std::env::var("GARYX_THREAD_STORE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| config.sessions.thread_store.clone());
    match raw.as_deref().map(str::trim) {
        None | Some("") | Some("file") => ThreadStoreBackend::File,
        Some("sqlite") => ThreadStoreBackend::Sqlite,
        Some("sqlite-only") => ThreadStoreBackend::SqliteOnly,
        Some(other) => {
            warn!(
                value = other,
                "unknown sessions.thread_store backend; falling back to file"
            );
            ThreadStoreBackend::File
        }
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

    /// Trait `set` with an acceptance signal: `false` means the write was
    /// rejected (archived thread) — the mirror layer must not write either.
    pub(crate) async fn set_accepted(&self, thread_id: &str, data: Value) -> bool {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        if self.reject_archived_thread_write(thread_id).await {
            return false;
        }
        self.write_record(thread_id, data).await;
        true
    }

    async fn write_record(&self, key: &str, mut data: Value) {
        // Structural invariant of the truth table: bodies never carry the
        // retired `messages` snapshot (#TASK-1864). Batch 1 removed every
        // producer; this strip guards legacy values arriving through
        // import/mirror paths.
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
        self.set_accepted(thread_id, data).await;
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
        self.update_accepted(thread_id, updates).await.map(|_| ())
    }
}

impl SqliteThreadStore {
    /// Trait `update` with an acceptance signal: `Ok(false)` means the
    /// write was rejected (archived thread) — the mirror layer must not
    /// write either (review #TASK-1901).
    pub(crate) async fn update_accepted(
        &self,
        thread_id: &str,
        updates: Value,
    ) -> Result<bool, ThreadStoreError> {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        if self.reject_archived_thread_write(thread_id).await {
            return Ok(false);
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
        Ok(true)
    }
}

/// Dual-write transition store (#TASK-1864 batch 2, D8 `sqlite` mode):
/// SQL is the truth — every write commits to the SqliteThreadStore first,
/// then mirrors best-effort to the file archive so `GARYX_THREAD_STORE=file`
/// rollback stays hot. Reads serve from SQL only; a sampled dual-read
/// comparison counts divergences for switchover confidence.
pub(crate) struct MirroredThreadStore {
    primary: Arc<SqliteThreadStore>,
    mirror: Arc<dyn ThreadStore>,
    /// Per-key locks spanning the SQL commit *and* the mirror write, so
    /// concurrent same-key writes cannot land on the mirror out of order
    /// (review #TASK-1901: SQL v1→v2 with the mirror finishing v2→v1 would
    /// exceed the D8 "one write behind" rollback window).
    key_locks: StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    stats: Arc<MirrorCompareStats>,
}

#[derive(Default)]
struct MirrorCompareStats {
    reads: std::sync::atomic::AtomicU64,
    comparisons: std::sync::atomic::AtomicU64,
    divergences: std::sync::atomic::AtomicU64,
}

const MIRROR_COMPARE_SAMPLE_EVERY: u64 = 64;

impl MirroredThreadStore {
    pub(crate) fn new(primary: Arc<SqliteThreadStore>, mirror: Arc<dyn ThreadStore>) -> Self {
        Self {
            primary,
            mirror,
            key_locks: StdMutex::new(HashMap::new()),
            stats: Arc::new(MirrorCompareStats::default()),
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

    /// Comparable view of a record: the retired snapshot (still present in
    /// unrewritten archive files), volatile bookkeeping timestamps, and the
    /// derived preview fields (seeded into the truth table by the boot
    /// import — mirror files only pick them up on their next rewrite, so
    /// they would read as permanent noise for low-traffic threads) are
    /// ignored. A genuine dual-write fault diverges on real fields too.
    fn comparable(mut value: Value) -> Value {
        if let Some(object) = value.as_object_mut() {
            object.remove("messages");
            object.remove("updated_at");
            object.remove(garyx_models::message_preview::LAST_USER_PREVIEW_FIELD);
            object.remove(garyx_models::message_preview::LAST_ASSISTANT_PREVIEW_FIELD);
        }
        value
    }

    /// Fire-and-forget sampled comparison: reads must never wait on the
    /// file mirror (#TASK-1903 — synchronous mirror reads on the get path
    /// stretched request latency under load).
    fn sample_compare_in_background(&self, thread_id: &str, primary_value: &Value) {
        let read_index = self
            .stats
            .reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !read_index.is_multiple_of(MIRROR_COMPARE_SAMPLE_EVERY) {
            return;
        }
        let mirror = Arc::clone(&self.mirror);
        let stats = Arc::clone(&self.stats);
        let thread_id = thread_id.to_owned();
        let primary_value = primary_value.clone();
        tokio::spawn(async move {
            let Some(mirror_value) = mirror.get(&thread_id).await else {
                // Absent mirror rows are expected until the first rewrite
                // of a record that predates the mirror; not a divergence
                // signal.
                return;
            };
            stats
                .comparisons
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if Self::comparable(primary_value) != Self::comparable(mirror_value) {
                let divergences = stats
                    .divergences
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    + 1;
                tracing::debug!(
                    thread_id = %thread_id,
                    divergences,
                    comparisons = stats.comparisons.load(std::sync::atomic::Ordering::Relaxed),
                    "thread record mirror divergence detected"
                );
            }
        });
    }
}

#[async_trait]
impl ThreadStore for MirroredThreadStore {
    async fn get(&self, thread_id: &str) -> Option<Value> {
        let value = self.primary.get(thread_id).await?;
        self.sample_compare_in_background(thread_id, &value);
        Some(value)
    }

    async fn set(&self, thread_id: &str, mut data: Value) {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        // Strip up-front so both sides persist the same shape; the mirror
        // rewrite is what strips legacy `messages` out of archive files.
        strip_retired_record_fields(&mut data);
        if self.primary.set_accepted(thread_id, data.clone()).await {
            self.mirror.set(thread_id, data).await;
        }
    }

    async fn delete(&self, thread_id: &str) -> bool {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        let removed = self.primary.delete(thread_id).await;
        self.mirror.delete(thread_id).await;
        removed
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Vec<String> {
        self.primary.list_keys(prefix).await
    }

    async fn exists(&self, thread_id: &str) -> bool {
        self.primary.exists(thread_id).await
    }

    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        // A rejected update (archived thread) must not touch the mirror
        // either (review #TASK-1901).
        if !self.primary.update_accepted(thread_id, updates).await? {
            return Ok(());
        }
        // Mirror the post-merge truth snapshot rather than re-running the
        // merge against a possibly stale archive copy.
        if let Some(merged) = self.primary.get(thread_id).await {
            self.mirror.set(thread_id, merged).await;
        }
        Ok(())
    }
}

/// Assemble the SQLite-backed thread store for runtime wiring
/// (#TASK-1864 batch 2): build the store over `garyx_db`, run the one-shot
/// boot import from the file archive when this machine has not imported
/// yet, and wrap the dual-write mirror in `sqlite` mode
/// (`mirror: Some(file store)`). The returned store is what
/// `AppStateBuilder::with_thread_store` should receive; the builder
/// deliberately does not re-wrap SQLite backends in the projecting store.
pub async fn assemble_sqlite_thread_store(
    garyx_db: Arc<GaryxDbService>,
    transcript_store: Arc<ThreadTranscriptStore>,
    bridge: &Arc<garyx_bridge::MultiProviderBridge>,
    import_source: Arc<dyn ThreadStore>,
    mirror: Option<Arc<dyn ThreadStore>>,
) -> Arc<dyn ThreadStore> {
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
    let sqlite_store = Arc::new(sqlite_store);
    match mirror {
        Some(mirror) => Arc::new(MirroredThreadStore::new(sqlite_store, mirror)),
        None => sqlite_store,
    }
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
    // on every boot, flowing the possibly one-write-behind file mirror
    // back over the SQL truth. A rollback to the file backend clears this
    // row at boot, which is the only event that must force a re-import.
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

    let mut summary = ThreadRecordImportSummary {
        source_keys: source_keys.len(),
        ..Default::default()
    };
    for key in &source_keys {
        let Some(mut data) = source.get(key).await else {
            summary.skipped += 1;
            continue;
        };
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
    async fn mirrored_store_dual_writes_and_respects_rejection() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let mirror: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let store = MirroredThreadStore::new(
            Arc::new(sqlite_store(Arc::clone(&garyx_db))),
            Arc::clone(&mirror),
        );
        let thread_id = "thread::mirrored";

        // set dual-writes, stripping legacy messages on both sides.
        store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "label": "Mirrored",
                    "messages": [{"role": "user", "content": "legacy"}],
                }),
            )
            .await;
        let primary_read = store.get(thread_id).await.expect("primary read");
        assert!(primary_read.get("messages").is_none());
        let mirror_read = mirror.get(thread_id).await.expect("mirror read");
        assert!(mirror_read.get("messages").is_none());
        assert_eq!(mirror_read["label"], "Mirrored");

        // update mirrors the post-merge truth snapshot.
        store
            .update(thread_id, json!({"label": "Merged", "extra": 1}))
            .await
            .expect("update");
        let mirror_read = mirror.get(thread_id).await.expect("mirror after update");
        assert_eq!(mirror_read["label"], "Merged");
        assert_eq!(mirror_read["extra"], 1);

        // Archived rejection: neither side takes the write.
        garyx_db.mark_thread_archived(thread_id).expect("archive");
        store
            .set(thread_id, json!({"thread_id": thread_id, "label": "after-archive"}))
            .await;
        assert_eq!(
            mirror.get(thread_id).await.expect("mirror unchanged")["label"],
            "Merged",
            "rejected writes must not reach the mirror"
        );

        // delete removes both sides.
        assert!(store.delete(thread_id).await);
        assert_eq!(mirror.get(thread_id).await, None);
    }

    /// Mirror whose first write stalls until released — reproduces the
    /// #TASK-1901 ordering race: without the mirror-spanning key lock,
    /// SQL committing v1→v2 could still leave the mirror at v1.
    struct StallingMirror {
        inner: InMemoryThreadStore,
        gate: tokio::sync::Semaphore,
        stalled_once: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl ThreadStore for StallingMirror {
        async fn get(&self, thread_id: &str) -> Option<Value> {
            self.inner.get(thread_id).await
        }
        async fn set(&self, thread_id: &str, data: Value) {
            if !self
                .stalled_once
                .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                let _permit = self.gate.acquire().await.expect("gate");
            }
            self.inner.set(thread_id, data).await;
        }
        async fn delete(&self, thread_id: &str) -> bool {
            self.inner.delete(thread_id).await
        }
        async fn list_keys(&self, prefix: Option<&str>) -> Vec<String> {
            self.inner.list_keys(prefix).await
        }
        async fn exists(&self, thread_id: &str) -> bool {
            self.inner.exists(thread_id).await
        }
        async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
            self.inner.update(thread_id, updates).await
        }
    }

    #[tokio::test]
    async fn mirrored_store_preserves_same_key_write_order_under_concurrency() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let mirror = Arc::new(StallingMirror {
            inner: InMemoryThreadStore::new(),
            gate: tokio::sync::Semaphore::new(0),
            stalled_once: std::sync::atomic::AtomicBool::new(false),
        });
        let store = Arc::new(MirroredThreadStore::new(
            Arc::new(sqlite_store(Arc::clone(&garyx_db))),
            Arc::clone(&mirror) as Arc<dyn ThreadStore>,
        ));
        let thread_id = "thread::write-order";

        // First writer's mirror write stalls at the gate; the second writer
        // must not be able to slip its mirror write in front of it.
        let first = {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                store
                    .set(thread_id, json!({"thread_id": thread_id, "generation": 1}))
                    .await;
            })
        };
        // Give the first writer time to reach the stalled mirror write.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let second = {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                store
                    .set(thread_id, json!({"thread_id": thread_id, "generation": 2}))
                    .await;
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        mirror.gate.add_permits(1);
        first.await.expect("first write");
        second.await.expect("second write");

        assert_eq!(
            mirror.get(thread_id).await.expect("mirror value")["generation"],
            2,
            "the mirror must end at the newest committed value"
        );
        assert_eq!(
            store.get(thread_id).await.expect("primary value")["generation"],
            2
        );
    }

    #[tokio::test]
    async fn mirrored_store_rejected_update_does_not_touch_the_mirror() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let mirror: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let store = MirroredThreadStore::new(
            Arc::new(sqlite_store(Arc::clone(&garyx_db))),
            Arc::clone(&mirror),
        );
        let thread_id = "thread::archived-update";

        store
            .set(thread_id, json!({"thread_id": thread_id, "label": "live"}))
            .await;
        // Simulate a mirror that is one write behind (the allowed window).
        mirror
            .set(thread_id, json!({"thread_id": thread_id, "label": "stale"}))
            .await;

        garyx_db.mark_thread_archived(thread_id).expect("archive");
        store
            .update(thread_id, json!({"label": "after-archive"}))
            .await
            .expect("rejected update reports Ok");

        assert_eq!(
            mirror.get(thread_id).await.expect("mirror unchanged")["label"],
            "stale",
            "a rejected update must not rewrite the mirror (#TASK-1901)"
        );
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

        // A file-mode rollback rewrites the record under the same key count…
        source
            .set("thread::rollback", json!({"thread_id": "thread::rollback", "label": "v2"}))
            .await;
        // …and the file-mode boot invalidates the import state
        // (#TASK-1901: same key count must not skip the re-import).
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

    #[test]
    fn mirror_comparison_ignores_retired_and_volatile_fields() {
        let primary = json!({
            "thread_id": "t",
            "label": "same",
            "updated_at": "2026-07-08T01:00:00Z",
            // Seeded by the boot import; absent from unrewritten mirrors.
            "last_user_preview": "seeded",
            "last_assistant_preview": "seeded",
        });
        let mirror = json!({
            "thread_id": "t",
            "label": "same",
            "updated_at": "2026-07-08T02:00:00Z",
            "messages": [{"role": "user", "content": "stale archive copy"}],
        });
        assert_eq!(
            MirroredThreadStore::comparable(primary),
            MirroredThreadStore::comparable(mirror)
        );
        let diverged = json!({"thread_id": "t", "label": "different"});
        assert_ne!(
            MirroredThreadStore::comparable(diverged),
            MirroredThreadStore::comparable(json!({"thread_id": "t", "label": "same"}))
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
