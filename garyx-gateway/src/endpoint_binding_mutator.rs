use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use garyx_models::config::GaryxConfig;
use garyx_router::{
    AtomicRecordMerge, ChannelBinding, EndpointBindResult, EndpointBindingMutationError,
    EndpointBindingMutator, EndpointBindingOwner, EndpointDetachResult,
    KNOWN_CHANNEL_ENDPOINTS_KEY, KnownChannelEndpoint, ThreadStore, ThreadStoreError,
    bindings_from_value, remove_binding, upsert_binding, validate_thread_accepts_bot_binding,
};
use serde_json::{Map, Value};
use tokio::sync::Mutex;

use crate::garyx_db::GaryxDbService;

pub(crate) struct SqlEndpointBindingMutator {
    thread_store: Arc<dyn ThreadStore>,
    garyx_db: Arc<GaryxDbService>,
    mutation_lock: Mutex<()>,
    binding_freezes: Arc<StdMutex<HashMap<String, u64>>>,
    next_freeze_token: AtomicU64,
}

pub(crate) enum DeleteBindingPreflight {
    InProgress,
    RejectedEnabledBinding,
    Frozen {
        guard: ThreadBindingFreezeGuard,
        enabled_channel_accounts: BTreeSet<(String, String)>,
    },
}

/// Owned fence installed in the endpoint mutator's serialization domain.
/// Drop conditionally removes only this generation, so a stale guard can
/// never thaw a replacement delete.
pub(crate) struct ThreadBindingFreezeGuard {
    freezes: Arc<StdMutex<HashMap<String, u64>>>,
    thread_id: String,
    token: u64,
}

impl Drop for ThreadBindingFreezeGuard {
    fn drop(&mut self) {
        let mut freezes = self
            .freezes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if freezes.get(&self.thread_id) == Some(&self.token) {
            freezes.remove(&self.thread_id);
        }
    }
}

impl SqlEndpointBindingMutator {
    pub(crate) fn new(thread_store: Arc<dyn ThreadStore>, garyx_db: Arc<GaryxDbService>) -> Self {
        Self {
            thread_store,
            garyx_db,
            mutation_lock: Mutex::new(()),
            binding_freezes: Arc::new(StdMutex::new(HashMap::new())),
            next_freeze_token: AtomicU64::new(0),
        }
    }

    fn thread_is_frozen(&self, thread_id: &str) -> bool {
        self.binding_freezes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .contains_key(thread_id)
    }

    /// Atomically classify authoritative bindings and, when deletion is
    /// allowed, install a thread-owned freeze before releasing the same lock
    /// used by bind/detach mutations. The enabled-account set is the caller's
    /// immutable config snapshot and is reused by the final SQLite belt.
    pub(crate) async fn preflight_and_freeze<F>(
        &self,
        thread_id: &str,
        config_snapshot: F,
    ) -> Result<DeleteBindingPreflight, EndpointBindingMutationError>
    where
        F: FnOnce() -> Arc<GaryxConfig>,
    {
        let _guard = self.mutation_lock.lock().await;
        let thread_id = thread_id.trim();

        // Existing freeze wins before record/config classification. A second
        // operation is ambiguous and must retry, never persist a new verdict.
        if self.thread_is_frozen(thread_id) {
            return Ok(DeleteBindingPreflight::InProgress);
        }

        // Config is captured only after the endpoint mutation lock is held.
        // This is the delete operation's enabled-ness linearization point.
        let config = config_snapshot();
        let enabled_channel_accounts = config
            .channels
            .plugins
            .iter()
            .flat_map(|(channel, plugin)| {
                plugin.accounts.iter().filter_map(|(account_id, account)| {
                    account
                        .enabled
                        .then(|| (channel.clone(), account_id.clone()))
                })
            })
            .collect::<BTreeSet<_>>();

        let record = self.thread_store.get(thread_id).await.map_err(|error| {
            EndpointBindingMutationError::WriteFailed {
                thread_id: thread_id.to_owned(),
                message: error.to_string(),
            }
        })?;
        let has_enabled_binding = record.as_ref().is_some_and(|record| {
            bindings_from_value(record).iter().any(|binding| {
                enabled_channel_accounts
                    .contains(&(binding.channel.clone(), binding.account_id.clone()))
            })
        });
        if has_enabled_binding {
            return Ok(DeleteBindingPreflight::RejectedEnabledBinding);
        }

        let token = self
            .next_freeze_token
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .expect("endpoint binding freeze token exhausted")
            + 1;
        self.binding_freezes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(thread_id.to_owned(), token);
        Ok(DeleteBindingPreflight::Frozen {
            guard: ThreadBindingFreezeGuard {
                freezes: Arc::clone(&self.binding_freezes),
                thread_id: thread_id.to_owned(),
                token,
            },
            enabled_channel_accounts,
        })
    }

    /// Current owner of one endpoint, resolved through the STORE'S OWN
    /// projection accessor (#TASK-2155): the lookup and the mutation
    /// writes share one truth source. The SQLite store answers with the
    /// indexed point query over the same database its writes derive
    /// into; an injected non-SQL store answers with the scan projection
    /// over itself — never an unrelated database.
    async fn projected_owner(
        &self,
        endpoint_key: &str,
    ) -> Result<Option<KnownChannelEndpoint>, EndpointBindingMutationError> {
        garyx_router::channel_endpoint_projection_for(&self.thread_store)
            .endpoint_owner(endpoint_key)
            .await
            .map_err(EndpointBindingMutationError::Projection)
    }

    async fn is_archived(&self, thread_id: &str) -> Result<bool, EndpointBindingMutationError> {
        let thread_id = thread_id.to_owned();
        self.garyx_db
            .run_blocking(move |db| db.is_thread_archived(&thread_id))
            .await
            .map_err(|error| EndpointBindingMutationError::Projection(error.to_string()))
    }

    /// The registry record merge for one binding upsert, computed from a
    /// fresh read so it can join the atomic batch instead of being a
    /// separate trailing write.
    async fn registry_merge(
        &self,
        binding: &ChannelBinding,
    ) -> Result<AtomicRecordMerge, EndpointBindingMutationError> {
        let mut registry = self
            .thread_store
            .get(KNOWN_CHANNEL_ENDPOINTS_KEY)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: KNOWN_CHANNEL_ENDPOINTS_KEY.to_owned(),
                message: error.to_string(),
            })?
            .unwrap_or_else(|| Value::Object(Map::new()));
        upsert_binding(&mut registry, binding.clone());
        Ok(AtomicRecordMerge {
            thread_id: KNOWN_CHANNEL_ENDPOINTS_KEY.to_owned(),
            fields: binding_fields_of(&registry),
            create_if_missing: true,
        })
    }

    /// Commit every record merge of one binding mutation as a single
    /// all-or-nothing write (#TASK-2099 root final review): the previous
    /// owner, the target, and the known-endpoint registry either all
    /// commit — records and projections alike — or none do, so a storage
    /// failure mid-mutation can never lose the active binding.
    async fn commit_atomic(
        &self,
        subject_thread_id: &str,
        entries: Vec<AtomicRecordMerge>,
    ) -> Result<(), EndpointBindingMutationError> {
        self.thread_store
            .update_many_atomic(entries)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: match &error {
                    ThreadStoreError::NotFound(id) | ThreadStoreError::Archived(id) => id.clone(),
                    ThreadStoreError::Serialization { thread_id, .. } => thread_id.clone(),
                    ThreadStoreError::Backend(_) => subject_thread_id.to_owned(),
                },
                message: error.to_string(),
            })
    }
}

/// `channel_bindings` + `updated_at` as a top-level merge for one record.
fn binding_fields_of(record: &Value) -> Value {
    let mut updates = Map::new();
    updates.insert(
        "channel_bindings".to_owned(),
        record
            .get("channel_bindings")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    if let Some(updated_at) = record.get("updated_at") {
        updates.insert("updated_at".to_owned(), updated_at.clone());
    }
    Value::Object(updates)
}

fn binding_from_endpoint(endpoint: &KnownChannelEndpoint) -> ChannelBinding {
    ChannelBinding {
        channel: endpoint.channel.clone(),
        account_id: endpoint.account_id.clone(),
        binding_key: endpoint.binding_key.clone(),
        chat_id: endpoint.chat_id.clone(),
        delivery_target_type: endpoint.delivery_target_type.clone(),
        delivery_target_id: endpoint.delivery_target_id.clone(),
        display_label: endpoint.display_label.clone(),
        last_inbound_at: endpoint.last_inbound_at.clone(),
        last_delivery_at: endpoint.last_delivery_at.clone(),
    }
}

#[async_trait]
impl EndpointBindingMutator for SqlEndpointBindingMutator {
    async fn binding_for_endpoint(
        &self,
        endpoint_key: &str,
    ) -> Result<Option<EndpointBindingOwner>, EndpointBindingMutationError> {
        let Some(endpoint) = self.projected_owner(endpoint_key).await? else {
            return Ok(None);
        };
        let Some(thread_id) = endpoint.thread_id.clone() else {
            return Err(EndpointBindingMutationError::Projection(format!(
                "endpoint projection '{endpoint_key}' has no owner"
            )));
        };
        Ok(Some(EndpointBindingOwner {
            thread_id,
            binding: binding_from_endpoint(&endpoint),
        }))
    }

    async fn bind_endpoint(
        &self,
        target_thread_id: &str,
        requested_binding: ChannelBinding,
    ) -> Result<EndpointBindResult, EndpointBindingMutationError> {
        let _guard = self.mutation_lock.lock().await;
        let target_thread_id = target_thread_id.trim();
        if self.thread_is_frozen(target_thread_id) {
            return Err(EndpointBindingMutationError::ThreadLifecycleInProgress(
                target_thread_id.to_owned(),
            ));
        }
        let endpoint_key = requested_binding.endpoint_key();
        let owner = self.projected_owner(&endpoint_key).await?;
        let binding = if let Some(owner) = owner.as_ref() {
            let mut binding = binding_from_endpoint(owner);
            if requested_binding.last_delivery_at.is_some() {
                binding.last_delivery_at = requested_binding.last_delivery_at;
            }
            binding
        } else {
            requested_binding
        };
        if self.is_archived(target_thread_id).await? {
            return Err(EndpointBindingMutationError::TargetArchived(
                target_thread_id.to_owned(),
            ));
        }
        let target = self
            .thread_store
            .get(target_thread_id)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: target_thread_id.to_owned(),
                message: error.to_string(),
            })?;
        let Some(mut target) = target else {
            return Err(EndpointBindingMutationError::TargetNotFound(
                target_thread_id.to_owned(),
            ));
        };
        validate_thread_accepts_bot_binding(
            target_thread_id,
            &target,
            &binding.channel,
            &binding.account_id,
        )
        .map_err(EndpointBindingMutationError::Incompatible)?;

        let owner_thread_id = owner.as_ref().and_then(|owner| owner.thread_id.clone());
        let previous_thread_id = owner_thread_id
            .as_deref()
            .filter(|previous| *previous != target_thread_id)
            .map(ToOwned::to_owned);
        let mut entries = Vec::new();
        if let Some(previous_thread_id) = previous_thread_id.as_deref() {
            match self.thread_store.get(previous_thread_id).await {
                Ok(Some(mut previous)) => {
                    if remove_binding(&mut previous, &endpoint_key) {
                        entries.push(AtomicRecordMerge {
                            thread_id: previous_thread_id.to_owned(),
                            fields: binding_fields_of(&previous),
                            create_if_missing: false,
                        });
                    } else {
                        return Err(EndpointBindingMutationError::PreviousOwnerUnavailable(
                            previous_thread_id.to_owned(),
                        ));
                    }
                }
                Ok(None) => {
                    return Err(EndpointBindingMutationError::PreviousOwnerUnavailable(
                        previous_thread_id.to_owned(),
                    ));
                }
                Err(error) => {
                    return Err(EndpointBindingMutationError::WriteFailed {
                        thread_id: previous_thread_id.to_owned(),
                        message: error.to_string(),
                    });
                }
            }
        }

        let existing_target_binding = bindings_from_value(&target)
            .into_iter()
            .find(|candidate| candidate.endpoint_key() == endpoint_key);
        let target_changed = existing_target_binding.as_ref() != Some(&binding);
        let changed = previous_thread_id.is_some() || target_changed;
        if changed {
            upsert_binding(&mut target, binding.clone());
            entries.push(AtomicRecordMerge {
                thread_id: target_thread_id.to_owned(),
                fields: binding_fields_of(&target),
                create_if_missing: false,
            });
        }
        entries.push(self.registry_merge(&binding).await?);
        self.commit_atomic(target_thread_id, entries).await?;

        Ok(EndpointBindResult {
            thread_id: target_thread_id.to_owned(),
            previous_thread_id,
            binding,
            changed,
        })
    }

    async fn detach_endpoint(
        &self,
        endpoint_key: &str,
    ) -> Result<EndpointDetachResult, EndpointBindingMutationError> {
        let _guard = self.mutation_lock.lock().await;
        let endpoint_key = endpoint_key.trim();
        let Some(owner) = self.projected_owner(endpoint_key).await? else {
            return Ok(EndpointDetachResult {
                previous_thread_id: None,
                binding: None,
                changed: false,
            });
        };
        let binding = binding_from_endpoint(&owner);
        let previous_thread_id = owner.thread_id.clone();
        let mut entries = Vec::new();
        let subject = if let Some(previous_thread_id) = previous_thread_id.as_deref() {
            match self.thread_store.get(previous_thread_id).await {
                Ok(Some(mut previous)) => {
                    if remove_binding(&mut previous, endpoint_key) {
                        entries.push(AtomicRecordMerge {
                            thread_id: previous_thread_id.to_owned(),
                            fields: binding_fields_of(&previous),
                            create_if_missing: false,
                        });
                        previous_thread_id.to_owned()
                    } else {
                        return Err(EndpointBindingMutationError::PreviousOwnerUnavailable(
                            previous_thread_id.to_owned(),
                        ));
                    }
                }
                Ok(None) => {
                    return Err(EndpointBindingMutationError::PreviousOwnerUnavailable(
                        previous_thread_id.to_owned(),
                    ));
                }
                Err(error) => {
                    return Err(EndpointBindingMutationError::WriteFailed {
                        thread_id: previous_thread_id.to_owned(),
                        message: error.to_string(),
                    });
                }
            }
        } else {
            return Err(EndpointBindingMutationError::Projection(format!(
                "endpoint projection '{endpoint_key}' has no owner"
            )));
        };
        entries.push(self.registry_merge(&binding).await?);
        self.commit_atomic(&subject, entries).await?;

        Ok(EndpointDetachResult {
            previous_thread_id,
            binding: Some(binding),
            changed: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use garyx_models::config::{TelegramAccount, telegram_account_to_plugin_entry};
    use garyx_router::{MessageRouter, ThreadStoreError, ThreadTranscriptStore};
    use serde_json::json;

    use super::*;
    use crate::recent_thread_projection::AlwaysActiveRunProbe;
    use crate::sqlite_thread_store::SqliteThreadStore;

    struct InstrumentedStore {
        inner: Arc<dyn ThreadStore>,
        list_calls: AtomicUsize,
        hidden_reads: StdMutex<HashSet<String>>,
        failed_updates: StdMutex<HashSet<String>>,
    }

    impl InstrumentedStore {
        fn new(inner: Arc<dyn ThreadStore>) -> Self {
            Self {
                inner,
                list_calls: AtomicUsize::new(0),
                hidden_reads: StdMutex::new(HashSet::new()),
                failed_updates: StdMutex::new(HashSet::new()),
            }
        }

        fn hide_read(&self, thread_id: &str) {
            self.hidden_reads
                .lock()
                .unwrap()
                .insert(thread_id.to_owned());
        }

        fn fail_update(&self, thread_id: &str) {
            self.failed_updates
                .lock()
                .unwrap()
                .insert(thread_id.to_owned());
        }
    }

    #[async_trait]
    impl ThreadStore for InstrumentedStore {
        async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
            if self.hidden_reads.lock().unwrap().contains(thread_id) {
                return Ok(None);
            }
            self.inner.get(thread_id).await
        }

        async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError> {
            self.inner.set(thread_id, data).await
        }

        async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.delete(thread_id).await
        }

        async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.list_keys(prefix).await
        }

        async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.exists(thread_id).await
        }

        async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
            if self.failed_updates.lock().unwrap().contains(thread_id) {
                return Err(ThreadStoreError::NotFound(thread_id.to_owned()));
            }
            self.inner.update(thread_id, updates).await
        }

        fn channel_endpoint_projection(
            &self,
        ) -> Option<Arc<dyn garyx_router::ChannelEndpointProjection>> {
            // A delegating wrapper shares its inner store's truth source,
            // projections included.
            self.inner.channel_endpoint_projection()
        }

        fn task_projection(&self) -> Option<Arc<dyn garyx_router::tasks::TaskProjectionReader>> {
            self.inner.task_projection()
        }

        async fn update_many_atomic(
            &self,
            entries: Vec<AtomicRecordMerge>,
        ) -> Result<(), ThreadStoreError> {
            // Mirror the transactional contract: a failure injected for ANY
            // record in the batch fails the WHOLE mutation before anything
            // is written.
            for entry in &entries {
                if self
                    .failed_updates
                    .lock()
                    .unwrap()
                    .contains(&entry.thread_id)
                {
                    return Err(ThreadStoreError::NotFound(entry.thread_id.clone()));
                }
            }
            self.inner.update_many_atomic(entries).await
        }
    }

    fn fixture() -> (
        Arc<GaryxDbService>,
        Arc<InstrumentedStore>,
        Arc<SqlEndpointBindingMutator>,
    ) {
        let db = Arc::new(GaryxDbService::memory().expect("in-memory database"));
        let inner: Arc<dyn ThreadStore> = Arc::new(SqliteThreadStore::new(
            db.clone(),
            Arc::new(ThreadTranscriptStore::memory()),
            Arc::new(AlwaysActiveRunProbe),
        ));
        let store = Arc::new(InstrumentedStore::new(inner));
        let mutator = Arc::new(SqlEndpointBindingMutator::new(store.clone(), db.clone()));
        (db, store, mutator)
    }

    fn binding() -> ChannelBinding {
        ChannelBinding {
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            binding_key: "1000000001".to_owned(),
            chat_id: "1000000001".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "1000000001".to_owned(),
            display_label: "Test User".to_owned(),
            last_inbound_at: Some("2026-07-11T08:00:00Z".to_owned()),
            last_delivery_at: None,
        }
    }

    async fn seed_thread(store: &Arc<InstrumentedStore>, thread_id: &str) {
        store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "label": thread_id,
                    "channel": "telegram",
                    "account_id": "main",
                    "updated_at": "2026-07-11T08:00:00Z"
                }),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn move_is_point_based_idempotent_and_has_no_ghost_projection() {
        let (db, store, mutator) = fixture();
        seed_thread(&store, "thread::old").await;
        seed_thread(&store, "thread::target").await;

        let first = mutator
            .bind_endpoint("thread::old", binding())
            .await
            .expect("initial bind");
        assert!(first.changed);
        let moved = mutator
            .bind_endpoint("thread::target", binding())
            .await
            .expect("move bind");
        assert_eq!(moved.previous_thread_id.as_deref(), Some("thread::old"));
        assert!(moved.changed);
        assert!(bindings_from_value(&store.get("thread::old").await.unwrap().unwrap()).is_empty());

        let idempotent = mutator
            .bind_endpoint("thread::target", binding())
            .await
            .expect("idempotent bind");
        assert!(!idempotent.changed);
        assert_eq!(idempotent.previous_thread_id, None);

        store
            .update("thread::old", json!({"unrelated": true}))
            .await
            .expect("unrelated old-owner update");
        let owner = db
            .get_thread_channel_endpoint("telegram::main::1000000001")
            .expect("projection lookup")
            .expect("projected owner");
        assert_eq!(owner.thread_id.as_deref(), Some("thread::target"));
        assert_eq!(store.list_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn router_delivery_updates_known_records_without_store_scan() {
        let (_db, store, mutator) = fixture();
        seed_thread(&store, "thread::old").await;
        seed_thread(&store, "thread::target").await;
        mutator
            .bind_endpoint("thread::old", binding())
            .await
            .expect("initial bind");

        let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
        router.set_endpoint_binding_mutator(mutator);
        let mutation = router
            .bind_endpoint_runtime("thread::target", binding())
            .await
            .expect("runtime bind");
        assert_eq!(mutation.previous_thread_id.as_deref(), Some("thread::old"));
        assert!(
            store.get("thread::target").await.unwrap().unwrap()["delivery_context"].is_object()
        );
        assert_eq!(store.list_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_owner_and_write_failure_are_explicit_errors() {
        let (db, store, mutator) = fixture();
        seed_thread(&store, "thread::old").await;
        seed_thread(&store, "thread::target").await;
        mutator
            .bind_endpoint("thread::old", binding())
            .await
            .expect("initial bind");
        store.hide_read("thread::old");

        let stale = mutator
            .bind_endpoint("thread::target", binding())
            .await
            .expect_err("stale owner must abort before creating a ghost binding");
        assert!(matches!(
            stale,
            EndpointBindingMutationError::PreviousOwnerUnavailable(ref thread_id)
                if thread_id == "thread::old"
        ));
        assert!(
            bindings_from_value(&store.get("thread::target").await.unwrap().unwrap()).is_empty()
        );

        store.hidden_reads.lock().unwrap().clear();
        store
            .update("thread::old", json!({"unrelated": true}))
            .await
            .expect("old owner remains writable");
        assert_eq!(
            db.get_thread_channel_endpoint("telegram::main::1000000001")
                .unwrap()
                .unwrap()
                .thread_id
                .as_deref(),
            Some("thread::old")
        );

        seed_thread(&store, "thread::failed").await;
        store.fail_update("thread::failed");
        let error = mutator
            .bind_endpoint("thread::failed", binding())
            .await
            .expect_err("target write failure must surface");
        assert!(matches!(
            error,
            EndpointBindingMutationError::WriteFailed { ref thread_id, .. }
                if thread_id == "thread::failed"
        ));
        // All-or-nothing (#TASK-2099 root final review): the failed move
        // must leave the previous owner's binding AND its projection
        // untouched — a mid-mutation storage failure never loses the
        // active binding.
        assert_eq!(
            bindings_from_value(&store.get("thread::old").await.unwrap().unwrap()),
            vec![binding()],
            "old owner keeps its binding after a failed move"
        );
        assert_eq!(
            db.get_thread_channel_endpoint("telegram::main::1000000001")
                .unwrap()
                .unwrap()
                .thread_id
                .as_deref(),
            Some("thread::old"),
            "projection owner is unchanged after a failed move"
        );
        assert_eq!(store.list_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn incompatible_target_is_rejected_before_mutation() {
        let (_db, store, mutator) = fixture();
        store
            .set(
                "thread::other-channel",
                json!({
                    "thread_id": "thread::other-channel",
                    "channel": "weixin",
                    "account_id": "main"
                }),
            )
            .await
            .unwrap();
        let error = mutator
            .bind_endpoint("thread::other-channel", binding())
            .await
            .expect_err("incompatible target");
        assert!(matches!(
            error,
            EndpointBindingMutationError::Incompatible(_)
        ));
        assert!(
            mutator
                .binding_for_endpoint("telegram::main::1000000001")
                .await
                .unwrap()
                .is_none()
        );

        let missing = mutator
            .bind_endpoint("thread::missing", binding())
            .await
            .expect_err("missing target");
        assert!(matches!(
            missing,
            EndpointBindingMutationError::TargetNotFound(ref thread_id)
                if thread_id == "thread::missing"
        ));
    }

    /// The trailing known-endpoint registry write is part of the same
    /// all-or-nothing mutation: failing it must leave the previous owner,
    /// the target, and the projection exactly as before the bind
    /// (#TASK-2099 root final review).
    #[tokio::test]
    async fn bind_registry_write_failure_leaves_owner_and_target_untouched() {
        let (db, store, mutator) = fixture();
        seed_thread(&store, "thread::old").await;
        seed_thread(&store, "thread::target").await;
        mutator
            .bind_endpoint("thread::old", binding())
            .await
            .expect("initial bind");

        store.fail_update("meta::known_channel_endpoints");
        let error = mutator
            .bind_endpoint("thread::target", binding())
            .await
            .expect_err("registry write failure must fail the whole mutation");
        assert!(matches!(
            error,
            EndpointBindingMutationError::WriteFailed { ref thread_id, .. }
                if thread_id == "meta::known_channel_endpoints"
        ));
        assert_eq!(
            bindings_from_value(&store.get("thread::old").await.unwrap().unwrap()),
            vec![binding()],
            "old owner keeps its binding"
        );
        assert!(
            bindings_from_value(&store.get("thread::target").await.unwrap().unwrap()).is_empty(),
            "target gains nothing from the failed mutation"
        );
        assert_eq!(
            db.get_thread_channel_endpoint("telegram::main::1000000001")
                .unwrap()
                .unwrap()
                .thread_id
                .as_deref(),
            Some("thread::old"),
            "projection owner is unchanged"
        );

        store.failed_updates.lock().unwrap().clear();
        let moved = mutator
            .bind_endpoint("thread::target", binding())
            .await
            .expect("mutation succeeds once storage recovers");
        assert_eq!(moved.previous_thread_id.as_deref(), Some("thread::old"));
    }

    /// Detach is the symmetric all-or-nothing mutation: an owner-write or
    /// trailing registry-write failure must leave the binding attached
    /// and projected (#TASK-2099 root final review).
    #[tokio::test]
    async fn detach_write_failures_leave_binding_attached() {
        let (db, store, mutator) = fixture();
        seed_thread(&store, "thread::owner").await;
        mutator
            .bind_endpoint("thread::owner", binding())
            .await
            .expect("initial bind");

        for failed_key in ["thread::owner", "meta::known_channel_endpoints"] {
            store.fail_update(failed_key);
            let error = mutator
                .detach_endpoint("telegram::main::1000000001")
                .await
                .expect_err("detach must fail as a whole");
            assert!(matches!(
                error,
                EndpointBindingMutationError::WriteFailed { ref thread_id, .. }
                    if thread_id == failed_key
            ));
            assert_eq!(
                bindings_from_value(&store.get("thread::owner").await.unwrap().unwrap()),
                vec![binding()],
                "owner keeps its binding after a failed detach ({failed_key})"
            );
            assert_eq!(
                db.get_thread_channel_endpoint("telegram::main::1000000001")
                    .unwrap()
                    .unwrap()
                    .thread_id
                    .as_deref(),
                Some("thread::owner"),
                "projection owner survives a failed detach ({failed_key})"
            );
            store.failed_updates.lock().unwrap().clear();
        }

        let detached = mutator
            .detach_endpoint("telegram::main::1000000001")
            .await
            .expect("detach succeeds once storage recovers");
        assert!(detached.changed);
    }

    /// Owner lookup and mutation writes must share ONE truth source
    /// (#TASK-2155): with an injected non-SQL store, the owner resolves
    /// through the scan projection over that same store — never an
    /// unrelated SQL database — so a move still strips the previous
    /// owner instead of leaving a ghost binding on both threads.
    #[tokio::test]
    async fn injected_store_moves_resolve_owner_from_the_same_store() {
        let db = Arc::new(GaryxDbService::memory().expect("in-memory database"));
        let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
        let mutator = SqlEndpointBindingMutator::new(store.clone(), db);
        for thread_id in ["thread::old", "thread::target"] {
            store
                .set(
                    thread_id,
                    json!({
                        "thread_id": thread_id,
                        "channel": "telegram",
                        "account_id": "main"
                    }),
                )
                .await
                .unwrap();
        }

        mutator
            .bind_endpoint("thread::old", binding())
            .await
            .expect("initial bind");
        let moved = mutator
            .bind_endpoint("thread::target", binding())
            .await
            .expect("move bind");
        assert_eq!(moved.previous_thread_id.as_deref(), Some("thread::old"));

        assert!(
            bindings_from_value(&store.get("thread::old").await.unwrap().unwrap()).is_empty(),
            "the previous owner must be stripped — no ghost binding"
        );
        assert_eq!(
            bindings_from_value(&store.get("thread::target").await.unwrap().unwrap()),
            vec![binding()],
        );
        let owner = mutator
            .binding_for_endpoint("telegram::main::1000000001")
            .await
            .expect("owner lookup")
            .expect("owner");
        assert_eq!(owner.thread_id, "thread::target");
    }

    #[tokio::test]
    async fn detach_is_point_based_and_idempotent() {
        let (db, store, mutator) = fixture();
        seed_thread(&store, "thread::owner").await;
        mutator
            .bind_endpoint("thread::owner", binding())
            .await
            .expect("initial bind");

        let detached = mutator
            .detach_endpoint("telegram::main::1000000001")
            .await
            .expect("detach");
        assert!(detached.changed);
        assert_eq!(
            detached.previous_thread_id.as_deref(),
            Some("thread::owner")
        );
        assert!(
            bindings_from_value(&store.get("thread::owner").await.unwrap().unwrap()).is_empty()
        );
        assert!(
            db.get_thread_channel_endpoint("telegram::main::1000000001")
                .unwrap()
                .is_none()
        );

        let second = mutator
            .detach_endpoint("telegram::main::1000000001")
            .await
            .expect("idempotent detach");
        assert!(!second.changed);
        assert_eq!(second.previous_thread_id, None);
        assert_eq!(store.list_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrent_moves_are_serialized_to_one_canonical_owner() {
        let (db, store, mutator) = fixture();
        seed_thread(&store, "thread::one").await;
        seed_thread(&store, "thread::two").await;

        let first = {
            let mutator = mutator.clone();
            tokio::spawn(async move { mutator.bind_endpoint("thread::one", binding()).await })
        };
        let second = {
            let mutator = mutator.clone();
            tokio::spawn(async move { mutator.bind_endpoint("thread::two", binding()).await })
        };
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        let projected = db
            .get_thread_channel_endpoint("telegram::main::1000000001")
            .unwrap()
            .unwrap()
            .thread_id
            .unwrap();
        let mut holders = Vec::new();
        for thread_id in ["thread::one", "thread::two"] {
            if store
                .get(thread_id)
                .await
                .unwrap()
                .is_some_and(|record| !bindings_from_value(&record).is_empty())
            {
                holders.push(thread_id);
            }
        }
        assert_eq!(holders, vec![projected.as_str()]);
        assert_eq!(store.list_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn delete_freeze_is_checked_before_config_and_blocks_then_releases_bind() {
        let (_db, store, mutator) = fixture();
        let thread_id = "thread::freeze-target";
        seed_thread(&store, thread_id).await;

        let preflight = mutator
            .preflight_and_freeze(thread_id, || Arc::new(GaryxConfig::default()))
            .await
            .expect("first preflight");
        let DeleteBindingPreflight::Frozen {
            guard,
            enabled_channel_accounts,
        } = preflight
        else {
            panic!("first preflight must freeze");
        };
        assert!(enabled_channel_accounts.is_empty());

        let config_was_read = Arc::new(AtomicUsize::new(0));
        let reads = Arc::clone(&config_was_read);
        let second = mutator
            .preflight_and_freeze(thread_id, move || {
                reads.fetch_add(1, Ordering::SeqCst);
                Arc::new(GaryxConfig::default())
            })
            .await
            .expect("second preflight");
        assert!(matches!(second, DeleteBindingPreflight::InProgress));
        assert_eq!(config_was_read.load(Ordering::SeqCst), 0);

        let blocked = mutator
            .bind_endpoint(thread_id, binding())
            .await
            .expect_err("freeze must block a new target bind");
        assert!(matches!(
            blocked,
            EndpointBindingMutationError::ThreadLifecycleInProgress(ref id) if id == thread_id
        ));

        drop(guard);
        mutator
            .bind_endpoint(thread_id, binding())
            .await
            .expect("bind resumes after freeze release");
    }

    #[tokio::test]
    async fn bind_committed_before_preflight_is_classified_with_locked_config_snapshot() {
        let (_db, store, mutator) = fixture();
        let thread_id = "thread::preflight-sees-bind";
        seed_thread(&store, thread_id).await;
        mutator
            .bind_endpoint(thread_id, binding())
            .await
            .expect("bind commits first");

        let mut config = GaryxConfig::default();
        config
            .channels
            .plugin_channel_mut("telegram")
            .accounts
            .insert(
                "main".to_owned(),
                telegram_account_to_plugin_entry(&TelegramAccount {
                    token: "${TOKEN}".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: None,
                    workspace_dir: None,
                    owner_target: None,
                    groups: HashMap::new(),
                }),
            );
        let preflight = mutator
            .preflight_and_freeze(thread_id, || Arc::new(config))
            .await
            .expect("preflight");
        assert!(matches!(
            preflight,
            DeleteBindingPreflight::RejectedEnabledBinding
        ));
    }
}
