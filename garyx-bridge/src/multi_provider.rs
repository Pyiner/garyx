use std::collections::HashMap;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use garyx_models::provider::{AgentDispatchOutcome, StreamEvent};
use garyx_models::thread_logs::ThreadLogSink;
use garyx_models::{AgentAvailabilitySnapshot, CustomAgentProfile};
use garyx_router::{
    AdmittedRun, AgentDispatcher, ThreadHistoryRepository, ThreadRunAborter, ThreadStore,
};
use tokio::sync::broadcast;

mod lifecycle;
mod persistence;
mod provider_factory;
mod resolver;
mod run_management;
mod state;
#[cfg(test)]
mod tests;
mod topology;

use state::{Inner, default_max_concurrent_runs};

/// Routes agent-run requests to the appropriate provider based on
/// channel/account/session affinity.
///
/// Rust port of `MultiProviderBridge` from
/// `src/garyx/agent_bridge/multi_provider_bridge.py`.
#[derive(Clone)]
pub struct MultiProviderBridge {
    inner: Arc<Inner>,
}

struct WeakBridgeThreadRunAborter {
    inner: Weak<Inner>,
}

#[async_trait]
impl ThreadRunAborter for WeakBridgeThreadRunAborter {
    async fn abort_and_drain_thread(&self, thread_id: &str) -> Result<(), String> {
        let Some(inner) = self.inner.upgrade() else {
            return Ok(());
        };
        MultiProviderBridge { inner }
            .abort_thread_runs_and_wait(thread_id)
            .await;
        Ok(())
    }
}

impl MultiProviderBridge {
    /// Create a new, empty bridge.
    pub fn new() -> Self {
        Self::new_with_max_concurrent_runs(default_max_concurrent_runs())
    }

    /// Create a bridge with an explicit global run-concurrency limit.
    pub fn new_with_max_concurrent_runs(max_concurrent_runs: usize) -> Self {
        Self {
            inner: Arc::new(Inner::new(max_concurrent_runs)),
        }
    }

    /// Maximum number of concurrent runs accepted by this bridge.
    pub fn max_concurrent_runs(&self) -> usize {
        self.inner.max_concurrent_runs
    }

    /// Currently available run slots.
    pub fn available_run_slots(&self) -> usize {
        self.inner.run_limiter.available_permits()
    }

    /// Set the thread store for persisting messages after agent runs.
    pub async fn set_thread_store(&self, store: Arc<dyn ThreadStore>) {
        store
            .run_coordinator()
            .set_aborter(Arc::new(WeakBridgeThreadRunAborter {
                inner: Arc::downgrade(&self.inner),
            }));
        *self.inner.thread_store.write().await = Some(store);
    }

    pub fn set_thread_store_blocking(&self, store: Arc<dyn ThreadStore>) {
        store
            .run_coordinator()
            .set_aborter(Arc::new(WeakBridgeThreadRunAborter {
                inner: Arc::downgrade(&self.inner),
            }));
        if let Ok(mut guard) = self.inner.thread_store.try_write() {
            *guard = Some(store);
        }
    }

    pub fn set_thread_history(&self, history: Arc<ThreadHistoryRepository>) {
        if let Ok(mut guard) = self.inner.thread_history.try_write() {
            *guard = Some(history);
        }
    }

    pub async fn thread_history(&self) -> Option<Arc<ThreadHistoryRepository>> {
        self.inner.thread_history.read().await.clone()
    }

    /// Set the gateway event channel for committed transcript fan-out.
    pub async fn set_event_tx(&self, tx: broadcast::Sender<String>) {
        *self.inner.event_tx.write().await = Some(tx);
    }

    /// Subscribe to the gateway event bus this bridge emits onto.
    ///
    /// The bridge emits replayable `committed_message{seq}` records and the
    /// task notification event. Channel consumers use committed messages to read
    /// the durable transcript stream instead of draining the live
    /// `external_callback`. Returns `None` before the event bus is wired.
    ///
    /// Subscribe BEFORE dispatching the run so no committed record is missed in
    /// the gap between subscribe and the first emit.
    pub async fn subscribe_events(&self) -> Option<broadcast::Receiver<String>> {
        self.inner
            .event_tx
            .read()
            .await
            .as_ref()
            .map(broadcast::Sender::subscribe)
    }

    pub fn set_thread_log_sink(&self, sink: Arc<dyn ThreadLogSink>) {
        if let Ok(mut guard) = self.inner.thread_logs.write() {
            *guard = Some(sink);
        }
    }

    pub fn thread_log_sink(&self) -> Option<Arc<dyn ThreadLogSink>> {
        self.inner
            .thread_logs
            .read()
            .map_err(|e| tracing::warn!(error = %e, "thread_logs RwLock poisoned"))
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub async fn replace_thread_workspace_bindings(&self, bindings: HashMap<String, String>) {
        *self.inner.thread_workspace_bindings.write().await = bindings;
    }

    /// Apply one atomically captured agent-store snapshot. Late delivery of an
    /// older revision is ignored; enabled/default remain gateway-only policy
    /// and are not consulted by bridge routing.
    pub async fn replace_agent_profiles(&self, snapshot: AgentAvailabilitySnapshot) -> bool {
        let mut state = self.inner.agent_profiles.write().await;
        if snapshot.agent_state_revision < state.revision {
            return false;
        }
        let mut next = HashMap::new();
        for profile in snapshot.agents {
            next.insert(profile.agent_id.clone(), profile);
        }
        state.revision = snapshot.agent_state_revision;
        state.profiles = next;
        true
    }

    pub async fn agent_profile(&self, agent_id: &str) -> Option<CustomAgentProfile> {
        self.inner
            .agent_profiles
            .read()
            .await
            .profiles
            .get(agent_id)
            .cloned()
    }

    pub async fn set_thread_workspace_binding(
        &self,
        thread_id: &str,
        workspace_dir: Option<String>,
    ) {
        let mut bindings = self.inner.thread_workspace_bindings.write().await;
        let normalized = workspace_dir.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(
                    std::fs::canonicalize(trimmed)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| trimmed.to_owned()),
                )
            }
        });
        if let Some(workspace_dir) = normalized {
            bindings.insert(thread_id.to_owned(), workspace_dir);
        } else {
            bindings.remove(thread_id);
        }
    }

    pub async fn remove_thread_workspace_binding(&self, thread_id: &str) {
        self.inner
            .thread_workspace_bindings
            .write()
            .await
            .remove(thread_id);
    }

    pub async fn thread_affinity_for(&self, thread_id: &str) -> Option<String> {
        self.inner
            .thread_affinity
            .read()
            .await
            .get(thread_id)
            .cloned()
    }

    pub async fn drop_thread_state(&self, thread_id: &str) {
        self.inner.thread_affinity.write().await.remove(thread_id);
        self.remove_thread_workspace_binding(thread_id).await;
    }

    pub async fn clear_thread_state(
        &self,
        thread_id: &str,
        provider_key_hint: Option<&str>,
    ) -> bool {
        let provider_key = self
            .inner
            .thread_affinity
            .read()
            .await
            .get(thread_id)
            .cloned()
            .or_else(|| {
                provider_key_hint
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            });

        let Some(provider_key) = provider_key else {
            return false;
        };
        let Some(provider) = self.get_provider(&provider_key).await else {
            return false;
        };
        if !provider.clear_session(thread_id).await {
            return false;
        }

        self.inner.thread_affinity.write().await.remove(thread_id);
        self.remove_thread_workspace_binding(thread_id).await;
        true
    }

    pub async fn thread_workspace_bindings_snapshot(&self) -> HashMap<String, String> {
        self.inner.thread_workspace_bindings.read().await.clone()
    }
}

impl Default for MultiProviderBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentDispatcher for MultiProviderBridge {
    async fn dispatch(
        &self,
        run: AdmittedRun,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<AgentDispatchOutcome, String> {
        let (request, lease) = run.into_dispatch_parts();
        self.start_admitted_run(request, lease, response_callback)
            .await
            .map_err(|e| e.to_string())
    }
}
