use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use chrono::Utc;
use garyx_models::config::{GaryxConfig, TelegramAccount, telegram_account_to_plugin_entry};
use garyx_models::provider::{
    AgentRunRequest, ImagePayload, ProviderMessage, ProviderRunOptions, ProviderRunResult,
    ProviderType, QueuedUserInput, StreamBoundaryKind, StreamEvent,
};
use garyx_models::{
    AgentTeamProfile, CustomAgentProfile, Principal, TaskStatus, ThreadTask,
    builtin_provider_agent_profiles,
};
use garyx_router::{
    InMemoryThreadStore, ThreadHistoryRepository, ThreadStore, ThreadTranscriptStore,
};
use serde_json::json;
use tokio::sync::{Mutex, Notify, mpsc};

use super::MultiProviderBridge;
use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

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

fn active_or_committed_messages(data: &serde_json::Value) -> Vec<serde_json::Value> {
    data["history"]["active_run_snapshot"]["messages"]
        .as_array()
        .cloned()
        .or_else(|| data["messages"].as_array().cloned())
        .unwrap_or_default()
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
        default_workspace_dir: None,
        system_prompt: system_prompt.to_owned(),
        built_in: false,
        standalone: true,
        created_at: "2026-04-19T00:00:00Z".to_owned(),
        updated_at: "2026-04-19T00:00:00Z".to_owned(),
    }
}

/// A mock provider for testing.
struct MockProvider {
    ready: AtomicBool,
    ptype: ProviderType,
    image_counts: std::sync::Mutex<Vec<usize>>,
    metadata_snapshots: std::sync::Mutex<Vec<HashMap<String, serde_json::Value>>>,
    workspace_dirs: std::sync::Mutex<Vec<Option<String>>>,
    run_delay_ms: u64,
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

struct EventfulProvider;

struct CheckpointingProvider {
    delta_sent: Arc<Notify>,
    release_run: Arc<Notify>,
}

struct FailingCheckpointProvider {
    delta_sent: Arc<Notify>,
}

struct EmptyResponseProvider;

struct FailedResultProvider;

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
impl AgentLoopProvider for MockProvider {
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
}

#[async_trait::async_trait]
impl AgentLoopProvider for TitleProvider {
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
impl AgentLoopProvider for ClearSessionProvider {
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

    async fn clear_session(&self, session_key: &str) -> bool {
        self.cleared_sessions
            .lock()
            .unwrap()
            .push(session_key.to_owned());
        self.should_clear.load(Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl AgentLoopProvider for EventfulProvider {
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
            json!({
                "type": "commandExecution",
                "command": "pwd",
            }),
            Some("tool-1".to_owned()),
            Some("shell".to_owned()),
        );
        let tool_result = ProviderMessage::tool_result(
            json!({
                "output": "/tmp",
            }),
            Some("tool-1".to_owned()),
            Some("shell".to_owned()),
            Some(false),
        );

        on_chunk(StreamEvent::Delta {
            text: "alpha".to_owned(),
        });
        on_chunk(StreamEvent::ToolUse { message: tool_use });
        on_chunk(StreamEvent::ToolResult {
            message: tool_result,
        });
        on_chunk(StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        });
        on_chunk(StreamEvent::Delta {
            text: "beta".to_owned(),
        });
        on_chunk(StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: None,
        });
        on_chunk(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id: "eventful-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "alphabeta".to_owned(),
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
}

#[async_trait::async_trait]
impl AgentLoopProvider for CheckpointingProvider {
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
impl AgentLoopProvider for QueuedInputProvider {
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
impl AgentLoopProvider for FailingCheckpointProvider {
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
impl AgentLoopProvider for EmptyResponseProvider {
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
impl AgentLoopProvider for FailedResultProvider {
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
impl AgentLoopProvider for DelayedQueuedInputProvider {
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
impl AgentLoopProvider for InterruptingFollowUpProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GeminiCli
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
async fn test_provider_type_for_team_returns_agent_team_meta_provider() {
    // A team is its own addressable agent in the unified namespace, served by
    // the AgentTeam meta-provider. Leader is NOT privileged — resolving the
    // leader's agent_id still returns the leader's own provider type, not the
    // team's.
    let bridge = MultiProviderBridge::new();
    bridge
        .replace_agent_profiles(builtin_provider_agent_profiles())
        .await;
    bridge
        .replace_team_profiles(vec![AgentTeamProfile {
            team_id: "product-ship".to_owned(),
            display_name: "Product Ship".to_owned(),
            leader_agent_id: "codex".to_owned(),
            member_agent_ids: vec!["codex".to_owned(), "claude".to_owned()],
            workflow_text: "Codex leads and Claude reviews.".to_owned(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
        }])
        .await;

    assert_eq!(
        bridge.provider_type_for_agent("product-ship").await,
        Some(ProviderType::AgentTeam)
    );
    assert_eq!(
        bridge.provider_type_for_agent("codex").await,
        Some(ProviderType::CodexAppServer)
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
                agent_id: "claude".to_owned(),
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
async fn test_reload_from_config_preserves_agent_team_provider() {
    // Regression: `reload_from_config` rebuilds `desired_provider_keys` from
    // config-driven channel routes + default providers + remote providers.
    // The AgentTeam meta-provider is registered imperatively at boot and has
    // no config representation, so reload must explicitly keep it or
    // team-bound threads lose their provider on the next config edit.
    let bridge = MultiProviderBridge::new();
    let agent_team = Arc::new(MockProvider::new(ProviderType::AgentTeam));
    bridge
        .register_provider("agent_team::default", agent_team)
        .await;

    let config = GaryxConfig::default();
    bridge.reload_from_config(&config).await.unwrap();

    assert!(
        bridge.get_provider("agent_team::default").await.is_some(),
        "AgentTeam provider must survive reload_from_config"
    );

    // Second reload must also keep it (the retain rule must hold across
    // repeated reconciliations, not just the first).
    bridge.reload_from_config(&config).await.unwrap();
    assert!(
        bridge.get_provider("agent_team::default").await.is_some(),
        "AgentTeam provider must survive successive reload_from_config calls"
    );
}

#[tokio::test]
async fn run_subagent_streaming_rederives_leaf_metadata_and_persists_history() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

    let provider = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge
        .register_provider("claude-child", provider.clone())
        .await;
    bridge
        .replace_agent_profiles(vec![custom_agent(
            "coder",
            "Coder",
            ProviderType::ClaudeCode,
            "claude-opus-child",
            "You are the coder child.",
        )])
        .await;

    store
        .set(
            "thread::child-coder",
            json!({
                "thread_id": "thread::child-coder",
                "agent_id": "coder",
                "provider_type": "claude_code",
            }),
        )
        .await;

    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("product-ship")),
        ("agent_display_name".to_owned(), json!("Product Ship")),
        ("system_prompt".to_owned(), json!("team prompt")),
        ("model".to_owned(), json!("team-model")),
        ("agent_team_id".to_owned(), json!("product-ship")),
        ("group_transcript_snapshot".to_owned(), json!([])),
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
        .run_subagent_streaming(
            "thread::child-coder",
            "hello child",
            metadata,
            None,
            None,
            Some(callback),
        )
        .await
        .expect("sub-agent run should succeed");

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
    assert!(!snapshot.contains_key("agent_team_id"));
    assert!(!snapshot.contains_key("group_transcript_snapshot"));

    let thread_data = store
        .get("thread::child-coder")
        .await
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
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[0]["role"], "user");
    assert_eq!(combined[0]["content"], "hello child");
    assert_eq!(combined[1]["role"], "assistant");
    assert_eq!(combined[1]["content"], "echo: hello child");
}

#[tokio::test]
async fn start_agent_run_rejects_thread_bound_to_missing_team() {
    let bridge = MultiProviderBridge::new();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));
    store
        .set(
            "thread::deleted-team",
            json!({
                "thread_id": "thread::deleted-team",
                "agent_id": "team::deleted",
                "provider_type": "agent_team",
            }),
        )
        .await;

    let error = bridge
        .start_agent_run(
            run_request(
                "thread::deleted-team",
                "hello",
                "run-missing-team",
                "api",
                "main",
            ),
            None,
        )
        .await
        .expect_err("missing team should fail before provider dispatch");

    match error {
        BridgeError::SessionError(message) => {
            assert!(
                message.contains("missing agent team team::deleted"),
                "unexpected error: {message}"
            );
        }
        other => panic!("expected SessionError, got {other:?}"),
    }
}

#[tokio::test]
async fn test_start_and_complete_run() {
    let bridge = MultiProviderBridge::new();
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

    // Give the spawned task time to complete.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

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
        .await;

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
        .await;

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
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    bridge
        .start_agent_run(
            run_request("sess", "hello", "run-1", "telegram", "main"),
            None,
        )
        .await
        .unwrap();

    let aborted = bridge.abort_run("run-1").await;
    // Either task was cancelled or cleanup happened.
    assert!(aborted || !bridge.is_run_active("run-1").await);
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
    bridge.set_thread_history(make_history(store.clone()));

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

    let data = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Some(data) = store.get("sess::tg::persist").await {
                let message_count = data["messages"]
                    .as_array()
                    .map(|messages| messages.len())
                    .unwrap_or(0);
                if message_count >= 2 {
                    break data;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("session data should be persisted");
    assert_eq!(data["provider_key"], "p1");
    let messages = data["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "hello bot");
    assert_eq!(messages[1]["role"], "assistant");
}

#[tokio::test]
async fn test_thread_persistence_checkpoints_streaming_output_before_run_completion() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(CheckpointingProvider::new());
    let delta_sent = provider.delta_sent();
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_default_provider_key("p1").await;

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

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
            let Some(data) = store.get("sess::tg::checkpoint").await else {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            };
            let messages = active_or_committed_messages(&data);
            if messages.iter().any(|message| {
                message["role"] == "assistant" && message["content"] == "partial reply"
            }) {
                break data;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("thread store should receive a partial checkpoint before completion");

    let checkpoint_messages = active_or_committed_messages(&checkpointed);
    assert_eq!(checkpoint_messages.len(), 2);
    assert_eq!(checkpoint_messages[0]["role"], "user");
    assert_eq!(checkpoint_messages[0]["content"], "keep this");
    assert_eq!(checkpoint_messages[1]["role"], "assistant");
    assert_eq!(checkpoint_messages[1]["content"], "partial reply");

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
        .expect("final thread data should exist");
    let final_messages = final_data["messages"]
        .as_array()
        .expect("messages should be an array");
    assert_eq!(final_messages.len(), 2);
    assert_eq!(final_messages[1]["content"], "partial reply");
    assert_eq!(final_data["sdk_session_id"], "sdk-sess::tg::checkpoint");
    assert_eq!(
        final_data["provider_sdk_session_ids"]["p1"],
        "sdk-sess::tg::checkpoint"
    );
}

#[tokio::test]
async fn test_failed_run_clears_active_snapshot_and_preserves_partial_messages() {
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
        .await;
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

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
        .expect("failed thread data should exist");
    assert!(
        final_data["history"]["active_run_snapshot"].is_null(),
        "active snapshot should be cleared after failure"
    );
    assert_eq!(final_data["sdk_session_id"], "sdk-existing");
    assert_eq!(final_data["provider_sdk_session_ids"]["p1"], "sdk-existing");
    assert_eq!(final_data["task"]["status"], "in_progress");

    let final_messages = final_data["messages"]
        .as_array()
        .expect("messages should be an array");
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
        .await;
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
        .await;
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

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
        .expect("thread data should exist");
    assert_eq!(final_data["task"]["status"], "in_progress");
    assert!(
        final_data["history"]["active_run_snapshot"].is_null(),
        "failed result should still clear the active snapshot"
    );
    let final_messages = final_data["messages"]
        .as_array()
        .expect("messages should be persisted");
    assert!(
        final_messages
            .iter()
            .any(|message| message["role"] == "assistant"
                && message["content"] == "I'll continue by editing files."),
        "partial response should be preserved for diagnosis"
    );
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
    bridge.set_thread_history(make_history(store.clone()));

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
        .add_streaming_input("sess::tg::queued-input", "follow-up", None, None, None)
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
            let Some(data) = store.get("sess::tg::queued-input").await else {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            };
            let pending_inputs = data["pending_user_inputs"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let messages = active_or_committed_messages(&data);
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

    provider.release_ack();

    let acked_checkpoint = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let Some(data) = store.get("sess::tg::queued-input").await else {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            };
            let pending_inputs = data["pending_user_inputs"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let messages = active_or_committed_messages(&data);
            let has_follow_up_user = messages
                .iter()
                .any(|message| message["role"] == "user" && message["content"] == "follow-up");
            let has_follow_up_assistant = messages.iter().any(|message| {
                message["role"] == "assistant" && message["content"] == "follow-up reply: follow-up"
            });
            if pending_inputs.is_empty() && has_follow_up_user && has_follow_up_assistant {
                break data;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("user_ack should promote the queued follow-up into the persisted transcript");

    let acked_messages = active_or_committed_messages(&acked_checkpoint);
    let roles: Vec<&str> = acked_messages
        .iter()
        .filter_map(|message| message["role"].as_str())
        .collect();
    assert_eq!(roles, vec!["user", "assistant", "user", "assistant"]);

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
        .await;
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(make_history(store.clone()));

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
        .add_streaming_input("sess::tg::queued-task", "继续", None, None, None)
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

    let acked_checkpoint = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let Some(data) = store.get("sess::tg::queued-task").await else {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            };
            let messages = active_or_committed_messages(&data);
            let has_raw_user = messages
                .iter()
                .any(|message| message["role"] == "user" && message["content"] == "继续");
            let has_provider_reply = messages.iter().any(|message| {
                message["role"] == "assistant" && message["content"] == "follow-up reply: 继续"
            });
            if has_raw_user && has_provider_reply {
                break data;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("provider-facing queued input should preserve raw task follow-up after ack");

    let messages = active_or_committed_messages(&acked_checkpoint);
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
        .await;

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
        .await;

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
        .await;

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

    assert!(!bridge.clear_thread_state("sess::clear-fail", None).await);
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
async fn test_clear_thread_state_removes_affinity_after_provider_clear_succeeds() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(ClearSessionProvider::new(ProviderType::ClaudeCode, true));
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_thread_affinity("sess::clear-ok", "p1").await;
    bridge
        .set_thread_workspace_binding("sess::clear-ok", Some("/tmp/workspace-ok".to_owned()))
        .await;

    assert!(bridge.clear_thread_state("sess::clear-ok", None).await);
    assert_eq!(
        provider.cleared_sessions(),
        vec!["sess::clear-ok".to_owned()]
    );
    assert_eq!(
        bridge
            .resolve_provider_for_thread("sess::clear-ok", "telegram", "main")
            .await,
        None
    );
    assert!(
        !bridge
            .thread_workspace_bindings_snapshot()
            .await
            .contains_key("sess::clear-ok")
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
async fn test_event_broadcast_after_run() {
    // P1-D: run lifecycle and live transcript events should appear on broadcast channel after run.
    let bridge = MultiProviderBridge::new();
    let p = Arc::new(MockProvider::new(ProviderType::ClaudeCode));
    bridge.register_provider("p1", p).await;
    bridge.set_default_provider_key("p1").await;

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(128);
    bridge.set_event_tx(tx).await;

    bridge
        .start_agent_run(
            run_request(
                "sess::tg::events",
                "hello",
                "run-events",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    // Collect events.
    let mut events = Vec::new();
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        if events.len() >= 2 {
            break;
        }
    }

    let parsed: Vec<serde_json::Value> = events
        .iter()
        .map(|value| serde_json::from_str(value).unwrap())
        .collect();
    let event_types: Vec<&str> = parsed
        .iter()
        .filter_map(|value| value.get("type").and_then(serde_json::Value::as_str))
        .collect();

    assert!(event_types.contains(&"user_message"));
    assert!(event_types.contains(&"assistant_delta"));
    assert!(event_types.contains(&"done"));
    assert!(event_types.contains(&"run_start"));
    assert!(event_types.contains(&"run_complete"));
}

#[tokio::test]
async fn test_event_broadcast_includes_tool_and_boundary_events_without_external_callback() {
    let bridge = MultiProviderBridge::new();
    let provider = Arc::new(EventfulProvider);
    bridge.register_provider("p1", provider).await;
    bridge.set_default_provider_key("p1").await;

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(128);
    bridge.set_event_tx(tx).await;

    bridge
        .start_agent_run(
            run_request(
                "thread::events-rich",
                "hello rich stream",
                "run-events-rich",
                "telegram",
                "main",
            ),
            None,
        )
        .await
        .unwrap();

    let mut events = Vec::new();
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        if events
            .iter()
            .any(|event| event.contains("\"type\":\"run_complete\""))
        {
            break;
        }
    }

    let parsed: Vec<serde_json::Value> = events
        .iter()
        .map(|value| serde_json::from_str(value).unwrap())
        .collect();
    let event_types: Vec<&str> = parsed
        .iter()
        .filter_map(|value| value.get("type").and_then(serde_json::Value::as_str))
        .collect();

    assert!(event_types.contains(&"user_message"));
    assert!(event_types.contains(&"assistant_delta"));
    assert!(event_types.contains(&"tool_use"));
    assert!(event_types.contains(&"tool_result"));
    assert!(event_types.contains(&"assistant_boundary"));
    assert!(event_types.contains(&"user_ack"));
    assert!(event_types.contains(&"done"));
    assert!(event_types.contains(&"run_start"));
    assert!(event_types.contains(&"run_complete"));
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
