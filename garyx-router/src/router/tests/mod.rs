use super::*;
use crate::endpoint_binding::{
    EndpointBindResult, EndpointBindingMutationError, EndpointBindingMutator, EndpointBindingOwner,
    EndpointDeliveryTimestampResult, EndpointDetachResult,
};
use crate::memory_store::InMemoryThreadStore;
use crate::message_ledger::MessageLedgerStore;
use crate::recent_threads::{
    RecentThreadFilter, RecentThreadListEntry, RecentThreadPage, RecentThreadPageReader,
};
use crate::store::{AtomicRecordMerge, ThreadPatchResult, ThreadRecordPatch, ThreadStoreError};
use crate::threads::{
    ChannelBinding, KNOWN_CHANNEL_ENDPOINTS_KEY, bindings_from_value, remove_binding,
    upsert_binding, validate_thread_accepts_bot_binding,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Mutex;

struct TestEndpointBindingMutator {
    store: Arc<dyn ThreadStore>,
    owners: Mutex<HashMap<String, EndpointBindingOwner>>,
}

struct NoScanThreadStore {
    inner: InMemoryThreadStore,
    list_calls: AtomicUsize,
}

impl NoScanThreadStore {
    fn new() -> Self {
        Self {
            inner: InMemoryThreadStore::new(),
            list_calls: AtomicUsize::new(0),
        }
    }
}

impl crate::ThreadStoreDomains for NoScanThreadStore {
    fn run_coordinator(&self) -> Arc<crate::ThreadRunCoordinator> {
        self.inner.run_coordinator()
    }
}

#[async_trait]
impl ThreadStore for NoScanThreadStore {
    async fn terminal_state(
        &self,
        thread_id: &str,
    ) -> Result<Option<crate::ThreadTerminalState>, ThreadStoreError> {
        self.inner.terminal_state(thread_id).await
    }

    async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
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

    async fn patch(
        &self,
        thread_id: &str,
        patch: ThreadRecordPatch,
    ) -> Result<ThreadPatchResult, ThreadStoreError> {
        self.inner.patch(thread_id, patch).await
    }

    async fn update_many_atomic(
        &self,
        entries: Vec<AtomicRecordMerge>,
    ) -> Result<(), ThreadStoreError> {
        self.inner.update_many_atomic(entries).await
    }
}

fn test_binding_merge(
    thread_id: &str,
    record: &Value,
    create_if_missing: bool,
) -> AtomicRecordMerge {
    AtomicRecordMerge {
        thread_id: thread_id.to_owned(),
        fields: serde_json::json!({
            "channel_bindings": record
                .get("channel_bindings")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
            "updated_at": record.get("updated_at").cloned().unwrap_or(Value::Null),
        }),
        create_if_missing,
    }
}

impl TestEndpointBindingMutator {
    fn new(store: Arc<dyn ThreadStore>) -> Self {
        Self {
            store,
            owners: Mutex::new(HashMap::new()),
        }
    }

    async fn seed_owner(&self, thread_id: &str, binding: ChannelBinding) {
        self.owners.lock().await.insert(
            binding.endpoint_key(),
            EndpointBindingOwner {
                thread_id: thread_id.to_owned(),
                binding,
            },
        );
    }
}

#[async_trait]
impl EndpointBindingMutator for TestEndpointBindingMutator {
    async fn binding_for_endpoint(
        &self,
        endpoint_key: &str,
    ) -> Result<Option<EndpointBindingOwner>, EndpointBindingMutationError> {
        Ok(self.owners.lock().await.get(endpoint_key).cloned())
    }

    async fn bind_endpoint(
        &self,
        target_thread_id: &str,
        binding: ChannelBinding,
    ) -> Result<EndpointBindResult, EndpointBindingMutationError> {
        let mut owners = self.owners.lock().await;
        let Ok(Some(mut target)) = self.store.get(target_thread_id).await else {
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

        let endpoint_key = binding.endpoint_key();
        let previous_owner = owners.get(&endpoint_key).cloned();
        let previous_thread_id = previous_owner
            .as_ref()
            .map(|owner| owner.thread_id.as_str())
            .filter(|owner| *owner != target_thread_id)
            .map(ToOwned::to_owned);
        let mut entries = Vec::new();
        if let Some(previous_thread_id) = previous_thread_id.as_deref() {
            match self
                .store
                .get(previous_thread_id)
                .await
                .expect("test store")
            {
                Some(mut previous) => {
                    if remove_binding(&mut previous, &endpoint_key) {
                        entries.push(test_binding_merge(previous_thread_id, &previous, false));
                    } else {
                        return Err(EndpointBindingMutationError::PreviousOwnerUnavailable(
                            previous_thread_id.to_owned(),
                        ));
                    }
                }
                None => {
                    return Err(EndpointBindingMutationError::PreviousOwnerUnavailable(
                        previous_thread_id.to_owned(),
                    ));
                }
            }
        }

        let target_changed = bindings_from_value(&target)
            .into_iter()
            .find(|candidate| candidate.endpoint_key() == endpoint_key)
            .as_ref()
            != Some(&binding);
        if target_changed || previous_thread_id.is_some() {
            upsert_binding(&mut target, binding.clone());
            entries.push(test_binding_merge(target_thread_id, &target, false));
        }
        let mut registry = self
            .store
            .get(KNOWN_CHANNEL_ENDPOINTS_KEY)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: crate::threads::KNOWN_CHANNEL_ENDPOINTS_KEY.to_owned(),
                message: error.to_string(),
            })?
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        upsert_binding(&mut registry, binding.clone());
        entries.push(test_binding_merge(
            KNOWN_CHANNEL_ENDPOINTS_KEY,
            &registry,
            true,
        ));
        self.store
            .update_many_atomic(entries)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: target_thread_id.to_owned(),
                message: error.to_string(),
            })?;
        owners.insert(
            endpoint_key,
            EndpointBindingOwner {
                thread_id: target_thread_id.to_owned(),
                binding: binding.clone(),
            },
        );

        Ok(EndpointBindResult {
            thread_id: target_thread_id.to_owned(),
            previous_thread_id,
            binding,
            changed: target_changed
                || previous_owner.is_some_and(|owner| owner.thread_id != target_thread_id),
        })
    }

    async fn detach_endpoint(
        &self,
        endpoint_key: &str,
    ) -> Result<EndpointDetachResult, EndpointBindingMutationError> {
        let mut owners = self.owners.lock().await;
        let Some(owner) = owners.remove(endpoint_key) else {
            return Ok(EndpointDetachResult {
                previous_thread_id: None,
                binding: None,
                changed: false,
            });
        };
        let mut entries = Vec::new();
        match self.store.get(&owner.thread_id).await.expect("test store") {
            Some(mut previous) => {
                if remove_binding(&mut previous, endpoint_key) {
                    entries.push(test_binding_merge(&owner.thread_id, &previous, false));
                } else {
                    return Err(EndpointBindingMutationError::PreviousOwnerUnavailable(
                        owner.thread_id,
                    ));
                }
            }
            None => {
                return Err(EndpointBindingMutationError::PreviousOwnerUnavailable(
                    owner.thread_id,
                ));
            }
        }
        let mut registry = self
            .store
            .get(KNOWN_CHANNEL_ENDPOINTS_KEY)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: KNOWN_CHANNEL_ENDPOINTS_KEY.to_owned(),
                message: error.to_string(),
            })?
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        upsert_binding(&mut registry, owner.binding.clone());
        entries.push(test_binding_merge(
            KNOWN_CHANNEL_ENDPOINTS_KEY,
            &registry,
            true,
        ));
        self.store
            .update_many_atomic(entries)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: owner.thread_id.clone(),
                message: error.to_string(),
            })?;
        Ok(EndpointDetachResult {
            previous_thread_id: Some(owner.thread_id),
            binding: Some(owner.binding),
            changed: true,
        })
    }

    async fn sync_delivery_timestamp(
        &self,
        endpoint_key: &str,
        expected_holder_thread_id: &str,
        last_delivery_at: Option<String>,
    ) -> Result<EndpointDeliveryTimestampResult, EndpointBindingMutationError> {
        let mut owners = self.owners.lock().await;
        let Some(owner) = owners.get(endpoint_key).cloned() else {
            return Ok(EndpointDeliveryTimestampResult::NotFound);
        };
        if owner.thread_id != expected_holder_thread_id {
            return Ok(EndpointDeliveryTimestampResult::OwnerChanged {
                current_holder: Some(owner.thread_id),
            });
        }
        let mut binding = owner.binding;
        let mut holder = self
            .store
            .get(expected_holder_thread_id)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: expected_holder_thread_id.to_owned(),
                message: error.to_string(),
            })?
            .ok_or_else(|| {
                EndpointBindingMutationError::PreviousOwnerUnavailable(
                    expected_holder_thread_id.to_owned(),
                )
            })?;
        let mut registry = self
            .store
            .get(KNOWN_CHANNEL_ENDPOINTS_KEY)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: KNOWN_CHANNEL_ENDPOINTS_KEY.to_owned(),
                message: error.to_string(),
            })?
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let holder_changed = binding.last_delivery_at != last_delivery_at;
        binding.last_delivery_at = last_delivery_at;
        let registry_changed = bindings_from_value(&registry)
            .into_iter()
            .find(|candidate| candidate.endpoint_key() == endpoint_key)
            .as_ref()
            != Some(&binding);
        if !holder_changed && !registry_changed {
            return Ok(EndpointDeliveryTimestampResult::Unchanged);
        }
        let mut entries = Vec::new();
        if holder_changed {
            upsert_binding(&mut holder, binding.clone());
            entries.push(test_binding_merge(
                expected_holder_thread_id,
                &holder,
                false,
            ));
        }
        if registry_changed {
            upsert_binding(&mut registry, binding.clone());
            entries.push(test_binding_merge(
                KNOWN_CHANNEL_ENDPOINTS_KEY,
                &registry,
                true,
            ));
        }
        self.store
            .update_many_atomic(entries)
            .await
            .map_err(|error| EndpointBindingMutationError::WriteFailed {
                thread_id: expected_holder_thread_id.to_owned(),
                message: error.to_string(),
            })?;
        owners.insert(
            endpoint_key.to_owned(),
            EndpointBindingOwner {
                thread_id: expected_holder_thread_id.to_owned(),
                binding,
            },
        );
        Ok(EndpointDeliveryTimestampResult::Applied)
    }
}

struct TestRecentThreadPageReader {
    entries: Mutex<Vec<RecentThreadListEntry>>,
    fail_page: AtomicBool,
    fail_contains: AtomicBool,
    page_calls: AtomicUsize,
    contains_calls: AtomicUsize,
}

impl TestRecentThreadPageReader {
    fn new(entries: Vec<RecentThreadListEntry>) -> Self {
        Self {
            entries: Mutex::new(entries),
            fail_page: AtomicBool::new(false),
            fail_contains: AtomicBool::new(false),
            page_calls: AtomicUsize::new(0),
            contains_calls: AtomicUsize::new(0),
        }
    }

    async fn replace_entries(&self, entries: Vec<RecentThreadListEntry>) {
        *self.entries.lock().await = entries;
    }
}

#[async_trait]
impl RecentThreadPageReader for TestRecentThreadPageReader {
    async fn page(
        &self,
        filter: RecentThreadFilter,
        limit: usize,
        offset: usize,
    ) -> Result<RecentThreadPage, String> {
        self.page_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_page.load(Ordering::SeqCst) {
            return Err("forced page failure".to_owned());
        }
        assert_eq!(filter, RecentThreadFilter::Exclude);
        let entries = self.entries.lock().await;
        let total = entries.len();
        let offset = offset.min(total);
        let page_entries = entries
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        Ok(RecentThreadPage {
            has_more: offset.saturating_add(page_entries.len()) < total,
            entries: page_entries,
            total,
            offset,
        })
    }

    async fn contains_selectable_thread(&self, thread_id: &str) -> Result<bool, String> {
        self.contains_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_contains.load(Ordering::SeqCst) {
            return Err("forced selectability failure".to_owned());
        }
        Ok(self
            .entries
            .lock()
            .await
            .iter()
            .any(|entry| entry.thread_id == thread_id))
    }
}

fn recent_entry(thread_id: &str, title: &str) -> RecentThreadListEntry {
    RecentThreadListEntry {
        thread_id: thread_id.to_owned(),
        title: title.to_owned(),
        last_message_preview: String::new(),
        last_active_at: "2026-07-11T08:00:00Z".to_owned(),
    }
}

fn test_router(
    store: Arc<dyn ThreadStore>,
    config: GaryxConfig,
) -> (MessageRouter, Arc<TestEndpointBindingMutator>) {
    let mut router = MessageRouter::new(store.clone(), config);
    let mutator = Arc::new(TestEndpointBindingMutator::new(store));
    router.set_endpoint_binding_mutator(mutator.clone());
    router.set_recent_thread_page_reader(Arc::new(TestRecentThreadPageReader::new(Vec::new())));
    (router, mutator)
}

fn make_router() -> MessageRouter {
    let store = Arc::new(InMemoryThreadStore::new());
    let config = GaryxConfig::default();
    let (mut router, _) = test_router(store, config);
    router.set_message_ledger_store(Arc::new(MessageLedgerStore::memory()));
    router
}

mod delivery;
mod dispatch;
mod inbound;
mod navigation;
mod routing;
