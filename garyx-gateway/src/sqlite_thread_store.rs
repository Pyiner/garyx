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
use garyx_router::{
    ThreadPatchResult, ThreadRecordPatch, ThreadRunCoordinator, ThreadStore, ThreadStoreDomains,
    ThreadStoreError, ThreadTerminalState, ThreadTranscriptStore,
    ensure_channel_bindings_unchanged, is_thread_key,
};
use serde_json::Value;

use crate::garyx_db::{
    CreateIntentKey, DispatchAdmissionKey, DispatchAdmissionRecord, DispatchOutcome, GaryxDbResult,
    GaryxDbService, NewDispatchAdmission, ThreadRecordProjections,
};
use crate::recent_thread_projection::{
    ActiveRunProbe, recent_thread_draft_from_thread_data_with_active_run, resolve_active_run_id,
};
use crate::task_projection::task_projection_draft_from_thread_data;
use crate::thread_meta_projection::thread_meta_projection_from_thread_data_with_active_run;
use crate::thread_record_normalization::strip_retired_recent_exclusion_fields;
use garyx_router::is_hidden_thread_value;

/// Remove fields that must never reach the truth table. This is the common
/// set/patch/atomic-merge choke point, so legacy clients cannot recreate
/// retired canonical state through a different write path.
pub(crate) fn strip_retired_record_fields(data: &mut Value) {
    if let Some(object) = data.as_object_mut() {
        object.remove("messages");
    }
    strip_retired_recent_exclusion_fields(data);
}

pub(crate) struct SqliteThreadStore {
    garyx_db: Arc<GaryxDbService>,
    transcript_store: Arc<ThreadTranscriptStore>,
    active_run_probe: Arc<dyn ActiveRunProbe>,
    /// SQL read seams over the projections this store derives in the same
    /// transaction as every record write. Exposed through the
    /// `ThreadStore` accessor methods so their lifetime is tied to the
    /// store itself — no process-global registry.
    endpoint_projection: Arc<dyn garyx_router::ChannelEndpointProjection>,
    task_projection: Arc<dyn garyx_router::tasks::TaskProjectionReader>,
    /// Per-key locks serializing read-merge-write cycles and the projection
    /// derivation for one key, so concurrent writes to the same thread
    /// cannot interleave (folded in from RecentThreadProjectingStore).
    key_locks: StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    run_coordinator: Arc<ThreadRunCoordinator>,
}

/// Typed witness for the runtime's SQLite thread store.
///
/// The public thread-store view intentionally remains the backend-agnostic
/// [`ThreadStore`] trait object. Keeping this handle alongside that view lets
/// the composition root prove that durable admission and thread-record writes
/// share this exact SQLite instance instead of trying to recover the concrete
/// backend after trait-object coercion.
#[derive(Clone)]
pub struct SqliteThreadStoreHandle {
    store: Arc<SqliteThreadStore>,
}

impl SqliteThreadStoreHandle {
    pub fn thread_store(&self) -> Arc<dyn ThreadStore> {
        self.store.clone()
    }

    pub(crate) fn concrete_store(&self) -> Arc<SqliteThreadStore> {
        self.store.clone()
    }
}

pub(crate) struct AtomicCreateDispatchLedger {
    pub key: DispatchAdmissionKey,
    pub request_fingerprint: String,
    pub requested_run_id: String,
    pub effective_run_id: String,
    pub pending_input_id: Option<String>,
    pub outcome: DispatchOutcome,
    pub attachment_claims: Vec<crate::garyx_db::PromptAttachmentClaim>,
}

pub(crate) struct AtomicCreateCommit {
    pub create_key: CreateIntentKey,
    pub create_request_fingerprint: String,
    pub target_thread_id: String,
    pub target_data: Value,
    pub merges: Vec<garyx_router::AtomicRecordMerge>,
    pub dispatch: Option<AtomicCreateDispatchLedger>,
}

pub(crate) struct AtomicExistingDispatchCommit {
    pub target_thread_id: String,
    pub target_patch: ThreadRecordPatch,
    pub merges: Vec<garyx_router::AtomicRecordMerge>,
    pub dispatch: AtomicCreateDispatchLedger,
}

impl SqliteThreadStore {
    pub(crate) fn new(
        garyx_db: Arc<GaryxDbService>,
        transcript_store: Arc<ThreadTranscriptStore>,
        active_run_probe: Arc<dyn ActiveRunProbe>,
    ) -> Self {
        Self {
            endpoint_projection: Arc::new(
                crate::endpoint_projection::SqlChannelEndpointProjection::new(garyx_db.clone()),
            ),
            task_projection: Arc::new(crate::task_projection::SqlTaskProjectionReader::new(
                garyx_db.clone(),
            )),
            garyx_db,
            transcript_store,
            active_run_probe,
            key_locks: StdMutex::new(HashMap::new()),
            run_coordinator: Arc::new(ThreadRunCoordinator::new()),
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

    /// Derive the projection set for one thread record. Non-thread keys get
    /// `None` (record-only write).
    async fn derive_projections(&self, key: &str, data: &Value) -> Option<ThreadRecordProjections> {
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
        let recent = if is_hidden_thread_value(data) {
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

    async fn write_record(&self, key: &str, mut data: Value) -> Result<(), ThreadStoreError> {
        // Structural invariant of the truth table: retired fields cannot
        // survive any set/update merge, including legacy imported values.
        strip_retired_record_fields(&mut data);
        let body =
            serde_json::to_string(&data).map_err(|error| ThreadStoreError::Serialization {
                thread_id: key.to_owned(),
                message: error.to_string(),
            })?;
        let updated_at = data
            .get("updated_at")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let projections = self.derive_projections(key, &data).await;
        let garyx_db = Arc::clone(&self.garyx_db);
        let key_owned = key.to_owned();
        garyx_db
            .run_blocking(move |db| {
                db.write_thread_record_with_projections(
                    &key_owned,
                    &body,
                    updated_at.as_deref(),
                    projections,
                )
            })
            .await
            .map_err(|error| match error {
                crate::garyx_db::GaryxDbError::ThreadArchived(thread_id) => {
                    ThreadStoreError::Archived(thread_id)
                }
                other => ThreadStoreError::Backend(other.to_string()),
            })
    }

    /// Shared sorted-key write domain for atomic thread creation. The new
    /// target, optional endpoint-owner merges, every derived projection,
    /// create claim, and dispatch admission reach one SQLite commit.
    pub(crate) async fn commit_create_intent_atomic(
        &self,
        command: AtomicCreateCommit,
    ) -> Result<(), ThreadStoreError> {
        let mut keys = command
            .merges
            .iter()
            .map(|entry| entry.thread_id().to_owned())
            .chain(std::iter::once(command.target_thread_id.clone()))
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys.dedup();
        let locks = keys
            .iter()
            .map(|key| self.key_lock(key))
            .collect::<Vec<_>>();
        let mut guards = Vec::with_capacity(locks.len());
        for lock in &locks {
            guards.push(lock.lock().await);
        }

        let mut writes = Vec::with_capacity(keys.len());
        for key in keys {
            let mut data = if key == command.target_thread_id {
                if self.get(&key).await?.is_some() {
                    return Err(ThreadStoreError::Backend(format!(
                        "reserved thread id already exists: {key}"
                    )));
                }
                command.target_data.clone()
            } else {
                let create_if_missing = command
                    .merges
                    .iter()
                    .filter(|entry| entry.thread_id() == key)
                    .all(|entry| entry.create_if_missing());
                match self.get(&key).await? {
                    Some(data) => data,
                    None if create_if_missing => Value::Object(serde_json::Map::new()),
                    None => return Err(ThreadStoreError::NotFound(key)),
                }
            };
            for entry in command
                .merges
                .iter()
                .filter(|entry| entry.thread_id() == key)
            {
                if let (Some(target), Some(fields)) =
                    (data.as_object_mut(), entry.fields().as_object())
                {
                    for (field, value) in fields {
                        target.insert(field.clone(), value.clone());
                    }
                }
            }
            strip_retired_record_fields(&mut data);
            let body =
                serde_json::to_string(&data).map_err(|error| ThreadStoreError::Serialization {
                    thread_id: key.clone(),
                    message: error.to_string(),
                })?;
            let updated_at = data
                .get("updated_at")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let projections = self.derive_projections(&key, &data).await;
            writes.push(crate::garyx_db::ThreadRecordWrite {
                key,
                body,
                updated_at,
                projections,
            });
        }

        let db = Arc::clone(&self.garyx_db);
        let create_key = command.create_key;
        let create_fingerprint = command.create_request_fingerprint;
        let target_thread_id = command.target_thread_id;
        let dispatch = command.dispatch;
        db.run_blocking(move |db| {
            let dispatch_input = dispatch.as_ref().map(|dispatch| NewDispatchAdmission {
                key: &dispatch.key,
                request_fingerprint: &dispatch.request_fingerprint,
                requested_run_id: Some(&dispatch.requested_run_id),
                effective_run_id: Some(&dispatch.effective_run_id),
                pending_input_id: dispatch.pending_input_id.as_deref(),
                outcome: Some(dispatch.outcome),
            });
            db.commit_create_intent_records(
                &create_key,
                &create_fingerprint,
                &target_thread_id,
                writes,
                dispatch_input,
                dispatch
                    .as_ref()
                    .map(|dispatch| dispatch.attachment_claims.as_slice())
                    .unwrap_or_default(),
            )
        })
        .await
        .map_err(|error| match error {
            crate::garyx_db::GaryxDbError::ThreadArchived(thread_id) => {
                ThreadStoreError::Archived(thread_id)
            }
            other => ThreadStoreError::Backend(other.to_string()),
        })?;
        drop(guards);
        Ok(())
    }

    /// Shared sorted-key admission domain for an existing thread. Prepared
    /// top-level changes, an optional endpoint mutation batch, projections,
    /// the ledger row, and attachment claims have one commit point.
    pub(crate) async fn commit_existing_dispatch_atomic(
        &self,
        command: AtomicExistingDispatchCommit,
    ) -> Result<DispatchAdmissionRecord, ThreadStoreError> {
        let mut keys = command
            .merges
            .iter()
            .map(|entry| entry.thread_id().to_owned())
            .chain(std::iter::once(command.target_thread_id.clone()))
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys.dedup();
        let locks = keys
            .iter()
            .map(|key| self.key_lock(key))
            .collect::<Vec<_>>();
        let mut guards = Vec::with_capacity(locks.len());
        for lock in &locks {
            guards.push(lock.lock().await);
        }

        let mut writes = Vec::with_capacity(keys.len());
        for key in keys {
            let create_if_missing = command
                .merges
                .iter()
                .filter(|entry| entry.thread_id() == key)
                .all(|entry| entry.create_if_missing());
            let mut data = match self.get(&key).await? {
                Some(data) => data,
                None if key != command.target_thread_id && create_if_missing => {
                    Value::Object(serde_json::Map::new())
                }
                None => return Err(ThreadStoreError::NotFound(key)),
            };
            if key == command.target_thread_id {
                command.target_patch.apply_to(&mut data)?;
            }
            for entry in command
                .merges
                .iter()
                .filter(|entry| entry.thread_id() == key)
            {
                if let (Some(target), Some(fields)) =
                    (data.as_object_mut(), entry.fields().as_object())
                {
                    for (field, value) in fields {
                        target.insert(field.clone(), value.clone());
                    }
                }
            }
            strip_retired_record_fields(&mut data);
            let body =
                serde_json::to_string(&data).map_err(|error| ThreadStoreError::Serialization {
                    thread_id: key.clone(),
                    message: error.to_string(),
                })?;
            let updated_at = data
                .get("updated_at")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let projections = self.derive_projections(&key, &data).await;
            writes.push(crate::garyx_db::ThreadRecordWrite {
                key,
                body,
                updated_at,
                projections,
            });
        }

        let db = Arc::clone(&self.garyx_db);
        let dispatch = command.dispatch;
        let record = db
            .run_blocking(move |db| {
                db.insert_dispatch_admission_with_records_for_existing_thread(
                    NewDispatchAdmission {
                        key: &dispatch.key,
                        request_fingerprint: &dispatch.request_fingerprint,
                        requested_run_id: Some(&dispatch.requested_run_id),
                        effective_run_id: Some(&dispatch.effective_run_id),
                        pending_input_id: dispatch.pending_input_id.as_deref(),
                        outcome: Some(dispatch.outcome),
                    },
                    writes,
                    &dispatch.attachment_claims,
                )
            })
            .await
            .map_err(|error| match error {
                crate::garyx_db::GaryxDbError::ThreadArchived(thread_id) => {
                    ThreadStoreError::Archived(thread_id)
                }
                other => ThreadStoreError::Backend(other.to_string()),
            })?;
        drop(guards);
        Ok(record)
    }
}

impl ThreadStoreDomains for SqliteThreadStore {
    fn run_coordinator(&self) -> Arc<ThreadRunCoordinator> {
        self.run_coordinator.clone()
    }

    fn channel_endpoint_projection(
        &self,
    ) -> Option<Arc<dyn garyx_router::ChannelEndpointProjection>> {
        Some(self.endpoint_projection.clone())
    }

    fn task_projection(&self) -> Option<Arc<dyn garyx_router::tasks::TaskProjectionReader>> {
        Some(self.task_projection.clone())
    }
}

#[async_trait]
impl ThreadStore for SqliteThreadStore {
    async fn terminal_state(
        &self,
        thread_id: &str,
    ) -> Result<Option<ThreadTerminalState>, ThreadStoreError> {
        let garyx_db = Arc::clone(&self.garyx_db);
        let key = thread_id.to_owned();
        garyx_db
            .run_blocking(move |db| db.thread_terminal_state(&key))
            .await
            .map_err(|error| ThreadStoreError::Backend(error.to_string()))
    }

    async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
        let garyx_db = Arc::clone(&self.garyx_db);
        let key = thread_id.to_owned();
        let Some(body) = garyx_db
            .run_blocking(move |db| db.get_thread_record_body(&key))
            .await
            .map_err(|error| ThreadStoreError::Backend(error.to_string()))?
        else {
            return Ok(None);
        };
        serde_json::from_str(&body)
            .map(Some)
            .map_err(|error| ThreadStoreError::Serialization {
                thread_id: thread_id.to_owned(),
                message: error.to_string(),
            })
    }

    async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError> {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        if let Some(current) = self.get(thread_id).await? {
            ensure_channel_bindings_unchanged(thread_id, &current, &data)?;
        }
        // Archived threads reject writes with Err(Archived): the tombstone
        // check runs inside the record-write transaction itself, so a
        // racing archive can never be overtaken (#TASK-2099; rejection
        // semantics from review #TASK-1901).
        self.write_record(thread_id, data).await
    }

    async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
        let lock = self.key_lock(thread_id);
        let garyx_db = Arc::clone(&self.garyx_db);
        let key = thread_id.to_owned();
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
        let admission = reservation.storage_delete_admission();
        let _guard = lock.lock().await;
        let removed = garyx_db
            .run_blocking(move |db| db.delete_thread_record_with_projections(&key, admission))
            .await
            .map_err(|error| ThreadStoreError::Backend(error.to_string()))?;
        if removed || prior.is_some() {
            reservation.settle_committed(Some(ThreadTerminalState::Deleted));
        } else {
            reservation.settle_decision(None);
        }
        Ok(removed)
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
        let garyx_db = Arc::clone(&self.garyx_db);
        let prefix = prefix.map(ToOwned::to_owned);
        garyx_db
            .run_blocking(move |db| db.list_thread_record_keys(prefix.as_deref()))
            .await
            .map_err(|error| ThreadStoreError::Backend(error.to_string()))
    }

    async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
        let garyx_db = Arc::clone(&self.garyx_db);
        let key = thread_id.to_owned();
        garyx_db
            .run_blocking(move |db| db.thread_record_exists(&key))
            .await
            .map_err(|error| ThreadStoreError::Backend(error.to_string()))
    }

    async fn count_keys(&self, prefix: Option<&str>) -> Result<usize, ThreadStoreError> {
        let garyx_db = Arc::clone(&self.garyx_db);
        let prefix = prefix.map(ToOwned::to_owned);
        garyx_db
            .run_blocking(move |db| db.count_thread_record_keys(prefix.as_deref()))
            .await
            .map_err(|error| ThreadStoreError::Backend(error.to_string()))
    }

    async fn patch(
        &self,
        thread_id: &str,
        patch: ThreadRecordPatch,
    ) -> Result<ThreadPatchResult, ThreadStoreError> {
        let lock = self.key_lock(thread_id);
        let _guard = lock.lock().await;
        if self.terminal_state(thread_id).await?.is_some() {
            return Err(ThreadStoreError::Archived(thread_id.to_owned()));
        }
        let mut data = self
            .get(thread_id)
            .await?
            .ok_or_else(|| ThreadStoreError::NotFound(thread_id.to_owned()))?;
        if !patch.apply_to(&mut data)? {
            return Ok(ThreadPatchResult::Unchanged);
        }
        self.write_record(thread_id, data).await?;
        Ok(ThreadPatchResult::Applied)
    }

    async fn update_many_atomic(
        &self,
        entries: Vec<garyx_router::AtomicRecordMerge>,
    ) -> Result<(), ThreadStoreError> {
        // All-or-nothing multi-record merge (#TASK-2099 root final
        // review): every per-key lock is held in sorted order across the
        // read-merge-derive-write cycle (no writer can interleave, no
        // lock-order deadlock), and every record plus its derived
        // projections commit in ONE SQLite transaction — a failure on any
        // record rolls the whole mutation back.
        let mut keys: Vec<&str> = entries.iter().map(|entry| entry.thread_id()).collect();
        keys.sort_unstable();
        keys.dedup();
        let locks: Vec<_> = keys.iter().map(|key| self.key_lock(key)).collect();
        let mut guards = Vec::with_capacity(locks.len());
        for lock in &locks {
            guards.push(lock.lock().await);
        }

        let mut writes = Vec::with_capacity(entries.len());
        for entry in entries {
            let (key, fields, create_if_missing) = entry.into_parts();
            if self.terminal_state(&key).await?.is_some() {
                return Err(ThreadStoreError::Archived(key));
            }
            let current = self.get(&key).await?;
            let mut data = match current {
                Some(data) => data,
                None if create_if_missing => Value::Object(serde_json::Map::new()),
                None => return Err(ThreadStoreError::NotFound(key)),
            };
            if let (Some(target), Some(updates)) = (data.as_object_mut(), fields.as_object()) {
                for (field, value) in updates {
                    target.insert(field.clone(), value.clone());
                }
            }
            strip_retired_record_fields(&mut data);
            let body =
                serde_json::to_string(&data).map_err(|error| ThreadStoreError::Serialization {
                    thread_id: key.clone(),
                    message: error.to_string(),
                })?;
            let updated_at = data
                .get("updated_at")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let projections = self.derive_projections(&key, &data).await;
            writes.push(crate::garyx_db::ThreadRecordWrite {
                key,
                body,
                updated_at,
                projections,
            });
        }

        let garyx_db = Arc::clone(&self.garyx_db);
        garyx_db
            .run_blocking(move |db| db.write_thread_records_with_projections_atomic(writes))
            .await
            .map_err(|error| match error {
                crate::garyx_db::GaryxDbError::ThreadArchived(thread_id) => {
                    ThreadStoreError::Archived(thread_id)
                }
                other => ThreadStoreError::Backend(other.to_string()),
            })
    }
}

/// Pure constructor for the runtime's only thread-record backend. Legacy
/// archive import and SQL cutovers are separate boot phases owned by the
/// runtime assembler and `AppStateBuilder`, respectively.
pub fn assemble_sqlite_thread_store(
    garyx_db: Arc<GaryxDbService>,
    transcript_store: Arc<ThreadTranscriptStore>,
    bridge: &Arc<garyx_bridge::MultiProviderBridge>,
) -> GaryxDbResult<SqliteThreadStoreHandle> {
    let probe: Arc<dyn ActiveRunProbe> = Arc::new(
        crate::recent_thread_projection::BridgeActiveRunProbe::new(Arc::downgrade(bridge)),
    );
    let sqlite_store = SqliteThreadStore::new(garyx_db, transcript_store, probe);
    Ok(SqliteThreadStoreHandle {
        store: Arc::new(sqlite_store),
    })
}

/// The SQLite backend runs the executable store contract published by
/// garyx-router (the trait crate; the in-memory reference backend runs the
/// same suite there), plus SQLite-specific truth-table behavior
/// (#TASK-1864 batch 2).
#[cfg(test)]
mod contract_tests {
    use garyx_router::store_contract;
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

    #[tokio::test]
    async fn sqlite_store_satisfies_the_contract_on_a_memory_database() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let store = sqlite_store(garyx_db);
        store_contract::run_thread_store_contract(&store).await;
    }

    #[tokio::test]
    async fn sqlite_store_satisfies_the_contract_on_a_file_database() {
        // File databases exercise the dedicated reader connection.
        let dir = tempfile::tempdir().expect("temp dir");
        let garyx_db =
            Arc::new(GaryxDbService::open(dir.path().join("garyx-db.sqlite3")).expect("db opens"));
        let store = sqlite_store(garyx_db);
        store_contract::run_thread_store_contract(&store).await;
    }

    #[tokio::test]
    async fn sqlite_store_protects_endpoint_fields_and_applies_patches() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        store_contract::run_patch_and_protected_field_contract(&sqlite_store(garyx_db)).await;
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
            .await
            .unwrap();

        let read = store.get(thread_id).await.unwrap().expect("read back");
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
            .await
            .unwrap();
        assert!(
            !garyx_db
                .list_recent_threads(10, 0)
                .expect("list recent")
                .iter()
                .any(|row| row.thread_id == thread_id)
        );
        assert!(store.exists(thread_id).await.unwrap());

        // Product archive is one transaction: tombstone written, record
        // and projections deleted together. A later write is rejected by
        // the in-transaction tombstone check with a typed error — a
        // dropped write must never look persisted to the caller
        // (#TASK-2099).
        assert!(garyx_db.archive_thread_record(thread_id).expect("archive"));
        assert_eq!(store.get(thread_id).await.unwrap(), None);
        let rejected = store
            .set(
                thread_id,
                json!({"thread_id": thread_id, "label": "after-archive"}),
            )
            .await;
        assert!(
            matches!(rejected, Err(ThreadStoreError::Archived(_))),
            "archived write must surface as ThreadStoreError::Archived, got {rejected:?}"
        );
        assert_eq!(
            store.get(thread_id).await.unwrap(),
            None,
            "the rejected write must not recreate the archived record"
        );
    }

    #[tokio::test]
    async fn sqlite_store_strips_all_retired_recent_exclusion_paths_on_every_write_shape() {
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let store = sqlite_store(Arc::clone(&garyx_db));
        let thread_id = "thread::retired-exclusion-input";

        store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "automation_thread_mode": "generated_thread",
                    "exclude_from_recent": true,
                    "excludeFromRecent": true,
                    "metadata": {
                        "automation_thread_mode": "generated_thread",
                        "exclude_from_recent": "yes",
                        "excludeFromRecent": true
                    }
                }),
            )
            .await
            .expect("legacy create payload");
        let created = store.get(thread_id).await.unwrap().unwrap();
        assert_retired_recent_exclusion_paths_absent(&created);

        let mut legacy_patch_fields = serde_json::Map::new();
        legacy_patch_fields.insert("exclude_from_recent".to_owned(), json!(true));
        legacy_patch_fields.insert("excludeFromRecent".to_owned(), json!(true));
        legacy_patch_fields.insert(
            "metadata".to_owned(),
            json!({
                "source": "legacy-client",
                "exclude_from_recent": true,
                "excludeFromRecent": true
            }),
        );
        store
            .patch(
                thread_id,
                ThreadRecordPatch::new(legacy_patch_fields, std::collections::BTreeSet::new())
                    .expect("legacy patch builds"),
            )
            .await
            .expect("legacy patch payload");
        let updated = store.get(thread_id).await.unwrap().unwrap();
        assert_retired_recent_exclusion_paths_absent(&updated);
        assert_eq!(updated["metadata"]["source"], "legacy-client");

        store
            .update_many_atomic(vec![
                garyx_router::AtomicRecordMerge::new(
                    thread_id,
                    json!({
                        "exclude_from_recent": true,
                        "excludeFromRecent": true,
                        "metadata": {
                            "source": "legacy-atomic-client",
                            "exclude_from_recent": true,
                            "excludeFromRecent": true
                        }
                    }),
                    false,
                )
                .expect("plain merge is valid"),
            ])
            .await
            .expect("legacy atomic merge payload");
        let atomically_updated = store.get(thread_id).await.unwrap().unwrap();
        assert_retired_recent_exclusion_paths_absent(&atomically_updated);
        assert_eq!(
            atomically_updated["metadata"]["source"],
            "legacy-atomic-client"
        );
        assert!(
            garyx_db
                .list_recent_threads(10, 0)
                .unwrap()
                .iter()
                .any(|row| row.thread_id == thread_id),
            "generated automation mode remains ordinary recent membership"
        );
    }

    fn assert_retired_recent_exclusion_paths_absent(data: &Value) {
        assert!(data.get("exclude_from_recent").is_none());
        assert!(data.get("excludeFromRecent").is_none());
        let metadata = data.get("metadata").and_then(Value::as_object).unwrap();
        assert!(metadata.get("exclude_from_recent").is_none());
        assert!(metadata.get("excludeFromRecent").is_none());
    }
}
