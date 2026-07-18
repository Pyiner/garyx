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

use crate::provider_trait::ClearSessionOutcome;

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

#[cfg(test)]
pub(crate) struct ProviderPersistenceProbe {
    pub ledger_messages: Vec<serde_json::Value>,
    pub ledger_seqs: Vec<u64>,
    pub committed_events: Vec<serde_json::Value>,
}

/// Test-only provider → persistence probe used by provider-specific adoption
/// tests. It runs both the streaming append and terminal reconcile, then
/// captures the real `committed_message` emitter output.
#[cfg(test)]
pub(crate) async fn probe_provider_persistence(
    session_messages: &[garyx_models::provider::ProviderMessage],
    assistant_response: &str,
    stream_events: &[StreamEvent],
) -> ProviderPersistenceProbe {
    use persistence::{
        PersistedRun, RunControlRecord, TerminalRunControl, save_streaming_partial,
        save_thread_messages_with_terminal_control,
    };

    const THREAD_ID: &str = "thread::provider-persistence-probe";
    const RUN_ID: &str = "run::provider-persistence-probe";
    const STARTED_AT: &str = "2026-07-18T12:00:00Z";

    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    let history = Arc::new(ThreadHistoryRepository::new(
        store.clone(),
        Arc::new(garyx_router::ThreadTranscriptStore::memory()),
    ));
    store.set(THREAD_ID, serde_json::json!({})).await.unwrap();

    let metadata = HashMap::from([(
        "bridge_run_id".to_owned(),
        serde_json::Value::String(RUN_ID.to_owned()),
    )]);
    let mut controls = vec![RunControlRecord::new(
        "run_start",
        THREAD_ID,
        RUN_ID,
        STARTED_AT.to_owned(),
        serde_json::Map::new(),
        0,
    )];
    for event in stream_events {
        let kind = match event {
            StreamEvent::Boundary {
                kind: garyx_models::provider::StreamBoundaryKind::AssistantSegment,
                ..
            } => Some("assistant_boundary"),
            StreamEvent::Boundary {
                kind: garyx_models::provider::StreamBoundaryKind::UserAck,
                ..
            } => Some("user_ack"),
            StreamEvent::Done => Some("done"),
            _ => None,
        };
        if let Some(kind) = kind {
            controls.push(RunControlRecord::new(
                kind,
                THREAD_ID,
                RUN_ID,
                STARTED_AT.to_owned(),
                serde_json::Map::new(),
                1 + session_messages.len(),
            ));
        }
    }

    let (event_tx, mut event_rx) = tokio::sync::broadcast::channel(32);
    let persisted_run = || PersistedRun {
        thread_id: THREAD_ID,
        user_message: "Run a synthetic delegated check",
        user_timestamp: Some(STARTED_AT),
        user_images: &[],
        assistant_response,
        sdk_session_id: Some("sdk-session-persistence-probe"),
        provider_key: "provider::claude-probe",
        provider_type: garyx_models::provider::ProviderType::ClaudeCode,
        session_messages,
        metadata: &metadata,
    };
    let (_, streaming_committed) = save_streaming_partial(
        &store,
        &history,
        persisted_run(),
        &[],
        &controls,
        session_messages.len(),
        0,
    )
    .await;
    run_management::emit_committed_records_for_test(
        &Some(event_tx.clone()),
        THREAD_ID,
        Some(RUN_ID),
        streaming_committed,
    );

    let terminal_committed = save_thread_messages_with_terminal_control(
        &store,
        &history,
        persisted_run(),
        &controls,
        Some(TerminalRunControl {
            duration_ms: Some(1),
            success: Some(true),
            ..Default::default()
        }),
    )
    .await;
    run_management::emit_committed_records_for_test(
        &Some(event_tx),
        THREAD_ID,
        Some(RUN_ID),
        terminal_committed,
    );

    let records = history
        .transcript_store()
        .records(THREAD_ID)
        .await
        .expect("probe ledger records");
    let mut committed_events = Vec::new();
    while let Ok(raw) = event_rx.try_recv() {
        committed_events.push(serde_json::from_str(&raw).expect("committed event json"));
    }
    ProviderPersistenceProbe {
        ledger_messages: records
            .iter()
            .map(|record| record.message.clone())
            .collect(),
        ledger_seqs: records.iter().map(|record| record.seq).collect(),
        committed_events,
    }
}

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
    ) -> ClearSessionOutcome {
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
            return ClearSessionOutcome::AlreadyAbsent;
        };
        let Some(provider) = self.get_provider(&provider_key).await else {
            return ClearSessionOutcome::AlreadyAbsent;
        };
        provider.clear_session(thread_id).await
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
