use std::sync::Arc;

use async_trait::async_trait;
use garyx_router::{
    ChannelBinding, EndpointBindResult, EndpointBindingMutationError, EndpointBindingMutator,
    EndpointBindingOwner, EndpointDetachResult, KnownChannelEndpoint, ThreadStore,
    bindings_from_value, remove_binding, upsert_binding, upsert_known_channel_endpoint,
    validate_thread_accepts_bot_binding,
};
use serde_json::{Map, Value};
use tokio::sync::Mutex;

use crate::garyx_db::GaryxDbService;

pub(crate) struct SqlEndpointBindingMutator {
    thread_store: Arc<dyn ThreadStore>,
    garyx_db: Arc<GaryxDbService>,
    mutation_lock: Mutex<()>,
}

impl SqlEndpointBindingMutator {
    pub(crate) fn new(thread_store: Arc<dyn ThreadStore>, garyx_db: Arc<GaryxDbService>) -> Self {
        Self {
            thread_store,
            garyx_db,
            mutation_lock: Mutex::new(()),
        }
    }

    async fn projected_owner(
        &self,
        endpoint_key: &str,
    ) -> Result<Option<KnownChannelEndpoint>, EndpointBindingMutationError> {
        let endpoint_key = endpoint_key.to_owned();
        self.garyx_db
            .run_blocking(move |db| db.get_thread_channel_endpoint(&endpoint_key))
            .await
            .map_err(|error| EndpointBindingMutationError::Projection(error.to_string()))
    }

    async fn is_archived(&self, thread_id: &str) -> Result<bool, EndpointBindingMutationError> {
        let thread_id = thread_id.to_owned();
        self.garyx_db
            .run_blocking(move |db| db.is_thread_archived(&thread_id))
            .await
            .map_err(|error| EndpointBindingMutationError::Projection(error.to_string()))
    }

    async fn write_binding_fields(
        &self,
        thread_id: &str,
        record: &Value,
    ) -> Result<(), EndpointBindingMutationError> {
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
        self.thread_store
            .update(thread_id, Value::Object(updates))
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: thread_id.to_owned(),
                message: error.to_string(),
            })
    }
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
        if let Some(previous_thread_id) = previous_thread_id.as_deref() {
            match self.thread_store.get(previous_thread_id).await {
                Ok(Some(mut previous)) => {
                    if remove_binding(&mut previous, &endpoint_key) {
                        self.write_binding_fields(previous_thread_id, &previous)
                            .await?;
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
            self.write_binding_fields(target_thread_id, &target).await?;
        }
        upsert_known_channel_endpoint(&self.thread_store, &binding)
            .await
            .map_err(|message| EndpointBindingMutationError::WriteFailed {
                thread_id: "meta::known_channel_endpoints".to_owned(),
                message,
            })?;

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
        if let Some(previous_thread_id) = previous_thread_id.as_deref() {
            match self.thread_store.get(previous_thread_id).await {
                Ok(Some(mut previous)) => {
                    if remove_binding(&mut previous, endpoint_key) {
                        self.write_binding_fields(previous_thread_id, &previous)
                            .await?;
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
        }
        upsert_known_channel_endpoint(&self.thread_store, &binding)
            .await
            .map_err(|message| EndpointBindingMutationError::WriteFailed {
                thread_id: "meta::known_channel_endpoints".to_owned(),
                message,
            })?;

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

    use garyx_models::config::GaryxConfig;
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
}
