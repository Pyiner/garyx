use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Utc;
use garyx_models::config::{GaryxConfig, TelegramAccount, telegram_account_to_plugin_entry};
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, AgentRunRequest, FORK_FROM_PROVIDER_TYPE_METADATA_KEY,
    FORK_FROM_SDK_SESSION_ID_METADATA_KEY, ImagePayload, PromptAttachment, PromptAttachmentKind,
    ProviderMessage, ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput,
    SDK_SESSION_FORK_METADATA_KEY, SDK_SESSION_ID_METADATA_KEY, StreamBoundaryKind, StreamEvent,
    attachments_to_metadata_value,
};
use garyx_models::thread_logs::{ThreadLogChunk, ThreadLogEvent, ThreadLogSink};
use garyx_models::{
    AgentAvailabilitySnapshot, CustomAgentProfile, Principal, TaskEvent, TaskEventKind, TaskStatus,
    ThreadTask, builtin_provider_agent_profiles,
};
use garyx_router::{
    AdmittedRun, AgentDispatcher, ArchiveBarrier, InMemoryThreadStore, ThreadHistoryRepository,
    ThreadStore, ThreadTranscriptStore,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc};

use super::{MultiProviderBridge, RunLifecycleEvent};
use crate::provider_trait::{
    BridgeError, ClearSessionOutcome, ProviderRuntime, ProviderRuntimeSelection, StreamCallback,
};

fn run_request(
    thread_id: &str,
    message: &str,
    run_id: &str,
    channel: &str,
    account_id: &str,
) -> AgentRunRequest {
    AgentRunRequest::new(
        thread_id,
        message,
        run_id,
        channel,
        account_id,
        HashMap::new(),
    )
}

fn make_history(store: Arc<dyn ThreadStore>) -> Arc<ThreadHistoryRepository> {
    Arc::new(ThreadHistoryRepository::new(
        store,
        Arc::new(ThreadTranscriptStore::memory()),
    ))
}

/// The client-visible transcript is the committed jsonl. Under F1 the worker
/// streams finalized rows into the committed transcript during the run, so
/// mid-run state must be read through the repository.
async fn combined_thread_messages(
    history: &Arc<ThreadHistoryRepository>,
    thread_id: &str,
) -> Vec<serde_json::Value> {
    history
        .thread_snapshot(thread_id, 1024)
        .await
        .map(|snapshot| {
            snapshot
                .combined_messages()
                .into_iter()
                .filter(|message| {
                    message.get("kind").and_then(serde_json::Value::as_str) != Some("control")
                        && message
                            .get("internal_kind")
                            .and_then(serde_json::Value::as_str)
                            != Some("control")
                })
                .collect()
        })
        .unwrap_or_default()
}

fn fork_thread_metadata(
    provider_type: ProviderType,
    parent_sdk_session_id: &str,
) -> serde_json::Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert(SDK_SESSION_FORK_METADATA_KEY.to_owned(), json!(true));
    metadata.insert(
        FORK_FROM_PROVIDER_TYPE_METADATA_KEY.to_owned(),
        serde_json::to_value(provider_type).unwrap(),
    );
    metadata.insert(
        FORK_FROM_SDK_SESSION_ID_METADATA_KEY.to_owned(),
        json!(parent_sdk_session_id),
    );
    serde_json::Value::Object(metadata)
}

fn custom_agent(
    agent_id: &str,
    display_name: &str,
    provider_type: ProviderType,
    model: &str,
    system_prompt: &str,
) -> CustomAgentProfile {
    CustomAgentProfile {
        agent_id: agent_id.to_owned(),
        display_name: display_name.to_owned(),
        provider_type,
        model: model.to_owned(),
        model_reasoning_effort: String::new(),
        model_service_tier: String::new(),
        provider_env: Default::default(),
        default_workspace_dir: None,
        avatar_data_url: None,
        system_prompt: system_prompt.to_owned(),
        built_in: false,
        enabled: true,
        standalone: true,
        created_at: "2026-04-19T00:00:00Z".to_owned(),
        updated_at: "2026-04-19T00:00:00Z".to_owned(),
    }
}

fn agent_snapshot(agents: Vec<CustomAgentProfile>) -> AgentAvailabilitySnapshot {
    AgentAvailabilitySnapshot {
        agents,
        default_agent_id: None,
        agent_state_revision: 1,
    }
}

#[tokio::test]
async fn older_agent_profile_revision_cannot_overwrite_newer_snapshot() {
    let bridge = MultiProviderBridge::new();
    let mut current = custom_agent(
        "reviewer",
        "Reviewer v2",
        ProviderType::CodexAppServer,
        "gpt-5-v2",
        "Review v2.",
    );
    current.updated_at = "2026-07-16T02:00:00Z".to_owned();
    assert!(
        bridge
            .replace_agent_profiles(AgentAvailabilitySnapshot {
                agents: vec![current.clone()],
                default_agent_id: Some("reviewer".to_owned()),
                agent_state_revision: 2,
            })
            .await
    );
    let mut stale = current;
    stale.display_name = "Reviewer v1".to_owned();
    stale.model = "gpt-5-v1".to_owned();
    assert!(
        !bridge
            .replace_agent_profiles(AgentAvailabilitySnapshot {
                agents: vec![stale],
                default_agent_id: None,
                agent_state_revision: 1,
            })
            .await
    );
    let applied = bridge.agent_profile("reviewer").await.unwrap();
    assert_eq!(applied.display_name, "Reviewer v2");
    assert_eq!(applied.model, "gpt-5-v2");
}

/// A mock provider for testing.
struct MockProvider {
    ready: AtomicBool,
    ptype: ProviderType,
    image_counts: std::sync::Mutex<Vec<usize>>,
    metadata_snapshots: std::sync::Mutex<Vec<HashMap<String, serde_json::Value>>>,
    workspace_dirs: std::sync::Mutex<Vec<Option<String>>>,
    run_delay_ms: u64,
    runtime_selection: ProviderRuntimeSelection,
}

impl MockProvider {
    fn new(ptype: ProviderType) -> Self {
        Self {
            ready: AtomicBool::new(true),
            ptype,
            image_counts: std::sync::Mutex::new(Vec::new()),
            metadata_snapshots: std::sync::Mutex::new(Vec::new()),
            workspace_dirs: std::sync::Mutex::new(Vec::new()),
            run_delay_ms: 0,
            runtime_selection: ProviderRuntimeSelection::default(),
        }
    }

    fn with_delay(ptype: ProviderType, run_delay_ms: u64) -> Self {
        Self {
            ready: AtomicBool::new(true),
            ptype,
            image_counts: std::sync::Mutex::new(Vec::new()),
            metadata_snapshots: std::sync::Mutex::new(Vec::new()),
            workspace_dirs: std::sync::Mutex::new(Vec::new()),
            run_delay_ms,
            runtime_selection: ProviderRuntimeSelection::default(),
        }
    }

    fn with_runtime_selection(
        ptype: ProviderType,
        model: Option<&str>,
        reasoning_effort: Option<&str>,
        service_tier: Option<&str>,
    ) -> Self {
        Self {
            ready: AtomicBool::new(true),
            ptype,
            image_counts: std::sync::Mutex::new(Vec::new()),
            metadata_snapshots: std::sync::Mutex::new(Vec::new()),
            workspace_dirs: std::sync::Mutex::new(Vec::new()),
            run_delay_ms: 0,
            runtime_selection: ProviderRuntimeSelection {
                model: model.map(ToOwned::to_owned),
                model_reasoning_effort: reasoning_effort.map(ToOwned::to_owned),
                model_service_tier: service_tier.map(ToOwned::to_owned),
            },
        }
    }

    fn image_counts(&self) -> Vec<usize> {
        self.image_counts.lock().unwrap().clone()
    }

    fn workspace_dirs(&self) -> Vec<Option<String>> {
        self.workspace_dirs.lock().unwrap().clone()
    }

    fn metadata_snapshots(&self) -> Vec<HashMap<String, serde_json::Value>> {
        self.metadata_snapshots.lock().unwrap().clone()
    }
}

struct ClearSessionProvider {
    ptype: ProviderType,
    cleared_sessions: std::sync::Mutex<Vec<String>>,
    should_clear: AtomicBool,
}

struct CheckpointingProvider {
    delta_sent: Arc<Notify>,
    release_run: Arc<Notify>,
}

struct SegmentedResponseProvider;

struct CapsuleStreamingProvider;

struct FailingCheckpointProvider {
    delta_sent: Arc<Notify>,
}

struct EmptyResponseProvider;

struct FailedResultProvider;

struct PanickingProvider;

struct TitleProvider {
    title: String,
}

struct QueuedInputProvider {
    delta_sent: Arc<Notify>,
    follow_up_received: Arc<Notify>,
    allow_ack: Arc<Notify>,
    release_run: Arc<Notify>,
    queue_tx: mpsc::UnboundedSender<QueuedUserInput>,
    queue_rx: Mutex<Option<mpsc::UnboundedReceiver<QueuedUserInput>>>,
    received_inputs: std::sync::Mutex<Vec<QueuedUserInput>>,
}

struct DelayedQueuedInputProvider {
    delta_sent: Arc<Notify>,
    follow_up_received: Arc<Notify>,
    release_run: Arc<Notify>,
    queue_tx: mpsc::UnboundedSender<QueuedUserInput>,
    queue_rx: Mutex<Option<mpsc::UnboundedReceiver<QueuedUserInput>>>,
    accept_inputs: AtomicBool,
    add_attempts: AtomicUsize,
    run_invocations: AtomicUsize,
}

struct InterruptingFollowUpProvider {
    entered_run: Arc<Notify>,
    release_run: Arc<Notify>,
    run_invocations: AtomicUsize,
    abort_count: AtomicUsize,
    active_runs: AtomicUsize,
    max_concurrent_runs: AtomicUsize,
}

struct PreemptiveAbortProvider {
    entered_run: Arc<Notify>,
    active: Arc<AtomicBool>,
    abort_observed_active_transport: AtomicBool,
}

impl PreemptiveAbortProvider {
    fn new() -> Self {
        Self {
            entered_run: Arc::new(Notify::new()),
            active: Arc::new(AtomicBool::new(false)),
            abort_observed_active_transport: AtomicBool::new(false),
        }
    }
}

struct PreemptiveRunGuard(Arc<AtomicBool>);

impl Drop for PreemptiveRunGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for PreemptiveAbortProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GrokBuild
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        _options: &ProviderRunOptions,
        _on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        self.active.store(true, Ordering::SeqCst);
        let _guard = PreemptiveRunGuard(Arc::clone(&self.active));
        self.entered_run.notify_one();
        std::future::pending::<()>().await;
        unreachable!()
    }

    fn abort_before_task_cancel(&self) -> bool {
        true
    }

    async fn abort(&self, _run_id: &str) -> bool {
        self.abort_observed_active_transport
            .store(self.active.load(Ordering::SeqCst), Ordering::SeqCst);
        true
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
    }
}

impl ClearSessionProvider {
    fn new(ptype: ProviderType, should_clear: bool) -> Self {
        Self {
            ptype,
            cleared_sessions: std::sync::Mutex::new(Vec::new()),
            should_clear: AtomicBool::new(should_clear),
        }
    }

    fn cleared_sessions(&self) -> Vec<String> {
        self.cleared_sessions.lock().unwrap().clone()
    }
}

impl CheckpointingProvider {
    fn new() -> Self {
        Self {
            delta_sent: Arc::new(Notify::new()),
            release_run: Arc::new(Notify::new()),
        }
    }

    fn delta_sent(&self) -> Arc<Notify> {
        self.delta_sent.clone()
    }

    fn release_run(&self) {
        self.release_run.notify_waiters();
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for SegmentedResponseProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::CodexAppServer
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        on_chunk(StreamEvent::Delta {
            text: "polling review status".to_owned(),
        });
        on_chunk(StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        });
        on_chunk(StreamEvent::Delta {
            text: "final implementation summary".to_owned(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "segmented-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "polling review status\n\nfinal implementation summary".to_owned(),
            session_messages: vec![
                ProviderMessage::assistant_text("polling review status"),
                ProviderMessage::assistant_text("final implementation summary"),
            ],
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            duration_ms: 1,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
    }
}

impl FailingCheckpointProvider {
    fn new() -> Self {
        Self {
            delta_sent: Arc::new(Notify::new()),
        }
    }

    fn delta_sent(&self) -> Arc<Notify> {
        self.delta_sent.clone()
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for PanickingProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        _options: &ProviderRunOptions,
        _on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        panic!("injected provider panic")
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

impl TitleProvider {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
        }
    }
}

impl QueuedInputProvider {
    fn new() -> Self {
        let (queue_tx, queue_rx) = mpsc::unbounded_channel();
        Self {
            delta_sent: Arc::new(Notify::new()),
            follow_up_received: Arc::new(Notify::new()),
            allow_ack: Arc::new(Notify::new()),
            release_run: Arc::new(Notify::new()),
            queue_tx,
            queue_rx: Mutex::new(Some(queue_rx)),
            received_inputs: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn delta_sent(&self) -> Arc<Notify> {
        self.delta_sent.clone()
    }

    fn follow_up_received(&self) -> Arc<Notify> {
        self.follow_up_received.clone()
    }

    fn release_ack(&self) {
        self.allow_ack.notify_waiters();
    }

    fn release_run(&self) {
        self.release_run.notify_waiters();
    }

    fn received_inputs(&self) -> Vec<QueuedUserInput> {
        self.received_inputs.lock().unwrap().clone()
    }
}

impl DelayedQueuedInputProvider {
    fn new() -> Self {
        let (queue_tx, queue_rx) = mpsc::unbounded_channel();
        Self {
            delta_sent: Arc::new(Notify::new()),
            follow_up_received: Arc::new(Notify::new()),
            release_run: Arc::new(Notify::new()),
            queue_tx,
            queue_rx: Mutex::new(Some(queue_rx)),
            accept_inputs: AtomicBool::new(false),
            add_attempts: AtomicUsize::new(0),
            run_invocations: AtomicUsize::new(0),
        }
    }

    fn delta_sent(&self) -> Arc<Notify> {
        self.delta_sent.clone()
    }

    fn follow_up_received(&self) -> Arc<Notify> {
        self.follow_up_received.clone()
    }

    fn release_run(&self) {
        self.release_run.notify_waiters();
    }

    fn add_attempts(&self) -> usize {
        self.add_attempts.load(Ordering::Relaxed)
    }

    fn run_invocations(&self) -> usize {
        self.run_invocations.load(Ordering::Relaxed)
    }
}

impl InterruptingFollowUpProvider {
    fn new() -> Self {
        Self {
            entered_run: Arc::new(Notify::new()),
            release_run: Arc::new(Notify::new()),
            run_invocations: AtomicUsize::new(0),
            abort_count: AtomicUsize::new(0),
            active_runs: AtomicUsize::new(0),
            max_concurrent_runs: AtomicUsize::new(0),
        }
    }

    fn entered_run(&self) -> Arc<Notify> {
        self.entered_run.clone()
    }

    fn release_run(&self) {
        self.release_run.notify_waiters();
    }

    fn run_invocations(&self) -> usize {
        self.run_invocations.load(Ordering::Relaxed)
    }

    fn abort_count(&self) -> usize {
        self.abort_count.load(Ordering::Relaxed)
    }

    fn max_concurrent_runs(&self) -> usize {
        self.max_concurrent_runs.load(Ordering::Relaxed)
    }
}

struct ActiveRunCounter<'a> {
    provider: &'a InterruptingFollowUpProvider,
}

impl<'a> ActiveRunCounter<'a> {
    fn new(provider: &'a InterruptingFollowUpProvider) -> Self {
        let current = provider.active_runs.fetch_add(1, Ordering::SeqCst) + 1;
        loop {
            let previous = provider.max_concurrent_runs.load(Ordering::SeqCst);
            if current <= previous {
                break;
            }
            if provider
                .max_concurrent_runs
                .compare_exchange(previous, current, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }
        Self { provider }
    }
}

impl Drop for ActiveRunCounter<'_> {
    fn drop(&mut self) {
        self.provider.active_runs.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for MockProvider {
    fn provider_type(&self) -> ProviderType {
        self.ptype.clone()
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        self.ready.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        self.ready.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        if self.run_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.run_delay_ms)).await;
        }
        self.image_counts
            .lock()
            .unwrap()
            .push(options.images.as_ref().map_or(0, Vec::len));
        self.metadata_snapshots
            .lock()
            .unwrap()
            .push(options.metadata.clone());
        self.workspace_dirs
            .lock()
            .unwrap()
            .push(options.workspace_dir.clone());
        let response = format!("echo: {}", options.message);
        on_chunk(StreamEvent::Delta {
            text: response.clone(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "mock-run".into(),
            thread_id: options.thread_id.clone(),
            response,
            session_messages: Vec::new(),
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 10,
            output_tokens: 5,
            cost: 0.001,
            duration_ms: 42,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }

    fn resolve_runtime_selection(&self, _options: &ProviderRunOptions) -> ProviderRuntimeSelection {
        self.runtime_selection.clone()
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for TitleProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        on_chunk(StreamEvent::Delta {
            text: "titled response".to_owned(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "title-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "titled response".to_owned(),
            session_messages: vec![ProviderMessage::assistant_text("titled response")],
            sdk_session_id: None,
            actual_model: None,
            thread_title: Some(self.title.clone()),
            success: true,
            error: None,
            input_tokens: 1,
            output_tokens: 1,
            cost: 0.0,
            duration_ms: 1,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for ClearSessionProvider {
    fn provider_type(&self) -> ProviderType {
        self.ptype.clone()
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        _options: &ProviderRunOptions,
        _on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        Err(BridgeError::Internal(
            "not used in clear-session test".to_owned(),
        ))
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }

    async fn clear_session(&self, session_key: &str) -> ClearSessionOutcome {
        self.cleared_sessions
            .lock()
            .unwrap()
            .push(session_key.to_owned());
        if self.should_clear.load(Ordering::Relaxed) {
            ClearSessionOutcome::Cleared
        } else {
            ClearSessionOutcome::RetryableFailure
        }
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for CheckpointingProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let sdk_session_id = format!("sdk-{}", options.thread_id);
        on_chunk(StreamEvent::SessionBound {
            sdk_session_id: sdk_session_id.clone(),
        });
        on_chunk(StreamEvent::Delta {
            text: "partial reply".to_owned(),
        });
        self.delta_sent.notify_waiters();
        self.release_run.notified().await;
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "checkpoint-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "partial reply".to_owned(),
            session_messages: vec![ProviderMessage::assistant_text("partial reply")],
            sdk_session_id: Some(sdk_session_id),
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 10,
            output_tokens: 5,
            cost: 0.001,
            duration_ms: 42,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for QueuedInputProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let sdk_session_id = format!("sdk-{}", options.thread_id);
        on_chunk(StreamEvent::Delta {
            text: "initial reply".to_owned(),
        });
        self.delta_sent.notify_waiters();

        let queued_input = {
            let mut queue_rx = self.queue_rx.lock().await;
            let Some(queue_rx) = queue_rx.as_mut() else {
                return Err(BridgeError::SessionError(
                    "queued input receiver already consumed".to_owned(),
                ));
            };
            queue_rx.recv().await.ok_or_else(|| {
                BridgeError::SessionError("queued input receiver closed".to_owned())
            })?
        };
        self.received_inputs
            .lock()
            .unwrap()
            .push(queued_input.clone());
        self.follow_up_received.notify_waiters();
        self.allow_ack.notified().await;
        on_chunk(StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: queued_input.pending_input_id.clone(),
        });
        on_chunk(StreamEvent::Delta {
            text: format!("follow-up reply: {}", queued_input.message),
        });
        self.release_run.notified().await;
        on_chunk(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id: "queued-input-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: format!("initial replyfollow-up reply: {}", queued_input.message),
            session_messages: vec![
                ProviderMessage::assistant_text("initial reply"),
                ProviderMessage::assistant_text(format!(
                    "follow-up reply: {}",
                    queued_input.message
                )),
            ],
            sdk_session_id: Some(sdk_session_id),
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 10,
            output_tokens: 5,
            cost: 0.001,
            duration_ms: 42,
        })
    }

    async fn add_streaming_input(&self, _thread_id: &str, input: QueuedUserInput) -> bool {
        self.queue_tx.send(input).is_ok()
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for CapsuleStreamingProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let tool_use = ProviderMessage::tool_use(
            json!({"tool": "mcp__garyx__capsule_create", "input": {"title": "Test Capsule"}}),
            Some("toolu_fixture_capsule_create".to_owned()),
            Some("mcp__garyx__capsule_create".to_owned()),
        );
        let tool_result = ProviderMessage::tool_result(
            json!({
                "result": [{
                    "type": "text",
                    "text": "{\"tool\":\"capsule_create\",\"status\":\"ok\",\"capsule_id\":\"01900000-0000-7000-8000-000000000006\",\"id\":\"01900000-0000-7000-8000-000000000006\",\"title\":\"Provider Stream Capsule\",\"revision\":1,\"open_url\":\"garyx://capsules/01900000-0000-7000-8000-000000000006\"}"
                }],
                "text": ""
            }),
            Some("toolu_fixture_capsule_create".to_owned()),
            None,
            Some(false),
        );
        let final_message = ProviderMessage::assistant_text("final answer after capsule");
        on_chunk(StreamEvent::ToolUse {
            message: tool_use.clone(),
        });
        on_chunk(StreamEvent::ToolResult {
            message: tool_result.clone(),
        });
        on_chunk(StreamEvent::Delta {
            text: "final answer after capsule".to_owned(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "provider-capsule-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "final answer after capsule".to_owned(),
            session_messages: vec![tool_use, tool_result, final_message],
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 10,
            output_tokens: 5,
            cost: 0.001,
            duration_ms: 42,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for FailingCheckpointProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        _options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let tool_use = ProviderMessage::tool_use(
            json!({
                "kind": "think",
                "title": "Delegating to agent 'generalist'",
            }),
            Some("generalist-1".to_owned()),
            Some("Delegating to agent 'generalist'".to_owned()),
        );
        on_chunk(StreamEvent::Delta {
            text: "partial reply".to_owned(),
        });
        on_chunk(StreamEvent::ToolUse { message: tool_use });
        self.delta_sent.notify_waiters();
        Err(BridgeError::Timeout)
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for EmptyResponseProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "empty-response-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: String::new(),
            session_messages: Vec::new(),
            sdk_session_id: Some(format!("sdk-{}", options.thread_id)),
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 1,
            output_tokens: 0,
            cost: 0.0,
            duration_ms: 1,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for FailedResultProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        on_chunk(StreamEvent::Delta {
            text: "I'll continue by editing files.".to_owned(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "failed-result-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "I'll continue by editing files.".to_owned(),
            session_messages: vec![ProviderMessage::assistant_text(
                "I'll continue by editing files.",
            )],
            sdk_session_id: Some(format!("sdk-{}", options.thread_id)),
            actual_model: None,
            thread_title: None,
            success: false,
            error: Some("process interrupted".to_owned()),
            input_tokens: 1,
            output_tokens: 1,
            cost: 0.0,
            duration_ms: 1,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for DelayedQueuedInputProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let sdk_session_id = format!("sdk-{}", options.thread_id);
        let invocation = self.run_invocations.fetch_add(1, Ordering::Relaxed);
        on_chunk(StreamEvent::Delta {
            text: "initial reply".to_owned(),
        });
        self.delta_sent.notify_waiters();

        if invocation > 0 {
            on_chunk(StreamEvent::Done);
            return Ok(ProviderRunResult {
                run_id: format!("unexpected-run-{invocation}"),
                thread_id: options.thread_id.clone(),
                response: format!("unexpected second run: {}", options.message),
                session_messages: vec![ProviderMessage::assistant_text(format!(
                    "unexpected second run: {}",
                    options.message
                ))],
                sdk_session_id: Some(sdk_session_id.clone()),
                actual_model: None,
                thread_title: None,
                success: true,
                error: None,
                input_tokens: 10,
                output_tokens: 5,
                cost: 0.001,
                duration_ms: 42,
            });
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        self.accept_inputs.store(true, Ordering::Relaxed);

        let queued_input = {
            let mut queue_rx = self.queue_rx.lock().await;
            let Some(queue_rx) = queue_rx.as_mut() else {
                return Err(BridgeError::SessionError(
                    "delayed queued input receiver already consumed".to_owned(),
                ));
            };
            queue_rx.recv().await.ok_or_else(|| {
                BridgeError::SessionError("delayed queued input receiver closed".to_owned())
            })?
        };
        self.follow_up_received.notify_waiters();
        on_chunk(StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: queued_input.pending_input_id.clone(),
        });
        on_chunk(StreamEvent::Delta {
            text: format!("follow-up reply: {}", queued_input.message),
        });
        self.release_run.notified().await;
        on_chunk(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id: "delayed-queued-input-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: format!("initial replyfollow-up reply: {}", queued_input.message),
            session_messages: vec![
                ProviderMessage::assistant_text("initial reply"),
                ProviderMessage::assistant_text(format!(
                    "follow-up reply: {}",
                    queued_input.message
                )),
            ],
            sdk_session_id: Some(sdk_session_id),
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 10,
            output_tokens: 5,
            cost: 0.001,
            duration_ms: 42,
        })
    }

    async fn add_streaming_input(&self, _thread_id: &str, input: QueuedUserInput) -> bool {
        self.add_attempts.fetch_add(1, Ordering::Relaxed);
        if !self.accept_inputs.load(Ordering::Relaxed) {
            return false;
        }
        self.queue_tx.send(input).is_ok()
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

#[async_trait::async_trait]
impl ProviderRuntime for InterruptingFollowUpProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::AntigravityCli
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let _active_run = ActiveRunCounter::new(self);
        self.run_invocations.fetch_add(1, Ordering::SeqCst);
        self.entered_run.notify_waiters();
        on_chunk(StreamEvent::Delta {
            text: format!("echo: {}", options.message),
        });
        self.release_run.notified().await;
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: format!("interrupting-run-{}", self.run_invocations()),
            thread_id: options.thread_id.clone(),
            response: format!("echo: {}", options.message),
            session_messages: vec![ProviderMessage::assistant_text(format!(
                "echo: {}",
                options.message
            ))],
            sdk_session_id: Some(format!("sdk-{}", options.thread_id)),
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 10,
            output_tokens: 5,
            cost: 0.001,
            duration_ms: 42,
        })
    }

    async fn abort(&self, _run_id: &str) -> bool {
        self.abort_count.fetch_add(1, Ordering::SeqCst);
        true
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

#[tokio::test]
async fn test_register_and_get_provider() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));

    bridge
        .register_provider("claude:abc", provider.clone())
        .await;

    let got = bridge.get_provider("claude:abc").await;
    assert!(got.is_some());
    assert_eq!(got.unwrap().provider_type(), ProviderType::ClaudeCode);

    assert!(bridge.get_provider("nonexistent").await.is_none());
}

#[tokio::test]
async fn test_resolve_provider_thread_affinity() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));

    bridge.register_provider("p1", provider).await;
    bridge.set_thread_affinity("sess::a::b", "p1").await;

    let resolved = bridge
        .resolve_provider_for_thread("sess::a::b", "telegram", "main")
        .await;
    assert_eq!(resolved, Some("p1".to_owned()));
}

#[tokio::test]
async fn test_resolve_provider_route_cache() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::CodexAppServer));

    bridge.register_provider("codex:xyz", provider).await;
    bridge.set_route("telegram", "secondary", "codex:xyz").await;

    let resolved = bridge
        .resolve_provider_for_thread("unknown-session", "telegram", "secondary")
        .await;
    assert_eq!(resolved, Some("codex:xyz".to_owned()));
}

#[tokio::test]
async fn test_resolve_provider_default_fallback() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));

    bridge.register_provider("default", provider).await;
    bridge.set_default_provider_key("default").await;

    let resolved = bridge
        .resolve_provider_for_thread("any", "any", "any")
        .await;
    assert_eq!(resolved, Some("default".to_owned()));
}

#[tokio::test]
async fn built_in_agent_uses_configured_default_provider_model() {
    let bridge = MultiProviderBridge::new();
    bridge
        .replace_agent_profiles(agent_snapshot(builtin_provider_agent_profiles()))
        .await;
    let mut config = GaryxConfig::default();
    config.agents.insert(
        "claude".to_owned(),
        json!({
            "provider_type": "claude_code",
            "default_model": "claude-opus-4-8",
            "model_reasoning_effort": "max"
        }),
    );

    bridge.reload_from_config(&config).await.unwrap();

    let provider_config = bridge
        .provider_config_for_agent("claude")
        .await
        .expect("built-in claude agent config");
    assert_eq!(provider_config.default_model, "claude-opus-4-8");
    assert_eq!(provider_config.model_reasoning_effort, "max");

    let legacy_provider_config = bridge
        .provider_config_for_agent("claude-tty")
        .await
        .expect("legacy built-in claude alias config");
    assert_eq!(legacy_provider_config.default_model, "claude-opus-4-8");
}

#[tokio::test]
async fn test_thread_bound_claude_code_agent_snapshots_env_on_shared_provider() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let thread_id = "thread::super-junior-env-snapshot";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "super-junior",
                "provider_type": "claude_code",
                "metadata": {}
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(history);

    let mut agent = custom_agent(
        "super-junior",
        "Super Junior",
        ProviderType::ClaudeCode,
        "claude-opus-4-8",
        "Use Claude Code through the configured proxy.",
    );
    agent.provider_env = HashMap::from([(
        "ANTHROPIC_BASE_URL".to_owned(),
        "http://127.0.0.1:15721".to_owned(),
    )]);
    bridge
        .replace_agent_profiles(agent_snapshot(vec![agent]))
        .await;

    let default_provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude_code", default_provider.clone())
        .await;
    bridge.set_default_provider_key("claude_code").await;

    bridge
        .start_agent_run(
            run_request(
                thread_id,
                "route through the shared Claude provider",
                "run-super-junior-env-snapshot",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-super-junior-env-snapshot").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run should finish");

    let snapshots = default_provider.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].get("agent_id").and_then(Value::as_str),
        Some("super-junior")
    );
    assert_eq!(
        snapshots[0]
            .get("provider_env")
            .and_then(Value::as_object)
            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
            .and_then(Value::as_str),
        Some("http://127.0.0.1:15721")
    );
    assert_eq!(
        bridge.thread_affinity_for(thread_id).await.as_deref(),
        Some("claude_code")
    );

    let stored = store.get(thread_id).await.unwrap().expect("stored thread");
    assert_eq!(
        stored
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("provider_env"))
            .and_then(Value::as_object)
            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
            .and_then(Value::as_str),
        Some("http://127.0.0.1:15721")
    );
}

#[tokio::test]
async fn test_resolve_provider_none_when_empty() {
    let bridge = MultiProviderBridge::new();
    let resolved = bridge
        .resolve_provider_for_thread("any", "any", "any")
        .await;
    assert_eq!(resolved, None);
}

#[tokio::test]
async fn test_resolve_provider_for_request_prefers_requested_type() {
    let bridge = MultiProviderBridge::new();
    let claude = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    let codex = Arc::new(MockProvider::new(ProviderType::CodexAppServer));

    bridge.register_provider("claude-default", claude).await;
    bridge.register_provider("codex-default", codex).await;
    bridge.set_default_provider_key("claude-default").await;

    let resolved = bridge
        .resolve_provider_for_request(
            "sess::provider-override",
            "api",
            "main",
            Some(ProviderType::CodexAppServer),
        )
        .await;
    assert_eq!(resolved, Some("codex-default".to_owned()));
}

#[tokio::test]
async fn test_start_agent_run_streams_capsule_attached_control_from_tool_result() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(history.clone());
    bridge
        .register_provider("p1", Arc::new(CapsuleStreamingProvider))
        .await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request(
                "thread::capsule-stream",
                "create capsule",
                "run-capsule-stream",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-capsule-stream").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("capsule stream run should finish");

    let records = history
        .transcript_store()
        .records("thread::capsule-stream")
        .await
        .expect("records load");
    let control_kinds = records
        .iter()
        .filter_map(|record| {
            record
                .message
                .pointer("/control/kind")
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        control_kinds,
        vec!["run_start", "capsule_attached", "done", "run_complete"]
    );
    let capsule_index = records
        .iter()
        .position(|record| {
            record
                .message
                .pointer("/control/kind")
                .and_then(Value::as_str)
                == Some("capsule_attached")
        })
        .expect("capsule marker committed");
    assert_eq!(
        records[capsule_index].message["control"]["capsule_id"],
        "01900000-0000-7000-8000-000000000006"
    );
    assert_eq!(
        records[capsule_index].message["control"]["title"],
        "Provider Stream Capsule"
    );
    assert_eq!(records[capsule_index].message["control"]["revision"], 1);
    assert_eq!(
        records[capsule_index].message["control"]["action"],
        "created"
    );
    assert_eq!(
        records[capsule_index.saturating_sub(1)].message["role"],
        "tool_result",
        "marker should be committed immediately after the successful capsule tool_result"
    );
    assert_eq!(
        records[capsule_index + 1].message["role"],
        "assistant",
        "marker should precede the final assistant content in the authoritative run records"
    );
}

#[tokio::test]
async fn test_start_run_treats_legacy_claude_tty_request_as_claude_code() {
    let bridge = MultiProviderBridge::new();
    let claude_sdk = Arc::new(MockProvider::new(ProviderType::ClaudeCode));

    bridge
        .register_provider("claude_code", claude_sdk.clone())
        .await;
    bridge.set_default_provider_key("claude_code").await;

    let metadata = HashMap::from([("requested_provider_type".to_owned(), json!("claude_tty"))]);
    bridge
        .start_agent_run(
            AgentRunRequest::new(
                "sess::claude-tty-metadata",
                "use claude",
                "run-claude-tty-metadata",
                "api",
                "main",
                metadata,
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert_eq!(claude_sdk.metadata_snapshots().len(), 1);
}

#[tokio::test]
async fn test_resolve_priority_order() {
    let bridge = MultiProviderBridge::new();
    let p1 = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    let p2 = Arc::new(MockProvider::new(ProviderType::CodexAppServer));

    bridge.register_provider("p1", p1).await;
    bridge.register_provider("p2", p2).await;

    // Set both route and affinity.
    bridge.set_route("tg", "main", "p2").await;
    bridge.set_thread_affinity("sess", "p1").await;
    bridge.set_default_provider_key("p2").await;

    // Session affinity wins over route cache.
    let resolved = bridge
        .resolve_provider_for_thread("sess", "tg", "main")
        .await;
    assert_eq!(resolved, Some("p1".to_owned()));
}

#[tokio::test]
async fn test_provider_keys() {
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));

    bridge.register_provider("a", p.clone()).await;
    bridge.register_provider("b", p).await;

    let mut keys = bridge.provider_keys().await;
    keys.sort();
    assert_eq!(keys, vec!["a", "b"]);
}

#[tokio::test]
async fn test_shutdown_clears_state() {
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;
    bridge.set_route("tg", "main", "p1").await;
    bridge.set_thread_affinity("sess", "p1").await;

    bridge.shutdown().await;

    assert!(bridge.provider_keys().await.is_empty());
    assert!(bridge.default_provider_key().await.is_none());
    assert!(bridge.get_provider("p1").await.is_none());
}

#[tokio::test]
async fn test_initialize_succeeds() {
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p).await;
    assert!(bridge.initialize().await.is_ok());
}

#[tokio::test]
async fn test_reload_from_config_removes_disabled_routes() {
    let bridge = MultiProviderBridge::new();
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            telegram_account_to_plugin_entry(&TelegramAccount {
                token: "token".to_owned(),
                enabled: true,
                name: None,
                agent_id: Some("claude".to_owned()),
                workspace_dir: None,
                owner_target: None,
                groups: HashMap::new(),
            }),
        );

    bridge.reload_from_config(&config).await.unwrap();
    let resolved_enabled = bridge
        .resolve_provider_for_thread("sess::x", "telegram", "main")
        .await;
    assert!(resolved_enabled.is_some());

    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .get_mut("main")
        .unwrap()
        .enabled = false;
    bridge.reload_from_config(&config).await.unwrap();

    let resolved_disabled = bridge
        .resolve_provider_for_thread("sess::x", "telegram", "main")
        .await;
    let default_key = bridge.default_provider_key().await;
    assert_eq!(resolved_disabled, default_key);
}

#[tokio::test]
async fn run_inline_streaming_rederives_target_metadata_and_persists_history() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude-child", provider.clone())
        .await;
    bridge
        .replace_agent_profiles(agent_snapshot(vec![custom_agent(
            "coder",
            "Coder",
            ProviderType::ClaudeCode,
            "claude-opus-child",
            "You are the coder child.",
        )]))
        .await;

    store
        .set(
            "thread::child-coder",
            json!({
                "thread_id": "thread::child-coder",
                "agent_id": "coder",
                "provider_type": "claude_code",
                "metadata": {
                    "agent_id": "coder",
                    "agent_display_name": "Coder",
                    "system_prompt": "You are the coder child.",
                    "model": "claude-opus-child",
                    "requested_provider_type": "claude_code"
                }
            }),
        )
        .await
        .unwrap();

    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("caller")),
        ("agent_display_name".to_owned(), json!("Caller")),
        ("system_prompt".to_owned(), json!("caller prompt")),
        ("model".to_owned(), json!("caller-model")),
        ("channel".to_owned(), json!("telegram")),
    ]);

    let streamed = Arc::new(std::sync::Mutex::new(String::new()));
    let streamed_cb = streamed.clone();
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        if let StreamEvent::Delta { text } = event {
            streamed_cb.lock().unwrap().push_str(&text);
        }
    });

    let result = bridge
        .run_inline_streaming(
            "thread::child-coder",
            "hello child",
            metadata,
            None,
            None,
            Some(callback),
        )
        .await
        .expect("inline run should succeed");

    assert_eq!(result.response, "echo: hello child");
    assert_eq!(*streamed.lock().unwrap(), "echo: hello child");

    let snapshots = provider.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    let snapshot = &snapshots[0];
    assert_eq!(
        snapshot.get("agent_id").and_then(|v| v.as_str()),
        Some("coder")
    );
    assert_eq!(
        snapshot.get("agent_display_name").and_then(|v| v.as_str()),
        Some("Coder")
    );
    assert_eq!(
        snapshot.get("system_prompt").and_then(|v| v.as_str()),
        Some("You are the coder child.")
    );
    assert_eq!(
        snapshot.get("model").and_then(|v| v.as_str()),
        Some("claude-opus-child")
    );
    let thread_data = store
        .get("thread::child-coder")
        .await
        .unwrap()
        .expect("thread data should exist");
    assert_eq!(
        thread_data.get("provider_key").and_then(|v| v.as_str()),
        Some("claude-child")
    );

    let snapshot = bridge
        .thread_history()
        .await
        .expect("thread history")
        .thread_snapshot("thread::child-coder", 10)
        .await
        .expect("child history");
    let combined: Vec<_> = snapshot
        .combined_messages()
        .into_iter()
        .filter(|message| {
            message.get("kind").and_then(serde_json::Value::as_str) != Some("control")
        })
        .collect();
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[0]["role"], "user");
    assert_eq!(combined[0]["content"], "hello child");
    assert_eq!(combined[1]["role"], "assistant");
    assert_eq!(combined[1]["content"], "echo: hello child");
}

#[tokio::test]
async fn run_inline_streaming_uses_global_run_limiter() {
    let bridge = Arc::new(MultiProviderBridge::new_with_max_concurrent_runs(1));
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;

    let provider = Arc::new(MockProvider::with_delay(ProviderType::ClaudeCode, 120));
    bridge
        .register_provider("claude-child", provider.clone())
        .await;
    bridge
        .replace_agent_profiles(agent_snapshot(vec![custom_agent(
            "coder",
            "Coder",
            ProviderType::ClaudeCode,
            "claude-opus-child",
            "You are the coder child.",
        )]))
        .await;

    for thread_id in ["thread::child-one", "thread::child-two"] {
        store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "agent_id": "coder",
                    "provider_type": "claude_code",
                    "metadata": {
                        "agent_id": "coder",
                        "requested_provider_type": "claude_code"
                    }
                }),
            )
            .await
            .unwrap();
    }

    let first_bridge = bridge.clone();
    let first = tokio::spawn(async move {
        first_bridge
            .run_inline_streaming(
                "thread::child-one",
                "first",
                HashMap::new(),
                None,
                None,
                None,
            )
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(bridge.available_run_slots(), 0);

    let second = bridge
        .run_inline_streaming(
            "thread::child-two",
            "second",
            HashMap::new(),
            None,
            None,
            None,
        )
        .await;
    assert!(matches!(second, Err(BridgeError::Overloaded(_))));

    first
        .await
        .expect("first task joins")
        .expect("first run succeeds");
    assert_eq!(bridge.available_run_slots(), 1);
}

#[tokio::test]
async fn test_start_and_complete_run() {
    let bridge = MultiProviderBridge::new();
    let mut lifecycle = bridge.subscribe_run_lifecycle();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    let result = bridge
        .start_agent_run(
            run_request("sess::tg::123", "hello", "run-1", "telegram", "main"),
            None,
        )
        .await;
    assert!(result.is_ok());

    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(3), lifecycle.recv())
            .await
            .expect("run start lifecycle timeout")
            .expect("run lifecycle channel"),
        RunLifecycleEvent::Started {
            thread_id: "sess::tg::123".to_owned(),
            run_id: "run-1".to_owned(),
        }
    );
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(3), lifecycle.recv())
            .await
            .expect("run terminal lifecycle timeout")
            .expect("run lifecycle channel"),
        RunLifecycleEvent::Terminal {
            thread_id: "sess::tg::123".to_owned(),
            run_id: "run-1".to_owned(),
        }
    );

    // Run should have been cleaned up.
    assert!(!bridge.is_run_active("run-1").await);
}

#[tokio::test]
async fn provider_title_update_is_forwarded_after_persistence_without_delaying_done() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));
    bridge
        .register_provider(
            "title-provider",
            Arc::new(TitleProvider::new("Provider Generated Title")),
        )
        .await;
    bridge.set_default_provider_key("title-provider").await;
    store
        .set(
            "thread::title-event",
            json!({
                "thread_id": "thread::title-event",
                "label": "Please summarize this request",
                "thread_title_source": "garyx_prompt"
            }),
        )
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let done = Arc::new(Notify::new());
    let title_seen = Arc::new(Notify::new());
    let events_for_callback = events.clone();
    let done_for_callback = done.clone();
    let title_for_callback = title_seen.clone();
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        if matches!(event, StreamEvent::Done) {
            done_for_callback.notify_one();
        }
        if matches!(event, StreamEvent::ThreadTitleUpdated { .. }) {
            title_for_callback.notify_one();
        }
        events_for_callback.lock().unwrap().push(event);
    });

    bridge
        .start_agent_run(
            run_request(
                "thread::title-event",
                "hello",
                "run-title-event",
                "api",
                "main",
            ),
            Some(callback),
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), done.notified())
        .await
        .expect("done should be forwarded");
    tokio::time::timeout(std::time::Duration::from_secs(2), title_seen.notified())
        .await
        .expect("title update should be forwarded after persistence");

    let updated = store
        .get("thread::title-event")
        .await
        .unwrap()
        .expect("thread exists");
    assert_eq!(updated["label"], "Provider Generated Title");
    assert_eq!(updated["thread_title_source"], "provider");

    let events = events.lock().unwrap().clone();
    let title_index = events
        .iter()
        .position(|event| {
            matches!(
                event,
                StreamEvent::ThreadTitleUpdated { title } if title == "Provider Generated Title"
            )
        })
        .expect("title update event should be forwarded");
    let done_index = events
        .iter()
        .position(|event| matches!(event, StreamEvent::Done))
        .expect("done event should be forwarded");
    assert!(
        done_index < title_index,
        "done reflects provider stream completion and should not wait for title persistence"
    );
}

#[tokio::test]
async fn provider_title_update_is_not_forwarded_when_explicit_label_wins() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));
    bridge
        .register_provider(
            "title-provider",
            Arc::new(TitleProvider::new("Provider Generated Title")),
        )
        .await;
    bridge.set_default_provider_key("title-provider").await;
    store
        .set(
            "thread::explicit-title-event",
            json!({
                "thread_id": "thread::explicit-title-event",
                "label": "Human Title",
                "thread_title_source": "explicit"
            }),
        )
        .await
        .unwrap();

    let events = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let done = Arc::new(Notify::new());
    let events_for_callback = events.clone();
    let done_for_callback = done.clone();
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        if matches!(event, StreamEvent::Done) {
            done_for_callback.notify_one();
        }
        events_for_callback.lock().unwrap().push(event);
    });

    bridge
        .start_agent_run(
            run_request(
                "thread::explicit-title-event",
                "hello",
                "run-explicit-title-event",
                "api",
                "main",
            ),
            Some(callback),
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), done.notified())
        .await
        .expect("done should be forwarded");

    let updated = store
        .get("thread::explicit-title-event")
        .await
        .unwrap()
        .expect("thread exists");
    assert_eq!(updated["label"], "Human Title");
    assert!(updated.get("provider_thread_title").is_none());

    let events = events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, StreamEvent::ThreadTitleUpdated { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::Done))
    );
}

#[tokio::test]
async fn test_start_run_with_images_pass_through() {
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let images = vec![ImagePayload {
        name: "sample.png".to_owned(),
        data: "abc123".to_owned(),
        media_type: "image/png".to_owned(),
    }];

    let result = bridge
        .start_agent_run(
            run_request(
                "sess::tg::img",
                "describe image",
                "run-img-1",
                "telegram",
                "main",
            )
            .with_images(Some(images)),
            None,
        )
        .await;
    assert!(result.is_ok());

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let counts = p.image_counts();
    assert!(!counts.is_empty());
    assert_eq!(counts[0], 1);
}

#[tokio::test]
async fn test_start_run_binds_workspace_override() {
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p.clone()).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request("sess::workspace", "hello", "run-workspace-1", "api", "main")
                .with_workspace_dir(Some("/tmp/gary-workspace-a".to_owned())),
            None,
        )
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        p.workspace_dirs(),
        vec![Some("/tmp/gary-workspace-a".to_owned())]
    );
}

#[tokio::test]
async fn test_start_run_rejects_conflicting_workspace_override() {
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request(
                "sess::workspace-conflict",
                "first",
                "run-workspace-ok",
                "api",
                "main",
            )
            .with_workspace_dir(Some("/tmp/gary-workspace-a".to_owned())),
            None,
        )
        .await
        .unwrap();

    let second = bridge
        .start_agent_run(
            run_request(
                "sess::workspace-conflict",
                "second",
                "run-workspace-conflict",
                "api",
                "main",
            )
            .with_workspace_dir(Some("/tmp/gary-workspace-b".to_owned())),
            None,
        )
        .await;

    assert!(matches!(second, Err(BridgeError::SessionError(_))));
}

#[tokio::test]
async fn test_start_run_no_provider() {
    let bridge = MultiProviderBridge::new();

    let result = bridge
        .start_agent_run(
            run_request("sess", "hello", "run-1", "telegram", "main"),
            None,
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        BridgeError::ProviderNotFound(_) => {}
        other => panic!("expected ProviderNotFound, got: {other}"),
    }
}

#[tokio::test]
async fn test_start_run_rejected_when_bridge_overloaded() {
    let bridge = MultiProviderBridge::new_with_max_concurrent_runs(1);
    let p = Arc::new(MockProvider::with_delay(ProviderType::ClaudeCode, 400));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request("sess::first", "first", "run-1", "telegram", "main"),
            None,
        )
        .await
        .unwrap();

    let second = bridge
        .start_agent_run(
            run_request("sess::second", "second", "run-2", "telegram", "main"),
            None,
        )
        .await;

    assert!(matches!(second, Err(BridgeError::Overloaded(_))));
}

#[tokio::test]
async fn test_abort_run() {
    let bridge = MultiProviderBridge::new();
    let mut lifecycle = bridge.subscribe_run_lifecycle();
    let p = Arc::new(MockProvider::with_delay(ProviderType::ClaudeCode, 5_000));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request("sess", "hello", "run-1", "telegram", "main"),
            None,
        )
        .await
        .unwrap();

    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(3), lifecycle.recv())
            .await
            .expect("run start lifecycle timeout")
            .expect("run lifecycle channel"),
        RunLifecycleEvent::Started { ref run_id, .. } if run_id == "run-1"
    ));

    let aborted = bridge.abort_run("run-1").await;
    assert!(aborted);
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(3), lifecycle.recv())
            .await
            .expect("run terminal lifecycle timeout")
            .expect("run lifecycle channel"),
        RunLifecycleEvent::Terminal { ref run_id, .. } if run_id == "run-1"
    ));
}

#[tokio::test]
async fn test_abort_run_flushes_preemptive_provider_before_dropping_task() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(PreemptiveAbortProvider::new());
    let entered_run = Arc::clone(&provider.entered_run);
    bridge.register_provider("grok", provider.clone()).await;
    bridge.set_default_provider_key("grok").await;

    bridge
        .start_agent_run(
            run_request("sess", "hello", "run-preemptive", "api", "main"),
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), entered_run.notified())
        .await
        .expect("provider run should start");

    assert!(bridge.abort_run("run-preemptive").await);
    assert!(
        provider
            .abort_observed_active_transport
            .load(Ordering::SeqCst),
        "provider abort must run while its stdio transport task is alive"
    );
}

#[tokio::test]
async fn test_abort_run_persists_interrupted_terminal_control() {
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::with_delay(ProviderType::ClaudeCode, 5_000));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn garyx_router::ThreadStore> =
        Arc::new(garyx_router::InMemoryThreadStore::new());
    let history = make_history(store.clone());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(history.clone());

    bridge
        .start_agent_run(
            run_request("sess::abort", "hello", "run-abort", "telegram", "main"),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let records = history
                .transcript_store()
                .records("sess::abort")
                .await
                .unwrap();
            if records.iter().any(|record| {
                record
                    .message
                    .pointer("/control/kind")
                    .and_then(|value| value.as_str())
                    == Some("run_start")
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run_start control should be persisted before abort");

    assert!(bridge.abort_run("run-abort").await);

    let records = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let records = history
                .transcript_store()
                .records("sess::abort")
                .await
                .unwrap();
            if records.iter().any(|record| {
                record
                    .message
                    .pointer("/control/kind")
                    .and_then(|value| value.as_str())
                    == Some("run_complete")
            }) {
                break records;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("abort terminal control should be persisted");
    let terminal = records
        .iter()
        .find(|record| {
            record
                .message
                .pointer("/control/kind")
                .and_then(|value| value.as_str())
                == Some("run_complete")
        })
        .expect("run_complete control");
    assert_eq!(
        terminal
            .message
            .pointer("/control/status")
            .and_then(|value| value.as_str()),
        Some("interrupted")
    );
    assert_eq!(
        terminal
            .message
            .pointer("/control/error")
            .and_then(|value| value.as_str()),
        Some("aborted")
    );
    assert!(
        records
            .iter()
            .map(|record| record.seq)
            .eq(1..=records.len() as u64)
    );

    let run_state = history
        .transcript_store()
        .run_state("sess::abort")
        .await
        .expect("run state should reduce from committed controls");
    assert!(!run_state.busy);
}

#[tokio::test]
async fn test_thread_persistence_after_run() {
    // P0-C: after agent run, thread store should contain user+assistant messages.
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn garyx_router::ThreadStore> =
        Arc::new(garyx_router::InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    let history = make_history(store.clone());
    bridge.set_thread_history(history.clone());

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::persist",
                "hello bot",
                "run-persist",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    let messages = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let messages = combined_thread_messages(&history, "sess::tg::persist").await;
            if messages.len() >= 2 {
                break messages;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("session data should be persisted");
    let data = store
        .get("sess::tg::persist")
        .await
        .unwrap()
        .expect("thread record should exist");
    assert_eq!(data["provider_key"], "p1");
    assert!(
        data.get("messages").is_none(),
        "record messages snapshot is retired (#TASK-1864 batch 1c)"
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "hello bot");
    assert_eq!(messages[1]["role"], "assistant");
}

#[tokio::test]
async fn test_worker_emits_committed_message_events_with_seqs() {
    // S5: the persistence worker publishes each committed jsonl row as a seq'd
    // `committed_message` event on the gateway bus (write-then-emit).
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn garyx_router::ThreadStore> =
        Arc::new(garyx_router::InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(256);
    bridge.set_event_tx(tx).await;

    bridge
        .start_agent_run(
            run_request("sess::tg::s5", "hello bot", "run-s5", "telegram", "main"),
            None,
        )
        .await
        .unwrap();

    let mut committed: Vec<(u64, String)> = Vec::new();
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        while let Ok(raw) = rx.try_recv() {
            let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
            if value.get("type").and_then(serde_json::Value::as_str) == Some("committed_message") {
                let seq = value
                    .get("seq")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap();
                let label = value
                    .pointer("/message/control/kind")
                    .or_else(|| value.pointer("/message/role"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                if !committed.iter().any(|(s, _)| *s == seq) {
                    committed.push((seq, label));
                }
            }
        }
        if committed.len() >= 5 {
            break;
        }
    }
    committed.sort_by_key(|(seq, _)| *seq);
    assert_eq!(
        committed,
        vec![
            (1, "run_start".to_owned()),
            (2, "user".to_owned()),
            (3, "assistant".to_owned()),
            (4, "done".to_owned()),
            (5, "run_complete".to_owned())
        ],
        "committed_message events carry gapless seqs for control and content rows"
    );
    // The event carries the thread_id so the per-thread stream can filter.
    // (Implicitly covered: the gateway filters on thread_id; here we assert the
    // seq/role contract the client cursor depends on.)
}

#[tokio::test]
async fn done_callback_observes_done_control_record_committed() {
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn garyx_router::ThreadStore> =
        Arc::new(garyx_router::InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    let history = make_history(store.clone());
    bridge.set_thread_history(history.clone());

    let done = Arc::new(Notify::new());
    let callback_saw_done_control = Arc::new(AtomicBool::new(false));
    let history_for_callback = history.clone();
    let done_for_callback = done.clone();
    let seen_for_callback = callback_saw_done_control.clone();
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        if !matches!(event, StreamEvent::Done) {
            return;
        }

        let history = history_for_callback.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("callback inspection runtime");
            let saw_done = runtime.block_on(async move {
                history
                    .transcript_store()
                    .records_after_seq("thread::done-callback-order", 0, 128)
                    .await
                    .expect("transcript records")
                    .iter()
                    .any(|record| {
                        record
                            .message
                            .pointer("/control/kind")
                            .and_then(serde_json::Value::as_str)
                            == Some("done")
                    })
            });
            tx.send(saw_done).expect("send callback inspection result");
        });
        let saw_done = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("callback transcript inspection should finish");
        seen_for_callback.store(saw_done, Ordering::SeqCst);
        done_for_callback.notify_one();
    });

    bridge
        .start_agent_run(
            run_request(
                "thread::done-callback-order",
                "hello bot",
                "run-done-callback-order",
                "api",
                "main",
            ),
            Some(callback),
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), done.notified())
        .await
        .expect("done should be forwarded");
    assert!(
        callback_saw_done_control.load(Ordering::SeqCst),
        "Done callback must run only after control.kind=done is committed"
    );
}

#[tokio::test]
async fn test_thread_persistence_checkpoints_streaming_metadata_before_run_completion() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(CheckpointingProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    let history = make_history(store.clone());
    bridge.set_thread_history(history.clone());

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::checkpoint",
                "keep this",
                "run-checkpoint",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .expect("provider should emit a streamed delta");

    let checkpointed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let Some(data) = store.get("sess::tg::checkpoint").await.unwrap() else {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            };
            let messages = combined_thread_messages(&history, "sess::tg::checkpoint").await;
            if messages
                .iter()
                .any(|message| message["role"] == "user" && message["content"] == "keep this")
            {
                break data;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("thread store should receive a committed checkpoint before completion");

    let checkpoint_messages = combined_thread_messages(&history, "sess::tg::checkpoint").await;
    assert_eq!(checkpoint_messages.len(), 1);
    assert_eq!(checkpoint_messages[0]["role"], "user");
    assert_eq!(checkpoint_messages[0]["content"], "keep this");
    assert_eq!(checkpointed["sdk_session_id"], "sdk-sess::tg::checkpoint");
    assert_eq!(
        checkpointed["provider_sdk_session_ids"]["p1"],
        "sdk-sess::tg::checkpoint"
    );
    assert_eq!(checkpointed["history"]["message_count"], 2);

    provider.release_run();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-checkpoint").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("checkpoint run should fully complete after the provider finishes");

    let final_data = store
        .get("sess::tg::checkpoint")
        .await
        .unwrap()
        .expect("final thread data should exist");
    let final_messages = combined_thread_messages(&history, "sess::tg::checkpoint").await;
    assert_eq!(final_messages.len(), 2);
    assert_eq!(final_messages[1]["content"], "partial reply");
    assert_eq!(final_data["sdk_session_id"], "sdk-sess::tg::checkpoint");
    assert_eq!(
        final_data["provider_sdk_session_ids"]["p1"],
        "sdk-sess::tg::checkpoint"
    );
}

#[tokio::test]
async fn test_failed_run_commits_terminal_control_and_preserves_partial_messages() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(FailingCheckpointProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let now = Utc::now();
    let task = ThreadTask {
        schema_version: 1,
        number: 8,
        title: "Review failed checkpoint".to_owned(),
        status: TaskStatus::InProgress,
        creator: Principal::Human {
            user_id: "user42".to_owned(),
        },
        assignee: Some(Principal::Agent {
            agent_id: "codex".to_owned(),
        }),
        notification_target: None,
        executor: None,
        source: None,
        body: None,
        created_at: now,
        updated_at: now,
        updated_by: Principal::Agent {
            agent_id: "codex".to_owned(),
        },
        events: Vec::new(),
    };
    store
        .set(
            "sess::tg::failed-checkpoint",
            json!({
                "sdk_session_id": "sdk-existing",
                "provider_sdk_session_ids": {
                    "p1": "sdk-existing"
                },
                "task": task
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    let history = make_history(store.clone());
    bridge.set_thread_history(history.clone());

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::failed-checkpoint",
                "keep this failure",
                "run-failed-checkpoint",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .expect("provider should emit a streamed delta before failing");

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-failed-checkpoint").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("failed run should clean up its active state");

    let final_data = store
        .get("sess::tg::failed-checkpoint")
        .await
        .unwrap()
        .expect("failed thread data should exist");
    assert_eq!(final_data["sdk_session_id"], "sdk-existing");
    assert_eq!(final_data["provider_sdk_session_ids"]["p1"], "sdk-existing");
    assert_eq!(final_data["task"]["status"], "in_progress");

    let final_messages = combined_thread_messages(&history, "sess::tg::failed-checkpoint").await;
    assert_eq!(final_messages[0]["role"], "user");
    assert_eq!(final_messages[0]["content"], "keep this failure");
    assert!(
        final_messages
            .iter()
            .any(|message| message["role"] == "assistant" && message["content"] == "partial reply"),
        "partial assistant text should survive failure finalization"
    );
    assert!(
        final_messages
            .iter()
            .any(|message| message["role"] == "tool_use"
                && message["tool_name"] == "Delegating to agent 'generalist'"),
        "tool trace should survive failure finalization"
    );
}

#[tokio::test]
async fn test_empty_successful_task_run_does_not_move_to_review() {
    let bridge = MultiProviderBridge::new();
    bridge
        .register_provider("p1", Arc::new(EmptyResponseProvider))
        .await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let now = Utc::now();
    let task = ThreadTask {
        schema_version: 1,
        number: 9,
        title: "Do not review empty runs".to_owned(),
        status: TaskStatus::InProgress,
        creator: Principal::Human {
            user_id: "user42".to_owned(),
        },
        assignee: Some(Principal::Agent {
            agent_id: "codex".to_owned(),
        }),
        notification_target: None,
        executor: None,
        source: None,
        body: None,
        created_at: now,
        updated_at: now,
        updated_by: Principal::Agent {
            agent_id: "codex".to_owned(),
        },
        events: Vec::new(),
    };
    store
        .set(
            "sess::tg::empty-task",
            json!({
                "task": task
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::empty-task",
                "start empty task",
                "run-empty-task",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-empty-task").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("empty task run should finish");

    let final_data = store
        .get("sess::tg::empty-task")
        .await
        .unwrap()
        .expect("thread data should exist");
    assert_eq!(final_data["task"]["status"], "in_progress");
}

#[tokio::test]
async fn test_unsuccessful_task_run_with_partial_response_does_not_move_to_review() {
    let bridge = MultiProviderBridge::new();
    bridge
        .register_provider("p1", Arc::new(FailedResultProvider))
        .await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let now = Utc::now();
    let task = ThreadTask {
        schema_version: 1,
        number: 10,
        title: "Do not review interrupted runs".to_owned(),
        status: TaskStatus::InProgress,
        creator: Principal::Human {
            user_id: "user42".to_owned(),
        },
        assignee: Some(Principal::Agent {
            agent_id: "codex".to_owned(),
        }),
        notification_target: None,
        executor: None,
        source: None,
        body: None,
        created_at: now,
        updated_at: now,
        updated_by: Principal::Agent {
            agent_id: "codex".to_owned(),
        },
        events: Vec::new(),
    };
    store
        .set(
            "sess::tg::failed-result-task",
            json!({
                "task": task
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    let history = make_history(store.clone());
    bridge.set_thread_history(history.clone());

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::failed-result-task",
                "start failed result task",
                "run-failed-result-task",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-failed-result-task").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("failed result task run should finish");

    let final_data = store
        .get("sess::tg::failed-result-task")
        .await
        .unwrap()
        .expect("thread data should exist");
    assert_eq!(final_data["task"]["status"], "in_progress");
    let final_messages = combined_thread_messages(&history, "sess::tg::failed-result-task").await;
    assert!(
        final_messages
            .iter()
            .any(|message| message["role"] == "assistant"
                && message["content"] == "I'll continue by editing files."),
        "partial response should be preserved for diagnosis"
    );
}

#[tokio::test]
async fn test_segmented_successful_task_run_hands_off_only_final_answer_segment() {
    let bridge = MultiProviderBridge::new();
    bridge
        .register_provider("p1", Arc::new(SegmentedResponseProvider))
        .await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let now = Utc::now();
    let task = ThreadTask {
        schema_version: 1,
        number: 11,
        title: "Review segmented handoff".to_owned(),
        status: TaskStatus::InProgress,
        creator: Principal::Human {
            user_id: "1000000011".to_owned(),
        },
        assignee: Some(Principal::Agent {
            agent_id: "codex".to_owned(),
        }),
        notification_target: None,
        executor: None,
        source: None,
        body: None,
        created_at: now,
        updated_at: now,
        updated_by: Principal::Agent {
            agent_id: "codex".to_owned(),
        },
        events: Vec::new(),
    };
    let thread_id = "sess::tg::segmented-task";
    store
        .set(
            thread_id,
            json!({
                "task": task
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(128);
    bridge.set_event_tx(tx).await;

    bridge
        .start_agent_run(
            run_request(
                thread_id,
                "finish segmented task",
                "run-segmented-task",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-segmented-task").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("segmented task run should finish");

    let ready_event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let raw = rx.recv().await.expect("event channel should stay open");
            let event: serde_json::Value = serde_json::from_str(&raw).expect("event parses");
            if event.get("type").and_then(serde_json::Value::as_str)
                == Some("task_ready_for_review")
            {
                break event;
            }
        }
    })
    .await
    .expect("segmented task should emit a task-ready event");

    assert_eq!(ready_event["thread_id"], thread_id);
    assert_eq!(ready_event["task_id"], "#TASK-11");
    assert_eq!(ready_event["run_id"], "run-segmented-task");
    assert_eq!(ready_event["handoff"], "final implementation summary");
}

#[tokio::test]
async fn test_work_run_wake_revives_in_review_task_before_completion() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(CheckpointingProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let now = Utc::now();
    let task = ThreadTask {
        schema_version: 1,
        number: 12,
        title: "Revive reviewed task".to_owned(),
        status: TaskStatus::InReview,
        creator: Principal::Human {
            user_id: "user42".to_owned(),
        },
        assignee: Some(Principal::Agent {
            agent_id: "codex".to_owned(),
        }),
        notification_target: None,
        executor: None,
        source: None,
        body: None,
        created_at: now,
        updated_at: now,
        updated_by: Principal::Agent {
            agent_id: "codex".to_owned(),
        },
        events: vec![TaskEvent {
            event_id: "evt-ready-before-wake".to_owned(),
            at: now,
            actor: Principal::Agent {
                agent_id: "codex".to_owned(),
            },
            kind: TaskEventKind::StatusChanged {
                from: TaskStatus::InProgress,
                to: TaskStatus::InReview,
                note: Some("ready before wake".to_owned()),
            },
        }],
    };
    let thread_id = "sess::tg::wake-reviewed-task";
    store
        .set(
            thread_id,
            json!({
                "task": task
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(128);
    bridge.set_event_tx(tx).await;

    bridge
        .start_agent_run(
            run_request(
                thread_id,
                "continue reviewed task",
                "run-wake-reviewed",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .expect("provider should emit a partial response");

    let status_during_run = store
        .get(thread_id)
        .await
        .unwrap()
        .and_then(|data| data["task"]["status"].as_str().map(ToOwned::to_owned));

    provider.release_run();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-wake-reviewed").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("revived task run should finish");

    assert_eq!(status_during_run.as_deref(), Some("in_progress"));
    let final_data = store
        .get(thread_id)
        .await
        .unwrap()
        .expect("thread data should exist");
    assert_eq!(final_data["task"]["status"], "in_review");

    let ready_event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let raw = rx.recv().await.expect("event channel should stay open");
            let event: serde_json::Value = serde_json::from_str(&raw).expect("event parses");
            if event.get("type").and_then(serde_json::Value::as_str)
                == Some("task_ready_for_review")
            {
                break event;
            }
        }
    })
    .await
    .expect("revived task should emit a fresh task-ready event");

    assert_eq!(ready_event["thread_id"], thread_id);
    assert_eq!(ready_event["task_id"], "#TASK-12");
    assert_eq!(ready_event["run_id"], "run-wake-reviewed");
    assert_eq!(ready_event["handoff"], "partial reply");
}

#[tokio::test]
async fn test_work_run_wake_revives_done_task_before_completion() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(CheckpointingProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let now = Utc::now();
    let task = ThreadTask {
        schema_version: 1,
        number: 13,
        title: "Revive done task".to_owned(),
        status: TaskStatus::Done,
        creator: Principal::Human {
            user_id: "user42".to_owned(),
        },
        assignee: Some(Principal::Agent {
            agent_id: "codex".to_owned(),
        }),
        notification_target: None,
        executor: None,
        source: None,
        body: None,
        created_at: now,
        updated_at: now,
        updated_by: Principal::Human {
            user_id: "reviewer".to_owned(),
        },
        events: vec![
            TaskEvent {
                event_id: "evt-ready-before-done-wake".to_owned(),
                at: now,
                actor: Principal::Agent {
                    agent_id: "codex".to_owned(),
                },
                kind: TaskEventKind::StatusChanged {
                    from: TaskStatus::InProgress,
                    to: TaskStatus::InReview,
                    note: Some("ready before approval".to_owned()),
                },
            },
            TaskEvent {
                event_id: "evt-done-before-wake".to_owned(),
                at: now,
                actor: Principal::Human {
                    user_id: "reviewer".to_owned(),
                },
                kind: TaskEventKind::StatusChanged {
                    from: TaskStatus::InReview,
                    to: TaskStatus::Done,
                    note: Some("approved".to_owned()),
                },
            },
        ],
    };
    let thread_id = "sess::tg::wake-done-task";
    store
        .set(
            thread_id,
            json!({
                "task": task
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(128);
    bridge.set_event_tx(tx).await;

    bridge
        .start_agent_run(
            run_request(
                thread_id,
                "continue done task",
                "run-wake-done",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .expect("provider should emit a partial response");

    let status_during_run = store
        .get(thread_id)
        .await
        .unwrap()
        .and_then(|data| data["task"]["status"].as_str().map(ToOwned::to_owned));

    provider.release_run();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-wake-done").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("revived task run should finish");

    assert_eq!(status_during_run.as_deref(), Some("in_progress"));
    let final_data = store
        .get(thread_id)
        .await
        .unwrap()
        .expect("thread data should exist");
    assert_eq!(final_data["task"]["status"], "in_review");

    let ready_event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let raw = rx.recv().await.expect("event channel should stay open");
            let event: serde_json::Value = serde_json::from_str(&raw).expect("event parses");
            if event.get("type").and_then(serde_json::Value::as_str)
                == Some("task_ready_for_review")
            {
                break event;
            }
        }
    })
    .await
    .expect("revived task should emit a fresh task-ready event");

    assert_eq!(ready_event["thread_id"], thread_id);
    assert_eq!(ready_event["task_id"], "#TASK-13");
    assert_eq!(ready_event["run_id"], "run-wake-done");
    assert_eq!(ready_event["handoff"], "partial reply");
}

async fn assert_non_work_run_does_not_revive_task(
    thread_id: &str,
    run_id: &str,
    metadata: HashMap<String, serde_json::Value>,
    initial_status: TaskStatus,
    expected_status: &str,
) {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(CheckpointingProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let now = Utc::now();
    let task = ThreadTask {
        schema_version: 1,
        number: 14,
        title: "Do not revive from internal wake".to_owned(),
        status: initial_status,
        creator: Principal::Human {
            user_id: "user42".to_owned(),
        },
        assignee: Some(Principal::Agent {
            agent_id: "codex".to_owned(),
        }),
        notification_target: None,
        executor: None,
        source: None,
        body: None,
        created_at: now,
        updated_at: now,
        updated_by: Principal::Agent {
            agent_id: "codex".to_owned(),
        },
        events: Vec::new(),
    };
    store
        .set(
            thread_id,
            json!({
                "task": task
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

    bridge
        .start_agent_run(
            AgentRunRequest::new(
                thread_id,
                "internal wake should not revive task",
                run_id,
                "telegram",
                "main",
                metadata,
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .expect("provider should emit a partial response");

    let status_during_run = store
        .get(thread_id)
        .await
        .unwrap()
        .and_then(|data| data["task"]["status"].as_str().map(ToOwned::to_owned));
    assert_eq!(status_during_run.as_deref(), Some(expected_status));

    provider.release_run();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active(run_id).await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("non-work wake run should finish");

    let final_data = store
        .get(thread_id)
        .await
        .unwrap()
        .expect("thread data should exist");
    assert_eq!(final_data["task"]["status"], expected_status);
}

#[tokio::test]
async fn test_non_work_run_wake_does_not_revive_reviewed_or_done_task() {
    assert_non_work_run_does_not_revive_task(
        "sess::tg::notify-wake-reviewed-task",
        "task-notify-14",
        HashMap::new(),
        TaskStatus::InReview,
        "in_review",
    )
    .await;
    assert_non_work_run_does_not_revive_task(
        "sess::tg::internal-wake-done-task",
        "run-internal-dispatch",
        HashMap::from([("internal_dispatch".to_owned(), json!(true))]),
        TaskStatus::Done,
        "done",
    )
    .await;
    assert_non_work_run_does_not_revive_task(
        "sess::tg::system-wake-done-task",
        "run-system",
        HashMap::from([("system".to_owned(), json!(true))]),
        TaskStatus::Done,
        "done",
    )
    .await;
}

#[tokio::test]
async fn test_thread_persistence_promotes_queued_input_after_user_ack() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(QueuedInputProvider::new());
    let delta_sent = provider.delta_sent();
    let follow_up_received = provider.follow_up_received();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    let history = make_history(store.clone());
    bridge.set_thread_history(history.clone());

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::queued-input",
                "start run",
                "run-queued-input",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .expect("provider should emit the initial streamed delta");

    let queued = bridge
        .add_streaming_input(
            "sess::tg::queued-input",
            "follow-up",
            None,
            None,
            None,
            Some("00000000-0000-0000-0000-000000000001".to_owned()),
        )
        .await;
    assert!(
        queued.is_some(),
        "follow-up should queue into the active run"
    );

    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        follow_up_received.notified(),
    )
    .await
    .expect("provider should receive the queued follow-up");

    let pending_checkpoint = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let Some(data) = store.get("sess::tg::queued-input").await.unwrap() else {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            };
            let pending_inputs = data["pending_user_inputs"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let messages = combined_thread_messages(&history, "sess::tg::queued-input").await;
            let has_follow_up_user = messages
                .iter()
                .any(|message| message["role"] == "user" && message["content"] == "follow-up");
            if pending_inputs.len() == 1 && !has_follow_up_user {
                break data;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("pending input should checkpoint before the provider acks it");
    assert_eq!(
        pending_checkpoint["pending_user_inputs"][0]["text"],
        "follow-up"
    );
    assert_eq!(
        pending_checkpoint["pending_user_inputs"][0]["origin_id"],
        "00000000-0000-0000-0000-000000000001"
    );

    provider.release_ack();

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let Some(data) = store.get("sess::tg::queued-input").await.unwrap() else {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            };
            let pending_inputs = data["pending_user_inputs"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let messages = combined_thread_messages(&history, "sess::tg::queued-input").await;
            let has_follow_up_user = messages
                .iter()
                .any(|message| message["role"] == "user" && message["content"] == "follow-up");
            let has_follow_up_assistant = messages.iter().any(|message| {
                message["role"] == "assistant" && message["content"] == "follow-up reply: follow-up"
            });
            if pending_inputs.is_empty() && has_follow_up_user && !has_follow_up_assistant {
                break data;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("user_ack should promote the queued follow-up user row into the persisted transcript");

    let acked_messages = combined_thread_messages(&history, "sess::tg::queued-input").await;
    let roles: Vec<&str> = acked_messages
        .iter()
        .filter_map(|message| message["role"].as_str())
        .collect();
    assert_eq!(roles, vec!["user", "assistant", "user"]);
    let follow_up_user = acked_messages
        .iter()
        .find(|message| message["role"] == "user" && message["content"] == "follow-up")
        .expect("follow-up user should be committed");
    assert_eq!(
        follow_up_user["metadata"]["origin_id"],
        "00000000-0000-0000-0000-000000000001"
    );
    assert_eq!(
        follow_up_user["metadata"]["queued_input_id"],
        pending_checkpoint["pending_user_inputs"][0]["id"]
    );

    provider.release_run();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-queued-input").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("queued-input run should fully complete after the provider finishes");

    let final_data = store
        .get("sess::tg::queued-input")
        .await
        .unwrap()
        .expect("final thread data should exist");
    assert_eq!(
        final_data["pending_user_inputs"]
            .as_array()
            .map(|items| items.len())
            .unwrap_or(0),
        0
    );
    assert_eq!(final_data["sdk_session_id"], "sdk-sess::tg::queued-input");
    assert_eq!(
        final_data["provider_sdk_session_ids"]["p1"],
        "sdk-sess::tg::queued-input"
    );
    let final_messages = combined_thread_messages(&history, "sess::tg::queued-input").await;
    let final_roles: Vec<&str> = final_messages
        .iter()
        .filter_map(|message| message["role"].as_str())
        .collect();
    assert_eq!(final_roles, vec!["user", "assistant", "user", "assistant"]);
}

#[derive(Clone, Copy)]
enum QueuedDispatchPath {
    Legacy,
    DurableExact,
}

async fn assert_busy_dispatch_persists_full_metadata(path: QueuedDispatchPath) {
    let suffix = match path {
        QueuedDispatchPath::Legacy => "legacy",
        QueuedDispatchPath::DurableExact => "durable",
    };
    let thread_id = format!("thread::queued-metadata-{suffix}");
    let active_run_id = format!("run::active-{suffix}");
    let requested_run_id = format!("run::notify-{suffix}");
    let message = format!("notification-{suffix}");

    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(QueuedInputProvider::new());
    let delta_sent = provider.delta_sent();
    let follow_up_received = provider.follow_up_received();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store.set(&thread_id, json!({})).await.unwrap();
    bridge.set_thread_store(store.clone()).await;
    let history = make_history(store.clone());
    bridge.set_thread_history(history.clone());

    let first = AdmittedRun::thread_bound(
        store.clone(),
        run_request(&thread_id, "active run", &active_run_id, "api", "main"),
    )
    .await
    .unwrap();
    assert_eq!(
        bridge.dispatch(first, None).await.unwrap(),
        garyx_models::provider::AgentDispatchOutcome::Started
    );
    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .expect("provider should emit the initial delta");

    let mut request = run_request(&thread_id, &message, &requested_run_id, "api", "main");
    request.metadata = HashMap::from([
        ("internal_dispatch".to_owned(), json!(true)),
        ("source".to_owned(), json!("task_notification")),
        (
            "task_notification".to_owned(),
            json!({
                "event": "ready_for_review",
                "status": "in_review",
                "task_id": "#TASK-42",
                "title": "Synthetic notification"
            }),
        ),
    ]);
    let source_family_metadata = HashMap::from([
        ("automation_id".to_owned(), json!("automation-1")),
        ("cron_job_id".to_owned(), json!("cron-1")),
        ("cron_action".to_owned(), json!("run")),
        ("task_auto_start".to_owned(), json!(true)),
        ("task_dispatch_reason".to_owned(), json!("created")),
        ("restart_wake".to_owned(), json!(true)),
        ("restart_wake_id".to_owned(), json!("wake-1")),
    ]);
    request.metadata.extend(source_family_metadata.clone());
    for key in super::persistence::RUNTIME_ONLY_METADATA_KEYS {
        request
            .metadata
            .insert((*key).to_owned(), json!(format!("sentinel-{key}")));
    }
    let second = AdmittedRun::thread_bound(store.clone(), request)
        .await
        .unwrap();
    let expected_pending_id = format!("pending::{suffix}");
    let outcome = match path {
        QueuedDispatchPath::Legacy => bridge.dispatch(second, None).await.unwrap(),
        QueuedDispatchPath::DurableExact => {
            let plan = bridge
                .prepare_durable_dispatch(second, expected_pending_id.clone())
                .await
                .unwrap();
            assert_eq!(plan.effective_run_id(), active_run_id);
            bridge.execute_durable_dispatch(plan, None).await.unwrap()
        }
    };
    let pending_input_id = match outcome {
        garyx_models::provider::AgentDispatchOutcome::QueuedToActiveRun {
            effective_run_id,
            pending_input_id,
        } => {
            assert_eq!(effective_run_id, active_run_id);
            pending_input_id
        }
        other => panic!("expected real queued outcome, got {other:?}"),
    };
    if matches!(path, QueuedDispatchPath::DurableExact) {
        assert_eq!(pending_input_id, expected_pending_id);
    }

    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        follow_up_received.notified(),
    )
    .await
    .expect("provider should receive queued notification");

    let pending = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let data = store.get(&thread_id).await.unwrap().unwrap_or_default();
            if let Some(pending) = data["pending_user_inputs"]
                .as_array()
                .and_then(|inputs| inputs.iter().find(|input| input["id"] == pending_input_id))
            {
                break pending.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("queued notification should persist before provider ACK");
    assert_eq!(pending["bridge_run_id"], active_run_id);
    assert_eq!(pending["metadata"]["internal_dispatch"], true);
    assert_eq!(pending["metadata"]["source"], "task_notification");
    assert_eq!(
        pending["metadata"]["task_notification"]["task_id"],
        "#TASK-42"
    );
    assert_eq!(pending["metadata"]["origin_run_id"], requested_run_id);
    for (key, value) in &source_family_metadata {
        assert_eq!(
            pending["metadata"].get(key),
            Some(value),
            "busy queue lost internal-source key {key}: {pending}"
        );
    }
    for key in super::persistence::RUNTIME_ONLY_METADATA_KEYS {
        assert!(
            pending["metadata"].get(*key).is_none(),
            "runtime key {key} persisted in pending input: {pending}"
        );
    }

    provider.release_ack();
    let committed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let messages = combined_thread_messages(&history, &thread_id).await;
            if let Some(message) = messages
                .iter()
                .find(|candidate| candidate["role"] == "user" && candidate["content"] == message)
            {
                break message.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("provider ACK should commit queued notification");
    assert_eq!(committed["metadata"]["internal_dispatch"], true);
    assert_eq!(
        committed["metadata"]["task_notification"]["title"],
        "Synthetic notification"
    );
    assert_eq!(committed["metadata"]["origin_run_id"], requested_run_id);
    assert_ne!(committed["metadata"]["origin_run_id"], active_run_id);
    assert_eq!(committed["metadata"]["queued_input_id"], pending_input_id);
    for (key, value) in &source_family_metadata {
        assert_eq!(
            committed["metadata"].get(key),
            Some(value),
            "ACK commit lost internal-source key {key}: {committed}"
        );
    }
    for key in super::persistence::RUNTIME_ONLY_METADATA_KEYS {
        assert!(committed["metadata"].get(*key).is_none());
    }

    provider.release_run();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active(&active_run_id).await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("active run should finish");
}

#[tokio::test]
async fn legacy_busy_dispatch_persists_full_metadata_before_and_after_ack() {
    assert_busy_dispatch_persists_full_metadata(QueuedDispatchPath::Legacy).await;
}

#[tokio::test]
async fn durable_exact_busy_dispatch_persists_full_metadata_before_and_after_ack() {
    assert_busy_dispatch_persists_full_metadata(QueuedDispatchPath::DurableExact).await;
}

#[tokio::test]
async fn test_start_agent_run_preserves_metadata_attachments_for_active_stream_follow_up() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(QueuedInputProvider::new());
    let delta_sent = provider.delta_sent();
    let follow_up_received = provider.follow_up_received();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    let history = make_history(store.clone());
    bridge.set_thread_history(history.clone());

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::queued-attachment",
                "start run",
                "run-queued-attachment-initial",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .expect("provider should emit the initial streamed delta");

    let attachment = PromptAttachment {
        attachment_id: None,
        kind: PromptAttachmentKind::File,
        path: "/tmp/garyx-test/inbound/spec.txt".to_owned(),
        name: "spec.txt".to_owned(),
        media_type: "text/plain".to_owned(),
    };
    let mut follow_up = run_request(
        "sess::tg::queued-attachment",
        "<media:file>",
        "run-queued-attachment-follow-up",
        "telegram",
        "main",
    );
    follow_up.metadata.insert(
        ATTACHMENTS_METADATA_KEY.to_owned(),
        attachments_to_metadata_value(std::slice::from_ref(&attachment)),
    );

    bridge.start_agent_run(follow_up, None).await.unwrap();

    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        follow_up_received.notified(),
    )
    .await
    .expect("provider should receive the queued follow-up");

    let received_inputs = provider.received_inputs();
    assert_eq!(
        received_inputs.len(),
        1,
        "follow-up should queue into the active provider run"
    );
    assert_eq!(received_inputs[0].message, "<media:file>");
    assert_eq!(received_inputs[0].attachments, vec![attachment]);

    provider.release_ack();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let messages = combined_thread_messages(&history, "sess::tg::queued-attachment").await;
            if messages.iter().any(|message| {
                message["role"] == "user"
                    && message["content"]
                        .as_array()
                        .is_some_and(|blocks| !blocks.is_empty())
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("provider should ack and commit the queued attachment follow-up user row");

    provider.release_run();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-queued-attachment-initial").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("queued attachment run should fully complete after the provider finishes");
}

#[tokio::test]
async fn test_streaming_input_preserves_raw_task_follow_up_for_provider() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(QueuedInputProvider::new());
    let delta_sent = provider.delta_sent();
    let follow_up_received = provider.follow_up_received();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let now = Utc::now();
    let task = ThreadTask {
        schema_version: 1,
        number: 7,
        title: "Verify queued task metadata".to_owned(),
        status: TaskStatus::InProgress,
        creator: Principal::Human {
            user_id: "user42".to_owned(),
        },
        assignee: Some(Principal::Agent {
            agent_id: "codex".to_owned(),
        }),
        notification_target: None,
        executor: None,
        source: None,
        body: None,
        created_at: now,
        updated_at: now,
        updated_by: Principal::Agent {
            agent_id: "codex".to_owned(),
        },
        events: Vec::new(),
    };
    store
        .set(
            "sess::tg::queued-task",
            json!({
                "thread_id": "sess::tg::queued-task",
                "channel": "telegram",
                "account_id": "codex_bot",
                "from_id": "user42",
                "agent_id": "codex",
                "task": task,
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    let history = make_history(store.clone());
    bridge.set_thread_history(history.clone());

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::queued-task",
                "start run",
                "run-queued-task",
                "telegram",
                "codex_bot",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .expect("provider should emit the initial streamed delta");

    let queued = bridge
        .add_streaming_input("sess::tg::queued-task", "继续", None, None, None, None)
        .await;
    assert!(
        queued.is_some(),
        "follow-up should queue into the active task run"
    );

    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        follow_up_received.notified(),
    )
    .await
    .expect("provider should receive the queued task follow-up");

    provider.release_ack();

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if store.get("sess::tg::queued-task").await.unwrap().is_none() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            };
            let messages = combined_thread_messages(&history, "sess::tg::queued-task").await;
            let has_raw_user = messages
                .iter()
                .any(|message| message["role"] == "user" && message["content"] == "继续");
            let has_provider_reply = messages.iter().any(|message| {
                message["role"] == "assistant" && message["content"] == "follow-up reply: 继续"
            });
            if has_raw_user && !has_provider_reply {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("provider-facing queued input should commit raw task follow-up after ack");

    let messages = combined_thread_messages(&history, "sess::tg::queued-task").await;
    assert!(
        !messages.iter().any(|message| message["role"] == "user"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("[task "))),
        "persisted user turn should keep the raw user text"
    );

    provider.release_run();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-queued-task").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("queued task run should fully complete after the provider finishes");

    let final_data = store
        .get("sess::tg::queued-task")
        .await
        .unwrap()
        .expect("final task thread data should exist");
    assert_eq!(final_data["task"]["status"], "in_review");
}

#[tokio::test]
async fn test_add_streaming_input_retries_until_provider_ready() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(DelayedQueuedInputProvider::new());
    let delta_sent = provider.delta_sent();
    let follow_up_received = provider.follow_up_received();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::delayed-queued-input",
                "start run",
                "run-delayed-queued-input",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(1), delta_sent.notified())
        .await
        .expect("provider should emit the initial streamed delta");

    let queued = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        bridge
            .add_streaming_input(
                "sess::tg::delayed-queued-input",
                "follow-up",
                None,
                None,
                None,
                None,
            )
            .await
    })
    .await
    .expect("queued input retry should finish before timeout");
    assert!(
        queued.is_some(),
        "bridge should retry until the active provider run can accept input"
    );

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        follow_up_received.notified(),
    )
    .await
    .expect("provider should eventually receive the queued follow-up");

    assert_eq!(
        provider.run_invocations(),
        1,
        "queued input should stay on the original run"
    );
    assert!(
        provider.add_attempts() > 1,
        "bridge should retry add_streaming_input while the provider run becomes ready"
    );

    provider.release_run();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn test_start_agent_run_retries_follow_up_into_active_stream() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(DelayedQueuedInputProvider::new());
    let delta_sent = provider.delta_sent();
    let follow_up_received = provider.follow_up_received();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::delayed-follow-up",
                "start run",
                "run-delayed-follow-up-1",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(1), delta_sent.notified())
        .await
        .expect("provider should emit the initial streamed delta");

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        bridge
            .start_agent_run(
                run_request(
                    "sess::tg::delayed-follow-up",
                    "follow-up",
                    "run-delayed-follow-up-2",
                    "telegram",
                    "main",
                ),
                None,
            )
            .await
    })
    .await
    .expect("follow-up dispatch should finish before timeout")
    .unwrap();

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        follow_up_received.notified(),
    )
    .await
    .expect("follow-up should land on the existing streaming run");

    assert_eq!(
        provider.run_invocations(),
        1,
        "follow-up dispatch should not start a second provider run"
    );
    assert!(
        provider.add_attempts() > 1,
        "follow-up dispatch should retry until the provider run can accept queued input"
    );

    provider.release_run();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn test_concurrent_start_agent_run_serializes_same_thread_follow_up() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(DelayedQueuedInputProvider::new());
    let follow_up_received = provider.follow_up_received();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let first = bridge.start_agent_run(
        run_request(
            "sess::tg::concurrent-follow-up",
            "start run",
            "run-concurrent-follow-up-1",
            "telegram",
            "main",
        ),
        None,
    );
    let second = bridge.start_agent_run(
        run_request(
            "sess::tg::concurrent-follow-up",
            "follow-up",
            "run-concurrent-follow-up-2",
            "telegram",
            "main",
        ),
        None,
    );

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let (first_result, second_result) = tokio::join!(first, second);
        first_result.unwrap();
        second_result.unwrap();
    })
    .await
    .expect("concurrent dispatches should finish before timeout");

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        follow_up_received.notified(),
    )
    .await
    .expect("second concurrent dispatch should be queued into the first run");

    assert_eq!(
        provider.run_invocations(),
        1,
        "concurrent same-thread dispatches should not start two provider runs"
    );

    provider.release_run();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn test_concurrent_start_agent_run_interrupts_non_streaming_follow_up() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(InterruptingFollowUpProvider::new());
    let entered_run = provider.entered_run();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::interrupt-follow-up",
                "start run",
                "run-interrupt-follow-up-1",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while provider.run_invocations() < 1 {
            entered_run.notified().await;
        }
    })
    .await
    .expect("first run should start");

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::interrupt-follow-up",
                "follow-up",
                "run-interrupt-follow-up-2",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while provider.run_invocations() < 2 {
            entered_run.notified().await;
        }
    })
    .await
    .expect("replacement run should start after interrupting the first run");

    assert_eq!(
        provider.max_concurrent_runs(),
        1,
        "same-thread follow-up must not start a second non-streaming run concurrently"
    );
    assert!(
        provider.abort_count() >= 1,
        "non-streaming follow-up should interrupt the in-flight run before restarting"
    );

    provider.release_run();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn durable_dispatch_interrupts_non_streaming_follow_up_instead_of_becoming_ambiguous() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(InterruptingFollowUpProvider::new());
    let entered_run = provider.entered_run();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let thread_id = "thread::durable-interrupt-follow-up";
    store.set(thread_id, json!({})).await.unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

    let first = AdmittedRun::thread_bound(
        store.clone(),
        run_request(
            thread_id,
            "start run",
            "run-durable-interrupt-follow-up-1",
            "api",
            "main",
        ),
    )
    .await
    .unwrap();
    bridge.dispatch(first, None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while provider.run_invocations() < 1 {
            entered_run.notified().await;
        }
    })
    .await
    .expect("first run should start");

    let stream_input_plan = bridge
        .prepare_durable_stream_input(
            thread_id.to_owned(),
            "follow-up".to_owned(),
            None,
            None,
            None,
            Some("intent::durable-interrupt".to_owned()),
            "pending::durable-stream-interrupt".to_owned(),
        )
        .await;
    assert!(
        stream_input_plan.effective_run_id().is_none(),
        "direct stream-input must let the caller fall back when the active provider cannot queue"
    );
    assert!(
        bridge
            .execute_durable_stream_input(stream_input_plan)
            .await
            .unwrap()
            .is_none()
    );

    let second = AdmittedRun::thread_bound(
        store.clone(),
        run_request(
            thread_id,
            "follow-up",
            "run-durable-interrupt-follow-up-2",
            "api",
            "main",
        ),
    )
    .await
    .unwrap();
    let plan = bridge
        .prepare_durable_dispatch(second, "pending::durable-interrupt".to_owned())
        .await
        .unwrap();
    let outcome = bridge
        .execute_durable_dispatch(plan, None)
        .await
        .expect("non-streaming follow-up must have a deterministic replacement outcome");
    assert_eq!(
        outcome,
        garyx_models::provider::AgentDispatchOutcome::Started
    );

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while provider.run_invocations() < 2 {
            entered_run.notified().await;
        }
    })
    .await
    .expect("replacement run should start after interrupting the first run");
    assert_eq!(provider.max_concurrent_runs(), 1);
    assert!(provider.abort_count() >= 1);

    provider.release_run();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn test_start_run_restores_provider_scoped_sdk_session_id_from_thread_store() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "sess::provider-sdk",
            serde_json::json!({
                "provider_key": "claude-local",
                "sdk_session_id": "legacy-session",
                "provider_sdk_session_ids": {
                    "claude-local": "claude-session",
                    "codex-local": "codex-thread"
                }
            }),
        )
        .await
        .unwrap();

    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude-local", provider.clone())
        .await;
    bridge.set_default_provider_key("claude-local").await;
    bridge.set_thread_store(store).await;

    bridge
        .start_agent_run(
            run_request(
                "sess::provider-sdk",
                "resume claude",
                "run-provider-sdk",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let snapshots = provider.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].get("sdk_session_id"),
        Some(&serde_json::Value::String("claude-session".to_owned()))
    );
}

#[tokio::test]
async fn test_start_run_restores_thread_bound_sdk_session_id_by_provider_type() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "sess::thread-provider-type",
            serde_json::json!({
                "provider_type": "claude_code",
                "provider_key": "claude_code:old-config-hash",
                "sdk_session_id": "thread-bound-session"
            }),
        )
        .await
        .unwrap();

    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude_code", provider.clone())
        .await;
    bridge.set_default_provider_key("claude_code").await;
    bridge.set_thread_store(store).await;

    bridge
        .start_agent_run(
            run_request(
                "sess::thread-provider-type",
                "resume from thread binding",
                "run-thread-provider-type",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let snapshots = provider.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].get("sdk_session_id"),
        Some(&serde_json::Value::String(
            "thread-bound-session".to_owned()
        ))
    );
}

#[tokio::test]
async fn test_start_run_uses_fork_parent_sdk_session_before_child_session_exists() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "sess::side-fork",
            serde_json::json!({
                "provider_type": "claude_code",
                "metadata": fork_thread_metadata(
                    ProviderType::ClaudeCode,
                    "parent-claude-session",
                )
            }),
        )
        .await
        .unwrap();

    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude_code", provider.clone())
        .await;
    bridge.set_default_provider_key("claude_code").await;
    bridge.set_thread_store(store).await;

    bridge
        .start_agent_run(
            run_request(
                "sess::side-fork",
                "side question",
                "run-side-fork",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let snapshots = provider.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].get(SDK_SESSION_ID_METADATA_KEY),
        Some(&serde_json::Value::String(
            "parent-claude-session".to_owned()
        ))
    );
    assert_eq!(
        snapshots[0]
            .get(SDK_SESSION_FORK_METADATA_KEY)
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[tokio::test]
async fn test_start_run_resumes_child_sdk_session_after_fork_child_is_bound() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "sess::side-fork-child",
            serde_json::json!({
                "provider_type": "claude_code",
                "sdk_session_id": "child-claude-session",
                "metadata": fork_thread_metadata(
                    ProviderType::ClaudeCode,
                    "parent-claude-session",
                )
            }),
        )
        .await
        .unwrap();

    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude_code", provider.clone())
        .await;
    bridge.set_default_provider_key("claude_code").await;
    bridge.set_thread_store(store).await;

    bridge
        .start_agent_run(
            run_request(
                "sess::side-fork-child",
                "follow up",
                "run-side-fork-child",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let snapshots = provider.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].get(SDK_SESSION_ID_METADATA_KEY),
        Some(&serde_json::Value::String(
            "child-claude-session".to_owned()
        ))
    );
    assert!(
        !snapshots[0].contains_key(SDK_SESSION_FORK_METADATA_KEY),
        "child session should resume normally instead of forking every turn"
    );
}

#[tokio::test]
async fn test_start_run_restores_claude_sdk_session_id_from_legacy_tty_record() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "sess::claude-backend-switch",
            serde_json::json!({
                "provider_type": "claude_code",
                "provider_key": "claude_code",
                "sdk_session_id": "claude-native-session"
            }),
        )
        .await
        .unwrap();

    let bridge = MultiProviderBridge::new();
    let claude_sdk = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude_code", claude_sdk.clone())
        .await;
    bridge.set_default_provider_key("claude_code").await;
    bridge.set_thread_store(store).await;

    bridge
        .start_agent_run(
            run_request(
                "sess::claude-backend-switch",
                "resume with claude",
                "run-claude-backend-switch",
                "api",
                "main",
            )
            .with_requested_provider(ProviderType::from_slug("claude_tty")),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let snapshots = claude_sdk.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].get("sdk_session_id"),
        Some(&serde_json::Value::String(
            "claude-native-session".to_owned()
        ))
    );
}

#[tokio::test]
async fn test_start_run_restores_claude_sdk_session_id_when_legacy_tty_was_default() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "sess::claude-sdk-switch",
            serde_json::json!({
                "provider_type": "claude_tty",
                "provider_key": "claude_tty",
                "sdk_session_id": "claude-native-session"
            }),
        )
        .await
        .unwrap();

    let bridge = MultiProviderBridge::new();
    let claude_sdk = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude_code", claude_sdk.clone())
        .await;
    bridge.set_default_provider_key("claude_code").await;
    bridge.set_thread_store(store).await;

    bridge
        .start_agent_run(
            run_request(
                "sess::claude-sdk-switch",
                "resume with the sdk backend",
                "run-claude-sdk-switch",
                "api",
                "main",
            )
            .with_requested_provider(Some(ProviderType::ClaudeCode)),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let snapshots = claude_sdk.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].get("sdk_session_id"),
        Some(&serde_json::Value::String(
            "claude-native-session".to_owned()
        ))
    );
}

#[tokio::test]
async fn test_start_run_ignores_legacy_sdk_session_id_from_other_provider() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "sess::provider-mismatch",
            serde_json::json!({
                "provider_key": "codex-local",
                "sdk_session_id": "codex-thread",
            }),
        )
        .await
        .unwrap();

    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude-local", provider.clone())
        .await;
    bridge.set_default_provider_key("claude-local").await;
    bridge.set_thread_store(store).await;

    bridge
        .start_agent_run(
            run_request(
                "sess::provider-mismatch",
                "start fresh claude",
                "run-provider-mismatch",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let snapshots = provider.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert!(
        !snapshots[0].contains_key("sdk_session_id"),
        "claude should not inherit a legacy sdk_session_id from another provider"
    );
}

#[tokio::test]
async fn test_clear_thread_state_keeps_affinity_when_provider_clear_fails() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(ClearSessionProvider::new(ProviderType::ClaudeCode, false));
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_thread_affinity("sess::clear-fail", "p1").await;
    bridge
        .set_thread_workspace_binding("sess::clear-fail", Some("/tmp/workspace".to_owned()))
        .await;

    assert_eq!(
        bridge.clear_thread_state("sess::clear-fail", None).await,
        ClearSessionOutcome::RetryableFailure
    );
    assert_eq!(
        provider.cleared_sessions(),
        vec!["sess::clear-fail".to_owned()]
    );
    assert_eq!(
        bridge
            .resolve_provider_for_thread("sess::clear-fail", "telegram", "main")
            .await,
        Some("p1".to_owned())
    );
    assert_eq!(
        bridge
            .thread_workspace_bindings_snapshot()
            .await
            .get("sess::clear-fail")
            .map(String::as_str),
        Some("/tmp/workspace")
    );
}

#[tokio::test]
async fn test_clear_thread_state_reports_success_before_local_state_is_dropped() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(ClearSessionProvider::new(ProviderType::ClaudeCode, true));
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_thread_affinity("sess::clear-ok", "p1").await;
    bridge
        .set_thread_workspace_binding("sess::clear-ok", Some("/tmp/workspace-ok".to_owned()))
        .await;

    assert_eq!(
        bridge.clear_thread_state("sess::clear-ok", None).await,
        ClearSessionOutcome::Cleared
    );
    assert_eq!(
        provider.cleared_sessions(),
        vec!["sess::clear-ok".to_owned()]
    );
    assert_eq!(
        bridge
            .resolve_provider_for_thread("sess::clear-ok", "telegram", "main")
            .await,
        Some("p1".to_owned())
    );
    assert!(
        bridge
            .thread_workspace_bindings_snapshot()
            .await
            .contains_key("sess::clear-ok")
    );
    bridge.drop_thread_state("sess::clear-ok").await;
    assert_eq!(
        bridge
            .resolve_provider_for_thread("sess::clear-ok", "telegram", "main")
            .await,
        None
    );
}

#[tokio::test]
async fn test_start_run_attaches_bridge_run_id_metadata() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let mut metadata = HashMap::new();
    metadata.insert(
        "client_run_id".to_owned(),
        serde_json::Value::String("external-run".to_owned()),
    );

    bridge
        .start_agent_run(
            AgentRunRequest::new(
                "sess::bridge-run-id",
                "hello",
                "bridge-run-1",
                "api",
                "main",
                metadata,
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let snapshots = provider.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].get("bridge_run_id"),
        Some(&serde_json::Value::String("bridge-run-1".to_owned()))
    );
    assert_eq!(
        snapshots[0].get("client_run_id"),
        Some(&serde_json::Value::String("external-run".to_owned()))
    );
}

#[tokio::test]
async fn test_start_agent_run_persists_runtime_snapshot_on_first_non_override_run() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let thread_id = "thread::runtime-snapshot-first-run";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "provider_type": "claude_code",
                "metadata": {},
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(history);
    let provider = Arc::new(MockProvider::with_runtime_selection(
        ProviderType::ClaudeCode,
        Some("provider-default-v1"),
        Some("high"),
        Some("flex"),
    ));
    bridge.register_provider("p1", provider).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request(
                thread_id,
                "pin runtime",
                "run-runtime-snapshot-first",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-runtime-snapshot-first").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run should finish");

    let data = store.get(thread_id).await.unwrap().expect("thread data");
    assert_eq!(data["metadata"]["model"], "provider-default-v1");
    assert_eq!(data["metadata"]["model_reasoning_effort"], "high");
    assert_eq!(data["metadata"]["model_service_tier"], "flex");
}

#[tokio::test]
async fn test_start_agent_run_skips_snapshot_fields_controlled_by_override() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let thread_id = "thread::runtime-snapshot-override";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "provider_type": "claude_code",
                "metadata": {
                    "model_override": "override-model",
                },
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(history);
    let provider = Arc::new(MockProvider::with_runtime_selection(
        ProviderType::ClaudeCode,
        Some("override-model"),
        Some("high"),
        Some("flex"),
    ));
    bridge.register_provider("p1", provider).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request(
                thread_id,
                "use override once",
                "run-runtime-snapshot-override",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-runtime-snapshot-override").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run should finish");

    let data = store.get(thread_id).await.unwrap().expect("thread data");
    assert_eq!(data["metadata"]["model_override"], "override-model");
    assert!(data["metadata"]["model"].is_null());
    assert_eq!(data["metadata"]["model_reasoning_effort"], "high");
    assert_eq!(data["metadata"]["model_service_tier"], "flex");
}

#[tokio::test]
async fn test_start_agent_run_does_not_overwrite_existing_runtime_snapshot() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let thread_id = "thread::runtime-snapshot-existing";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "provider_type": "claude_code",
                "metadata": {
                    "model": "provider-default-v1",
                    "model_reasoning_effort": "high",
                },
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(history);
    let provider = Arc::new(MockProvider::with_runtime_selection(
        ProviderType::ClaudeCode,
        Some("provider-default-v2"),
        Some("max"),
        Some("flex"),
    ));
    bridge.register_provider("p1", provider).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request(
                thread_id,
                "keep runtime",
                "run-runtime-snapshot-existing",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-runtime-snapshot-existing").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run should finish");

    let data = store.get(thread_id).await.unwrap().expect("thread data");
    assert_eq!(data["metadata"]["model"], "provider-default-v1");
    assert_eq!(data["metadata"]["model_reasoning_effort"], "high");
    assert_eq!(data["metadata"]["model_service_tier"], "flex");
}

#[tokio::test]
async fn test_start_agent_run_uses_thread_snapshot_before_agent_profile_metadata() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let thread_id = "thread::runtime-snapshot-run-metadata";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "test-agent",
                "provider_type": "claude_code",
                "metadata": {
                    "model": "provider-default-v1",
                    "model_reasoning_effort": "high",
                },
            }),
        )
        .await
        .unwrap();
    bridge.set_thread_store(store).await;
    bridge.set_thread_history(history);
    bridge
        .replace_agent_profiles(agent_snapshot(vec![custom_agent(
            "test-agent",
            "Test Agent",
            ProviderType::ClaudeCode,
            "agent-model-v2",
            "Synthetic test agent.",
        )]))
        .await;
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request(
                thread_id,
                "use pinned runtime",
                "run-runtime-snapshot-metadata",
                "api",
                "main",
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run-runtime-snapshot-metadata").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run should finish");

    let snapshots = provider.metadata_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].get("model"),
        Some(&serde_json::Value::String("provider-default-v1".to_owned()))
    );
    assert_eq!(
        snapshots[0].get("model_reasoning_effort"),
        Some(&serde_json::Value::String("high".to_owned()))
    );
    assert_eq!(
        snapshots[0].get("agent_display_name"),
        Some(&serde_json::Value::String("Test Agent".to_owned()))
    );
}

#[tokio::test]
async fn test_set_thread_workspace_binding_canonicalizes_path() {
    let bridge = MultiProviderBridge::default();
    bridge
        .set_thread_workspace_binding("thread-1", Some("/tmp/../tmp".to_owned()))
        .await;
    let bindings = bridge.thread_workspace_bindings_snapshot().await;
    let expected = std::fs::canonicalize("/tmp")
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(bindings.get("thread-1").unwrap(), &expected);
}

#[tokio::test]
async fn test_set_thread_workspace_binding_fallback_for_nonexistent() {
    let bridge = MultiProviderBridge::default();
    let bogus = "/nonexistent_path_abc123_xyz".to_owned();
    bridge
        .set_thread_workspace_binding("thread-2", Some(bogus.clone()))
        .await;
    let bindings = bridge.thread_workspace_bindings_snapshot().await;
    assert_eq!(bindings.get("thread-2").unwrap(), &bogus);
}

/// Bug A: editing `agents.claude.default_model` on disk and running
/// `POST /api/settings/reload` reaches `reload_from_config`, but
/// `get_or_create_provider` short-circuits on the stable provider key
/// (`compute_provider_key` intentionally excludes `default_model` to keep
/// thread affinity / SDK session ids stable) and silently drops the new
/// config. The existing provider instance keeps serving the old
/// `default_model` until the gateway restarts.
///
/// Target behavior: the provider key stays stable AND the already-registered
/// provider instance hot-applies the new model defaults, so new runs (in new
/// or existing threads) immediately use the reloaded provider default.
#[tokio::test]
async fn test_reload_from_config_hot_applies_new_provider_default_model() {
    let bridge = MultiProviderBridge::new();
    let mut config = GaryxConfig::default();
    config.agents.insert(
        "claude".to_owned(),
        json!({
            "provider_type": "claude_code",
            "default_model": "claude-opus-4-8",
            "model_reasoning_effort": "medium"
        }),
    );
    bridge.reload_from_config(&config).await.unwrap();
    let key_before = bridge
        .default_provider_key()
        .await
        .expect("default provider key after first reload");

    // Simulate the user editing the provider default in garyx.json and
    // triggering a settings reload.
    config.agents.insert(
        "claude".to_owned(),
        json!({
            "provider_type": "claude_code",
            "default_model": "claude-fable-5",
            "model_reasoning_effort": "high"
        }),
    );
    bridge.reload_from_config(&config).await.unwrap();
    let key_after = bridge
        .default_provider_key()
        .await
        .expect("default provider key after second reload");
    assert_eq!(
        key_before, key_after,
        "provider key must stay stable across default-model edits (thread affinity)"
    );

    let provider = bridge
        .get_provider(&key_after)
        .await
        .expect("default provider instance");
    let options = ProviderRunOptions {
        thread_id: "thread::reload-default-model".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };
    let selection = provider.resolve_runtime_selection(&options);
    assert_eq!(
        selection.model.as_deref(),
        Some("claude-fable-5"),
        "runs without a model request must use the reloaded provider default model"
    );
    assert_eq!(
        selection.model_reasoning_effort.as_deref(),
        Some("high"),
        "runs without an effort request must use the reloaded provider default effort"
    );
}

/// Single-cell contract (bridge side, guard): thread `metadata.model` (plus
/// effort/tier) is THE model cell for the thread — "what this thread actually
/// runs". `backfill_bound_agent_runtime_metadata` must keep injecting the
/// cell into run metadata so a thread with a pinned cell stays on that model
/// even after the provider default changes (thread pinning is intentional).
#[tokio::test]
async fn test_backfill_runtime_metadata_injects_thread_model_cell() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    let thread_id = "thread::model-cell";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "provider_type": "claude_code",
                "metadata": {
                    "model": "claude-opus-4-8",
                    "model_reasoning_effort": "low",
                    "model_service_tier": "flex",
                },
            }),
        )
        .await
        .unwrap();

    let mut metadata: HashMap<String, Value> = HashMap::new();
    bridge
        .backfill_bound_agent_runtime_metadata(thread_id, &mut metadata)
        .await;

    assert_eq!(
        metadata.get("model"),
        Some(&Value::String("claude-opus-4-8".to_owned())),
        "the thread model cell must drive the run model"
    );
    assert_eq!(
        metadata.get("model_reasoning_effort"),
        Some(&Value::String("low".to_owned())),
        "the thread effort cell must drive the run effort"
    );
    assert_eq!(
        metadata.get("model_service_tier"),
        Some(&Value::String("flex".to_owned())),
        "the thread service-tier cell must drive the run service tier"
    );
}

/// Single-cell contract (bridge side, guard): with the target priority
/// `thread cell (metadata.model) > agent.model > provider default > catalog`,
/// the thread's model cell must win over the bound agent's model.
#[tokio::test]
async fn test_backfill_runtime_metadata_prefers_model_cell_over_agent_model() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    bridge
        .replace_agent_profiles(agent_snapshot(vec![custom_agent(
            "test-agent",
            "Test Agent",
            ProviderType::ClaudeCode,
            "agent-model-v2",
            "Synthetic test agent.",
        )]))
        .await;
    let thread_id = "thread::cell-vs-agent-model";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "test-agent",
                "provider_type": "claude_code",
                "metadata": {
                    "model": "thread-cell-model",
                },
            }),
        )
        .await
        .unwrap();

    let mut metadata: HashMap<String, Value> = HashMap::new();
    bridge
        .backfill_bound_agent_runtime_metadata(thread_id, &mut metadata)
        .await;

    assert_eq!(
        metadata.get("model"),
        Some(&Value::String("thread-cell-model".to_owned())),
        "the thread model cell must win over the bound agent model"
    );
}

/// Legacy compatibility (bridge side, guard): stored threads may still carry
/// the old dual-track `metadata.model_override`. Until the write paths migrate
/// it into the cell, reads must coalesce(override, cell) — the legacy override
/// keeps the highest priority.
#[tokio::test]
async fn test_backfill_runtime_metadata_legacy_override_wins_over_cell() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    let thread_id = "thread::legacy-override-vs-cell";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "provider_type": "claude_code",
                "metadata": {
                    "model": "cell-model",
                    "model_override": "legacy-override-model",
                },
            }),
        )
        .await
        .unwrap();

    let mut metadata: HashMap<String, Value> = HashMap::new();
    bridge
        .backfill_bound_agent_runtime_metadata(thread_id, &mut metadata)
        .await;

    assert_eq!(
        metadata.get("model"),
        Some(&Value::String("legacy-override-model".to_owned())),
        "legacy model_override data must keep winning over the cell until migrated"
    );
}

/// Provider-env contract (bridge side, guard): the thread's agent runtime
/// snapshot `provider_env` is the only client-independent env source for a
/// run. `backfill_bound_agent_runtime_metadata` must inject it into run
/// metadata for every dispatch path (`/api/chat/start`, `/api/chat/ws`,
/// internal dispatch all funnel through this backfill), so proxy routing
/// like `ANTHROPIC_BASE_URL` works without any client cooperation.
#[tokio::test]
async fn test_backfill_runtime_metadata_injects_thread_provider_env_snapshot() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    let thread_id = "thread::provider-env-snapshot";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "provider_type": "claude_code",
                "metadata": {
                    "provider_env": {
                        "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721",
                        "ANTHROPIC_MODEL": "claude-opus-4-8",
                    },
                },
            }),
        )
        .await
        .unwrap();

    let mut metadata: HashMap<String, Value> = HashMap::new();
    bridge
        .backfill_bound_agent_runtime_metadata(thread_id, &mut metadata)
        .await;

    let provider_env = metadata
        .get("provider_env")
        .and_then(Value::as_object)
        .expect("thread snapshot provider_env must reach run metadata");
    assert_eq!(
        provider_env
            .get("ANTHROPIC_BASE_URL")
            .and_then(Value::as_str),
        Some("http://127.0.0.1:15721"),
        "proxy base URL from the thread snapshot must drive the run"
    );
    assert_eq!(
        provider_env.get("ANTHROPIC_MODEL").and_then(Value::as_str),
        Some("claude-opus-4-8"),
    );
}

/// Provider-env contract (bridge side, guard): backfill is existing-wins —
/// a run that already carries an explicit `provider_env` (e.g. internal
/// dispatch with a per-run override) must not have it replaced by the
/// thread snapshot.
#[tokio::test]
async fn test_backfill_runtime_metadata_keeps_explicit_provider_env() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    let thread_id = "thread::provider-env-explicit";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "provider_type": "claude_code",
                "metadata": {
                    "provider_env": {
                        "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721",
                    },
                },
            }),
        )
        .await
        .unwrap();

    let mut metadata: HashMap<String, Value> = HashMap::new();
    metadata.insert(
        "provider_env".to_owned(),
        json!({ "ANTHROPIC_BASE_URL": "http://127.0.0.1:19999" }),
    );
    bridge
        .backfill_bound_agent_runtime_metadata(thread_id, &mut metadata)
        .await;

    assert_eq!(
        metadata
            .get("provider_env")
            .and_then(Value::as_object)
            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
            .and_then(Value::as_str),
        Some("http://127.0.0.1:19999"),
        "explicit run provider_env must win over the thread snapshot"
    );
}

async fn wait_for_lease_count(store: &Arc<dyn ThreadStore>, thread_id: &str, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if store.run_coordinator().lease_count(thread_id) == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("lease count should converge");
}

async fn history_with_blocked_transcript(
    store: Arc<dyn ThreadStore>,
    thread_id: &str,
) -> (
    Arc<ThreadHistoryRepository>,
    tempfile::TempDir,
    std::path::PathBuf,
) {
    let temp = tempfile::tempdir().expect("transcript tempdir");
    let transcript_store = Arc::new(
        ThreadTranscriptStore::file(temp.path())
            .await
            .expect("file transcript store"),
    );
    let transcript_path = transcript_store
        .transcript_path(thread_id)
        .expect("file transcript path");
    std::fs::create_dir(&transcript_path).expect("block transcript file with directory");
    (
        Arc::new(ThreadHistoryRepository::new(store, transcript_store)),
        temp,
        transcript_path,
    )
}

async fn wait_for_failed_initial_persistence(store: &Arc<dyn ThreadStore>, thread_id: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let attempted = store
                .get(thread_id)
                .await
                .ok()
                .flatten()
                .and_then(|record| {
                    record
                        .pointer("/history/message_count")
                        .and_then(Value::as_u64)
                })
                == Some(0);
            if attempted {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("initial persistence attempt should finish with zero committed rows");
}

struct BlockRunAcceptedLogSink {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait::async_trait]
impl ThreadLogSink for BlockRunAcceptedLogSink {
    async fn record_event(&self, event: ThreadLogEvent) {
        if event.message == "run accepted" {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }

    async fn read_chunk(
        &self,
        thread_id: &str,
        cursor: Option<u64>,
    ) -> Result<ThreadLogChunk, String> {
        Ok(ThreadLogChunk {
            thread_id: thread_id.to_owned(),
            path: String::new(),
            text: String::new(),
            cursor: cursor.unwrap_or(0),
            reset: cursor.is_none(),
        })
    }

    async fn delete_thread(&self, _thread_id: &str) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn cancelling_dispatch_after_lease_promotion_releases_without_runtime_state() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let thread_id = "thread::lease-cancelled-dispatch";
    let run_id = "run::lease-cancelled-dispatch";
    store.set(thread_id, json!({})).await.unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));
    let entered = Arc::new(Notify::new());
    bridge.set_thread_log_sink(Arc::new(BlockRunAcceptedLogSink {
        entered: entered.clone(),
        release: Arc::new(Notify::new()),
    }));

    let admitted = AdmittedRun::thread_bound(
        store.clone(),
        run_request(thread_id, "cancel", run_id, "api", "main"),
    )
    .await
    .unwrap();
    let dispatch_bridge = bridge.clone();
    let dispatch = tokio::spawn(async move { dispatch_bridge.dispatch(admitted, None).await });
    tokio::time::timeout(std::time::Duration::from_secs(3), entered.notified())
        .await
        .expect("dispatch reached the post-promotion barrier");
    assert!(store.run_coordinator().has_active_lease(thread_id));

    dispatch.abort();
    let _ = dispatch.await;
    wait_for_lease_count(&store, thread_id, 0).await;
    assert!(!bridge.is_run_active(run_id).await);
    assert!(bridge.thread_affinity_for(thread_id).await.is_none());
    assert!(provider.metadata_snapshots().is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), store.delete(thread_id))
            .await
            .expect("delete cannot hang on a cancelled dispatch")
            .expect("delete result")
    );
}

#[tokio::test]
async fn delete_between_admission_and_dispatch_rejects_before_provider_or_affinity() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let thread_id = "thread::delete-before-dispatch";
    store.set(thread_id, json!({})).await.unwrap();
    bridge.set_thread_store(store.clone()).await;

    let admitted = AdmittedRun::thread_bound(
        store.clone(),
        run_request(
            thread_id,
            "must not dispatch",
            "run::delete-before-dispatch",
            "api",
            "main",
        ),
    )
    .await
    .unwrap();
    let deleting_store = store.clone();
    let delete = tokio::spawn(async move { deleting_store.delete(thread_id).await });
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !store.run_coordinator().mutation_in_progress(thread_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("delete must invalidate the admitted token");

    let error = bridge
        .dispatch(admitted, None)
        .await
        .expect_err("invalidated admission must fail at bridge entry");
    assert!(error.contains("being archived or deleted"));
    assert!(delete.await.unwrap().unwrap());
    assert!(provider.metadata_snapshots().is_empty());
    assert!(bridge.thread_affinity_for(thread_id).await.is_none());
    assert!(store.get(thread_id).await.unwrap().is_none());
}

#[tokio::test]
async fn admitted_started_run_holds_active_lease_until_provider_terminal() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(CheckpointingProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let thread_id = "thread::lease-started";
    store.set(thread_id, json!({})).await.unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

    let admitted = AdmittedRun::thread_bound(
        store.clone(),
        run_request(thread_id, "hold", "run::lease-started", "api", "main"),
    )
    .await
    .unwrap();
    bridge.dispatch(admitted, None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .unwrap();
    assert!(store.run_coordinator().has_active_lease(thread_id));
    let archive = store
        .run_coordinator()
        .reserve_archive(store.as_ref(), thread_id)
        .await
        .unwrap();
    assert!(matches!(archive, ArchiveBarrier::ActiveLease(_)));

    provider.release_run();
    wait_for_lease_count(&store, thread_id, 0).await;
}

#[tokio::test]
async fn failed_initial_persistence_keeps_lease_until_provider_terminal_and_blocks_archive() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(CheckpointingProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let thread_id = "thread::lease-persistence-failure";
    store.set(thread_id, json!({})).await.unwrap();
    let (history, _temp, transcript_path) =
        history_with_blocked_transcript(store.clone(), thread_id).await;
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(history);

    let admitted = AdmittedRun::thread_bound(
        store.clone(),
        run_request(
            thread_id,
            "hold after persistence failure",
            "run::lease-persistence-failure",
            "api",
            "main",
        ),
    )
    .await
    .unwrap();
    bridge.dispatch(admitted, None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .unwrap();
    wait_for_failed_initial_persistence(&store, thread_id).await;
    assert!(
        transcript_path.is_dir(),
        "run_start append was forced to fail"
    );
    assert!(store.run_coordinator().has_active_lease(thread_id));

    let archive = store
        .run_coordinator()
        .reserve_archive(store.as_ref(), thread_id)
        .await
        .unwrap();
    assert!(matches!(archive, ArchiveBarrier::ActiveLease(_)));

    provider.release_run();
    wait_for_lease_count(&store, thread_id, 0).await;
}

#[tokio::test]
async fn delete_aborts_and_drains_a_run_even_after_initial_persistence_failed() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(CheckpointingProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider).await;
    bridge.set_default_provider_key("p1").await;
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let thread_id = "thread::delete-persistence-failure";
    let run_id = "run::delete-persistence-failure";
    store.set(thread_id, json!({})).await.unwrap();
    let (history, _temp, transcript_path) =
        history_with_blocked_transcript(store.clone(), thread_id).await;
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(history);

    let admitted = AdmittedRun::thread_bound(
        store.clone(),
        run_request(
            thread_id,
            "delete after persistence failure",
            run_id,
            "api",
            "main",
        ),
    )
    .await
    .unwrap();
    bridge.dispatch(admitted, None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .unwrap();
    wait_for_failed_initial_persistence(&store, thread_id).await;
    assert!(
        transcript_path.is_dir(),
        "run_start append was forced to fail"
    );
    assert!(store.run_coordinator().has_active_lease(thread_id));

    let deleted = tokio::time::timeout(std::time::Duration::from_secs(3), store.delete(thread_id))
        .await
        .expect("delete must drain without deadlocking")
        .expect("coordinated delete");
    assert!(deleted);
    assert_eq!(store.run_coordinator().lease_count(thread_id), 0);
    assert!(store.get(thread_id).await.unwrap().is_none());
    assert!(!bridge.is_run_active(run_id).await);
}

#[tokio::test]
async fn queued_dispatch_releases_its_request_lease_after_active_run_accepts_input() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(QueuedInputProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let thread_id = "thread::lease-queued";
    store.set(thread_id, json!({})).await.unwrap();
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

    let first = AdmittedRun::thread_bound(
        store.clone(),
        run_request(thread_id, "first", "run::lease-first", "api", "main"),
    )
    .await
    .unwrap();
    bridge.dispatch(first, None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), delta_sent.notified())
        .await
        .unwrap();
    wait_for_lease_count(&store, thread_id, 1).await;

    let second = AdmittedRun::thread_bound(
        store.clone(),
        run_request(thread_id, "second", "run::lease-second", "api", "main"),
    )
    .await
    .unwrap();
    assert_eq!(
        store.run_coordinator().lease_count(thread_id),
        2,
        "same-thread requests own independent leases before queue handoff"
    );
    let outcome = bridge.dispatch(second, None).await.unwrap();
    assert!(matches!(
        outcome,
        garyx_models::provider::AgentDispatchOutcome::QueuedToActiveRun { .. }
    ));
    assert_eq!(store.run_coordinator().lease_count(thread_id), 1);

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while provider.received_inputs().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    provider.release_ack();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    provider.release_run();
    wait_for_lease_count(&store, thread_id, 0).await;
}

#[tokio::test]
async fn provider_error_and_panic_both_release_active_lease() {
    let cases: Vec<(&str, Arc<dyn ProviderRuntime>)> = vec![
        (
            "error",
            Arc::new(FailingCheckpointProvider::new()) as Arc<dyn ProviderRuntime>,
        ),
        (
            "panic",
            Arc::new(PanickingProvider) as Arc<dyn ProviderRuntime>,
        ),
    ];
    for (case, provider) in cases {
        let bridge = MultiProviderBridge::new();
        bridge.register_provider("p1", provider).await;
        bridge.set_default_provider_key("p1").await;
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let thread_id = format!("thread::lease-{case}");
        store.set(&thread_id, json!({})).await.unwrap();
        bridge.set_thread_store(store.clone()).await;
        bridge.set_thread_history(make_history(store.clone()));
        let run_id = format!("run::lease-{case}");
        let admitted = AdmittedRun::thread_bound(
            store.clone(),
            run_request(&thread_id, "fail", &run_id, "api", "main"),
        )
        .await
        .unwrap();
        bridge.dispatch(admitted, None).await.unwrap();
        wait_for_lease_count(&store, &thread_id, 0).await;
        let _ = bridge.abort_run(&run_id).await;
    }
}
