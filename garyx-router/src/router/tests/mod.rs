use super::*;
use crate::endpoint_binding::{
    EndpointBindResult, EndpointBindingMutationError, EndpointBindingMutator, EndpointBindingOwner,
    EndpointDetachResult,
};
use crate::memory_store::InMemoryThreadStore;
use crate::message_ledger::MessageLedgerStore;
use crate::recent_threads::{
    RecentThreadFilter, RecentThreadListEntry, RecentThreadPage, RecentThreadPageReader,
};
use crate::store::ThreadStoreError;
use crate::threads::{
    ChannelBinding, bindings_from_value, remove_binding, upsert_binding,
    upsert_known_channel_endpoint, validate_thread_accepts_bot_binding,
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

#[async_trait]
impl ThreadStore for NoScanThreadStore {
    async fn get(&self, thread_id: &str) -> Option<Value> {
        self.inner.get(thread_id).await
    }

    async fn set(&self, thread_id: &str, data: Value) {
        self.inner.set(thread_id, data).await;
    }

    async fn delete(&self, thread_id: &str) -> bool {
        self.inner.delete(thread_id).await
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Vec<String> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.list_keys(prefix).await
    }

    async fn exists(&self, thread_id: &str) -> bool {
        self.inner.exists(thread_id).await
    }

    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
        self.inner.update(thread_id, updates).await
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
        let Some(mut target) = self.store.get(target_thread_id).await else {
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
        if let Some(previous_thread_id) = previous_thread_id.as_deref() {
            match self.store.get(previous_thread_id).await {
                Some(mut previous) => {
                    if remove_binding(&mut previous, &endpoint_key) {
                        self.store.set(previous_thread_id, previous).await;
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
            self.store.set(target_thread_id, target).await;
        }
        upsert_known_channel_endpoint(&self.store, &binding)
            .await
            .map_err(|message| EndpointBindingMutationError::WriteFailed {
                thread_id: crate::threads::KNOWN_CHANNEL_ENDPOINTS_KEY.to_owned(),
                message,
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
        match self.store.get(&owner.thread_id).await {
            Some(mut previous) => {
                if remove_binding(&mut previous, endpoint_key) {
                    self.store.set(&owner.thread_id, previous).await;
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
        upsert_known_channel_endpoint(&self.store, &owner.binding)
            .await
            .map_err(|message| EndpointBindingMutationError::WriteFailed {
                thread_id: crate::threads::KNOWN_CHANNEL_ENDPOINTS_KEY.to_owned(),
                message,
            })?;
        Ok(EndpointDetachResult {
            previous_thread_id: Some(owner.thread_id),
            binding: Some(owner.binding),
            changed: true,
        })
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
