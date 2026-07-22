use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{BridgeError, ProviderRuntime, StreamCallback};
use garyx_channels::{
    ChannelDispatcher, ChannelInfo, OutboundMessage, SendMessageResult, StreamDispatchCallback,
    StreamingDispatchTarget,
};
use garyx_models::config::{ApiAccount, GaryxConfig};
use garyx_models::provider::{
    AgentRunRequest, ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput,
    StreamEvent,
};
use garyx_router::{
    AdmittedRun, AgentDispatcher, ChannelBinding, EndpointBindingMutator, InMemoryThreadStore,
    ThreadCreationError, ThreadCreator, ThreadEnsureOptions, ThreadStore,
};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;
use tower::ServiceExt;

use crate::application::chat::contracts::{
    ChatRequest, IdempotencyScope, InterruptRequest, StreamInputRequest,
};
use crate::server::{AppState, AppStateBuilder};

use super::{
    ChatWsForwardOutcome, chat_health, chat_interrupt, chat_start, chat_stream_input,
    forward_chat_ws_committed_value, is_terminal_bus_record_for_run,
    spawn_chat_ws_committed_stream, start_chat_run_with_quota_recovery,
};

struct ReadyProvider;
struct SlowProvider {
    delay_ms: u64,
}
struct BlockingReplyProvider {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    text: String,
}
struct QueueAcceptingProvider {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    queued_inputs: Arc<AtomicUsize>,
}
struct WorkspaceRecordingProvider {
    observed_workspace_dir: Arc<Mutex<Option<Option<String>>>>,
}
struct MetadataRecordingProvider {
    calls: Arc<AtomicUsize>,
    observed: Arc<Mutex<Vec<ProviderRunOptions>>>,
}

struct FailingStorageThreadCreator;

#[async_trait]
impl ThreadCreator for FailingStorageThreadCreator {
    async fn create_thread(
        &self,
        _thread_store: Arc<dyn ThreadStore>,
        _options: ThreadEnsureOptions,
    ) -> Result<(String, Value), ThreadCreationError> {
        Err(ThreadCreationError::Storage(
            "injected creation backend failure".to_owned(),
        ))
    }
}

#[derive(Default)]
struct RecordingDispatcher {
    calls: Arc<Mutex<Vec<OutboundMessage>>>,
}

impl RecordingDispatcher {
    fn calls(&self) -> Vec<OutboundMessage> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .clone()
    }
}

#[async_trait]
impl ChannelDispatcher for RecordingDispatcher {
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, garyx_channels::ChannelError> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .push(request);
        Ok(SendMessageResult {
            message_ids: vec!["msg-bound-http".to_owned()],
        })
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        vec![ChannelInfo {
            channel: "telegram".to_owned(),
            account_id: "bot1".to_owned(),
            is_running: true,
        }]
    }

    fn build_stream_event_callback(
        &self,
        target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback> {
        let calls = self.calls.clone();
        Some(Arc::new(move |envelope| {
            if let StreamEvent::Delta { text } = envelope.event {
                let mut message = OutboundMessage::text(
                    target.channel.clone(),
                    target.account_id.clone(),
                    target.chat_id.clone(),
                    target.delivery_target_type.clone(),
                    target.delivery_target_id.clone(),
                    text,
                );
                message.thread_id = target.thread_id.clone();
                calls
                    .lock()
                    .expect("recording dispatcher lock poisoned")
                    .push(message);
            }
        }))
    }
}

#[async_trait]
impl ProviderRuntime for ReadyProvider {
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
        _on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        Ok(ProviderRunResult {
            run_id: "ready-provider".to_owned(),
            thread_id: options.thread_id.clone(),
            response: String::new(),
            session_messages: Vec::new(),
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

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

#[async_trait]
impl ProviderRuntime for BlockingReplyProvider {
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
        self.started.notify_one();
        self.release.notified().await;
        on_chunk(StreamEvent::Delta {
            text: self.text.clone(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "blocking-reply-provider".to_owned(),
            thread_id: options.thread_id.clone(),
            response: self.text.clone(),
            session_messages: Vec::new(),
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

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

#[async_trait]
impl ProviderRuntime for QueueAcceptingProvider {
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
        _on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(ProviderRunResult {
            run_id: "queue-accepting-provider".to_owned(),
            thread_id: options.thread_id.clone(),
            response: String::new(),
            session_messages: Vec::new(),
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

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn add_streaming_input(&self, _thread_id: &str, _input: QueuedUserInput) -> bool {
        self.queued_inputs.fetch_add(1, Ordering::SeqCst);
        true
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

#[async_trait]
impl ProviderRuntime for SlowProvider {
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
        _on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        Ok(ProviderRunResult {
            run_id: "slow-provider".to_owned(),
            thread_id: options.thread_id.clone(),
            response: String::new(),
            session_messages: Vec::new(),
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            duration_ms: self.delay_ms as i64,
        })
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

#[async_trait]
impl ProviderRuntime for WorkspaceRecordingProvider {
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
        _on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        *self.observed_workspace_dir.lock().unwrap() = Some(options.workspace_dir.clone());
        Ok(ProviderRunResult {
            run_id: "workspace-recording-provider".to_owned(),
            thread_id: options.thread_id.clone(),
            response: String::new(),
            session_messages: Vec::new(),
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

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

#[async_trait]
impl ProviderRuntime for MetadataRecordingProvider {
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
        _on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.observed.lock().unwrap().push(options.clone());
        Ok(ProviderRunResult {
            run_id: "metadata-recording-provider".to_owned(),
            thread_id: options.thread_id.clone(),
            response: String::new(),
            session_messages: Vec::new(),
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

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

async fn test_state_with_provider() -> Arc<AppState> {
    let mut config = GaryxConfig::default();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("ready-provider", Arc::new(ReadyProvider))
        .await;
    bridge.set_route("api", "main", "ready-provider").await;
    bridge.set_default_provider_key("ready-provider").await;

    AppStateBuilder::new(config).with_bridge(bridge).build()
}

async fn test_state_with_slow_provider() -> (Arc<AppState>, Arc<MultiProviderBridge>) {
    let mut config = GaryxConfig::default();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("slow-provider", Arc::new(SlowProvider { delay_ms: 5_000 }))
        .await;
    bridge.set_route("api", "main", "slow-provider").await;
    bridge.set_default_provider_key("slow-provider").await;

    (
        AppStateBuilder::new(config)
            .with_bridge(bridge.clone())
            .build(),
        bridge,
    )
}

async fn recording_state(
    account_agent_id: Option<&str>,
    disabled_agent_ids: &[&str],
) -> (
    Arc<AppState>,
    Arc<MultiProviderBridge>,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<ProviderRunOptions>>>,
) {
    let mut config = GaryxConfig::default();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: account_agent_id.map(ToOwned::to_owned),
            workspace_dir: None,
            workspace_mode: None,
        },
    );
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    for agent_id in disabled_agent_ids {
        custom_agents
            .set_enabled(agent_id, false)
            .await
            .expect("disable test agent");
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider(
            "metadata-recording-provider",
            Arc::new(MetadataRecordingProvider {
                calls: calls.clone(),
                observed: observed.clone(),
            }),
        )
        .await;
    bridge
        .set_route("api", "main", "metadata-recording-provider")
        .await;
    bridge
        .set_default_provider_key("metadata-recording-provider")
        .await;
    bridge
        .replace_agent_profiles(custom_agents.snapshot().await)
        .await;
    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .with_custom_agent_store(custom_agents)
        .build();
    (state, bridge, calls, observed)
}

fn test_state_no_bridge() -> Arc<AppState> {
    AppStateBuilder::new(GaryxConfig::default()).build()
}

async fn test_state_with_unready_provider_runtime() -> Arc<AppState> {
    let mut config = GaryxConfig::default();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("ready-provider", Arc::new(ReadyProvider))
        .await;
    bridge.set_route("api", "main", "ready-provider").await;
    bridge.set_default_provider_key("ready-provider").await;

    AppStateBuilder::new(config)
        .with_bridge(bridge)
        .with_provider_runtime_ready(false)
        .build()
}

fn test_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/chat/health", axum::routing::get(chat_health))
        .route("/api/chat/start", axum::routing::post(chat_start))
        .route("/api/chat/interrupt", axum::routing::post(chat_interrupt))
        .route(
            "/api/chat/stream-input",
            axum::routing::post(chat_stream_input),
        )
        .with_state(state)
}

struct Task2571ThreadSubtitleCapture {
    thread_id: String,
    workspace_dir: String,
    active_recent: Value,
    active_summary: Value,
    completed_recent: Value,
    completed_summary: Value,
}

async fn task_2571_json_request(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = crate::test_support::authed_request()
        .method(method)
        .uri(uri);
    let body = match body {
        Some(payload) => {
            request = request.header("content-type", "application/json");
            Body::from(payload.to_string())
        }
        None => Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let payload = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "{method} {uri} returned non-JSON body: {error}; body={}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, payload)
}

fn task_2571_thread_row(page: &Value, thread_id: &str) -> Value {
    page["threads"]
        .as_array()
        .expect("thread page rows")
        .iter()
        .find(|row| row["thread_id"] == thread_id)
        .unwrap_or_else(|| panic!("missing {thread_id} in {page}"))
        .clone()
}

/// Captures the exact gateway payloads used by the iOS subtitle repro. This
/// intentionally drives the production HTTP routes and provider persistence:
/// create a private-workspace thread, submit a user message, inspect both
/// projections while the provider is blocked, then let the run commit and
/// inspect both projections again.
async fn task_2571_capture_thread_subtitle_payloads() -> Task2571ThreadSubtitleCapture {
    let data_dir = tempdir().unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let prompt = "Latest user sentence";
    let reply = "Assistant answer";
    let mut config = crate::test_support::with_gateway_auth(GaryxConfig::default());
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider(
            "task-2571-blocking-provider",
            Arc::new(BlockingReplyProvider {
                started: started.clone(),
                release: release.clone(),
                text: reply.to_owned(),
            }),
        )
        .await;
    bridge
        .set_route("api", "main", "task-2571-blocking-provider")
        .await;
    bridge
        .set_default_provider_key("task-2571-blocking-provider")
        .await;

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .build();
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    let router = crate::route_graph::build_router(state);

    let (status, created) = task_2571_json_request(
        &router,
        "POST",
        "/api/threads",
        Some(json!({
            "label": "New Thread",
            "noWorkspace": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create response: {created}");
    let thread_id = created["thread_id"].as_str().expect("thread id").to_owned();
    let workspace_dir = created["workspace_dir"]
        .as_str()
        .expect("private workspace")
        .to_owned();

    let (status, started_payload) = task_2571_json_request(
        &router,
        "POST",
        "/api/chat/start",
        Some(json!({
            "threadId": thread_id,
            "message": prompt,
            "waitForResponse": false
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "chat/start response: {started_payload}"
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .expect("provider should block after the user message is accepted");

    // The provider notification fires at the provider boundary. Wait for the
    // independently persisted active summary to land too, so the captured
    // pair is stable rather than scheduler-dependent.
    let active_summary = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let (status, page) =
                task_2571_json_request(&router, "GET", "/api/thread-summaries?limit=100", None)
                    .await;
            assert_eq!(status, StatusCode::OK);
            let row = task_2571_thread_row(&page, &thread_id);
            if row["active_run_id"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
                && row["message_count"]
                    .as_u64()
                    .is_some_and(|count| count >= 2)
            {
                break row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("active summary projection should settle while provider is blocked");
    let active_recent = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let (status, page) =
                task_2571_json_request(&router, "GET", "/api/recent-threads?limit=200", None).await;
            assert_eq!(status, StatusCode::OK);
            let row = task_2571_thread_row(&page, &thread_id);
            if row["active_run_id"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
                && row["message_count"]
                    .as_u64()
                    .is_some_and(|count| count >= 2)
            {
                break row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("active Recent projection should settle while provider is blocked");

    release.notify_one();
    let (completed_recent, completed_summary) =
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let (_, recent_page) =
                    task_2571_json_request(&router, "GET", "/api/recent-threads?limit=200", None)
                        .await;
                let (_, summary_page) =
                    task_2571_json_request(&router, "GET", "/api/thread-summaries?limit=100", None)
                        .await;
                let recent = task_2571_thread_row(&recent_page, &thread_id);
                let summary = task_2571_thread_row(&summary_page, &thread_id);
                if recent["last_message_preview"] == prompt
                    && summary["last_message_preview"] == prompt
                    && summary["active_run_id"].is_null()
                {
                    break (recent, summary);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("terminal projections should settle");

    Task2571ThreadSubtitleCapture {
        thread_id,
        workspace_dir,
        active_recent,
        active_summary,
        completed_recent,
        completed_summary,
    }
}

#[tokio::test]
async fn task_2571_new_thread_preview_is_present_while_first_run_is_active() {
    let capture = task_2571_capture_thread_subtitle_payloads().await;
    eprintln!(
        "TASK2571_NEW_THREAD_CAPTURE={}",
        json!({
            "thread_id": capture.thread_id,
            "workspace_dir": capture.workspace_dir,
            "active_recent": capture.active_recent,
            "active_summary": capture.active_summary,
            "completed_recent": capture.completed_recent,
            "completed_summary": capture.completed_summary,
        })
    );

    // Once chat/start has accepted the user message, both list projections
    // expose that sentence even while the provider is still running.
    assert_eq!(
        capture.active_recent["last_message_preview"],
        "Latest user sentence"
    );
    assert_eq!(
        capture.active_summary["last_message_preview"],
        "Latest user sentence"
    );
}

#[tokio::test]
async fn task_2571_recent_and_summary_routes_agree_after_completed_run() {
    let capture = task_2571_capture_thread_subtitle_payloads().await;
    eprintln!(
        "TASK2571_SOURCE_CONSISTENCY={}",
        json!({
            "recent": capture.completed_recent,
            "summary": capture.completed_summary,
        })
    );

    // Both endpoints feed the same iOS summary cache, so the visible subtitle
    // must not depend on which request completes last.
    assert_eq!(
        capture.completed_recent["last_message_preview"],
        capture.completed_summary["last_message_preview"]
    );
}

#[tokio::test]
async fn test_chat_start_http_returns_503_while_provider_runtime_starts() {
    let state = test_state_with_unready_provider_runtime().await;
    let router = test_router(state.clone());
    let thread_id = "thread::chat-start-runtime-starting";

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "threadId": thread_id,
                "message": "hello",
                "waitForResponse": false
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "gateway_provider_runtime_starting");
    assert!(
        state
            .threads
            .thread_store
            .get(thread_id)
            .await
            .unwrap()
            .is_none(),
        "startup rejection should not create or mutate a thread"
    );
}

#[tokio::test]
async fn test_chat_start_http_returns_accepted() {
    let state = test_state_with_provider().await;
    state
        .threads
        .thread_store
        .set(
            "thread::chat-start-http",
            json!({
                "thread_id": "thread::chat-start-http",
                "agent_id": "claude",
                "provider_type": "claude_code"
            }),
        )
        .await
        .unwrap();
    let router = test_router(state.clone());

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "threadId": "thread::chat-start-http",
                "message": "hello",
                "waitForResponse": false
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "accepted");
    assert_eq!(json["threadId"], "thread::chat-start-http");
    let run_id = json["runId"].as_str().expect("run id");
    assert!(!run_id.is_empty());
}

#[tokio::test]
async fn test_chat_start_same_durable_intent_dispatches_provider_exactly_once() {
    let (state, _bridge, calls, _observed) = recording_state(Some("claude"), &[]).await;
    let thread_id = "thread::durable-dispatch-count-one";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "claude",
                "provider_type": "claude_code"
            }),
        )
        .await
        .unwrap();
    let router = test_router(state.clone());
    let payload = json!({
        "threadId": thread_id,
        "message": "deliver once",
        "clientIntentId": "intent-provider-count-one",
        "idempotencyScope": {
            "identity": "integration-test-client",
            "epoch": 1
        },
        "waitForResponse": false
    });

    let send = |router: Router| {
        let body = payload.clone();
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/chat/start")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let payload = serde_json::from_slice::<Value>(&bytes).unwrap();
            assert_eq!(status, StatusCode::OK, "{payload}");
            payload
        }
    };

    let first = send(router.clone()).await;
    let second = send(router).await;
    assert_eq!(first["runId"], second["runId"]);
    assert_eq!(first["effectiveRunId"], second["effectiveRunId"]);
    assert_eq!(first["pendingInputId"], second["pendingInputId"]);
    assert_eq!(first["idempotencyReplay"], false);
    assert_eq!(second["idempotencyReplay"], true);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider should receive the admitted run");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "replaying the same durable request must not dispatch the provider twice"
    );
    let stored = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .unwrap()
        .expect("durably admitted thread");
    assert_eq!(stored["label"], "deliver once");
    assert!(
        stored["workspace_dir"]
            .as_str()
            .is_some_and(|path| Path::new(path).is_dir())
    );
    assert!(
        garyx_router::bindings_from_value(&stored)
            .iter()
            .any(|binding| binding.endpoint_key() == "api::main::api-user")
    );
    let admission = state
        .ops
        .garyx_db
        .run_blocking(|db| {
            db.dispatch_admission(&crate::garyx_db::DispatchAdmissionKey {
                scope_identity: "integration-test-client".to_owned(),
                scope_epoch: 1,
                thread_id: thread_id.to_owned(),
                kind: crate::garyx_db::DispatchAdmissionKind::ChatStart,
                client_intent_id: "intent-provider-count-one".to_owned(),
            })
        })
        .await
        .unwrap()
        .expect("durable admission");
    assert_eq!(
        admission.state,
        crate::garyx_db::DispatchAdmissionState::Accepted
    );
}

#[tokio::test]
async fn quota_recovery_never_queues_continue_into_an_active_run() {
    let thread_id = "thread::quota-busy-race";
    let active_run_id = "run::already-active";
    let blocked_run_id = "run::quota-blocked";
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let queued_inputs = Arc::new(AtomicUsize::new(0));
    let mut config = GaryxConfig::default();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
        },
    );
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider(
            "queue-accepting-provider",
            Arc::new(QueueAcceptingProvider {
                started: started.clone(),
                release: release.clone(),
                queued_inputs: queued_inputs.clone(),
            }),
        )
        .await;
    bridge
        .set_route("api", "main", "queue-accepting-provider")
        .await;
    bridge
        .set_default_provider_key("queue-accepting-provider")
        .await;
    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .build();
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "claude",
                "provider_type": "claude_code"
            }),
        )
        .await
        .unwrap();
    state
        .ops
        .garyx_db
        .run_thread_data_startup_migrations()
        .unwrap();
    state
        .ops
        .garyx_db
        .write_thread_record_with_projections(thread_id, "{}", None, None)
        .unwrap();

    let active = AdmittedRun::thread_bound(
        state.threads.thread_store.clone(),
        AgentRunRequest::new(
            thread_id,
            "user run",
            active_run_id,
            "api",
            "main",
            Default::default(),
        ),
    )
    .await
    .unwrap();
    bridge.dispatch(active, None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .expect("the non-durable user run should become active");

    let job = state
        .ops
        .garyx_db
        .register_quota_recovery_job(crate::garyx_db::NewQuotaRecoveryJob {
            thread_id,
            provider: "claude_code",
            blocked_run_id,
            blocked_seq: 1,
            quota_window: Some("primary"),
            reset_at: Some("2026-07-23T00:00:00Z"),
            due_at: "2026-07-23T00:01:00Z",
        })
        .unwrap();
    let claim_token = "claim::busy-race";
    state
        .ops
        .garyx_db
        .claim_next_due_quota_recovery("2026-07-23T00:01:01Z", claim_token, "2026-07-23T00:03:01Z")
        .unwrap()
        .expect("recovery generation should be claimed");
    let admission_key = crate::garyx_db::DispatchAdmissionKey {
        scope_identity: "__quota_recovery__".to_owned(),
        scope_epoch: 1,
        thread_id: thread_id.to_owned(),
        kind: crate::garyx_db::DispatchAdmissionKind::ChatStart,
        client_intent_id: job.dispatch_intent_id.clone(),
    };
    let request = ChatRequest {
        message: "continue".to_owned(),
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        thread_id: Some(thread_id.to_owned()),
        client_intent_id: Some(job.dispatch_intent_id.clone()),
        idempotency_scope: Some(IdempotencyScope {
            identity: "__quota_recovery__".to_owned(),
            epoch: 1,
        }),
        bot: None,
        from_id: "garyx-quota-recovery".to_owned(),
        account_id: "main".to_owned(),
        wait_for_response: false,
        workspace_path: None,
        provider_type: None,
        metadata: HashMap::from([
            ("internal_dispatch".to_owned(), Value::Bool(true)),
            ("quota_recovery".to_owned(), Value::Bool(true)),
        ]),
    };

    let result = start_chat_run_with_quota_recovery(
        &state,
        request,
        crate::garyx_db::QuotaRecoveryClaimWitness {
            job_id: job.job_id.clone(),
            claim_token: claim_token.to_owned(),
        },
    )
    .await;

    let (status, payload) = match result {
        Ok(_) => panic!("quota recovery must refuse a busy thread"),
        Err(error) => error,
    };
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(payload.0["error"], "quota_recovery_active_run");
    assert_eq!(queued_inputs.load(Ordering::SeqCst), 0);
    assert!(
        state
            .ops
            .garyx_db
            .dispatch_admission(&admission_key)
            .unwrap()
            .is_none(),
        "refused recovery must not commit a durable admission row"
    );
    assert_eq!(
        state
            .ops
            .garyx_db
            .quota_recovery_job(&job.job_id)
            .unwrap()
            .unwrap()
            .state,
        crate::garyx_db::QuotaRecoveryState::Superseded
    );

    release.notify_one();
}

#[tokio::test]
async fn test_threadless_durable_chat_atomically_claims_one_thread_and_dispatches_once() {
    let (state, _bridge, calls, _observed) = recording_state(Some("claude"), &[]).await;
    let router = test_router(state.clone());
    let payload = json!({
        "message": "create once and deliver once",
        "clientIntentId": "threadless-intent-provider-count-one",
        "idempotencyScope": {
            "identity": "threadless-integration-test-client",
            "epoch": 1
        },
        "accountId": "main",
        "fromId": "api-user",
        "waitForResponse": false
    });

    let send = |router: Router| {
        let body = payload.clone();
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/chat/start")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let payload = serde_json::from_slice::<Value>(&bytes).unwrap();
            assert_eq!(status, StatusCode::OK, "{payload}");
            payload
        }
    };

    let first = send(router.clone()).await;
    let second = send(router).await;
    assert_eq!(first["threadId"], second["threadId"]);
    assert_eq!(first["runId"], second["runId"]);
    assert_eq!(first["idempotencyReplay"], false);
    assert_eq!(second["idempotencyReplay"], true);
    let thread_id = first["threadId"].as_str().expect("thread id");
    let keys = state
        .threads
        .thread_store
        .list_keys(Some("thread::"))
        .await
        .unwrap();
    assert_eq!(keys, vec![thread_id.to_owned()]);
    let owner = state
        .ops
        .garyx_db
        .run_blocking(|db| db.get_thread_channel_endpoint("api::main::api-user"))
        .await
        .unwrap()
        .expect("atomic endpoint owner");
    assert_eq!(owner.thread_id.as_deref(), Some(thread_id));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider should receive the admitted run");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_chat_start_missing_explicit_thread_is_404_before_agent_gate_or_bridge() {
    let (state, bridge, calls, _) =
        recording_state(None, &["claude", "codex", "traex", "antigravity"]).await;
    let thread_id = "thread::missing-explicit-all-disabled";
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "threadId": thread_id,
                "message": "must not run",
                "waitForResponse": false
            })
            .to_string(),
        ))
        .unwrap();

    let response = test_router(state).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(bridge.thread_affinity_for(thread_id).await.is_none());
}

#[tokio::test]
async fn test_chat_start_implicit_thread_fails_with_no_enabled_agent_before_bridge() {
    let (state, _, calls, _) =
        recording_state(None, &["claude", "codex", "traex", "antigravity"]).await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "message": "must not create",
                "waitForResponse": false
            })
            .to_string(),
        ))
        .unwrap();

    let response = test_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"], "no enabled standalone agent is available");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        state
            .threads
            .thread_store
            .list_keys(Some("thread::"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn test_chat_start_implicit_thread_storage_failure_returns_500() {
    let state = test_state_with_provider().await;
    state
        .threads
        .router
        .lock()
        .await
        .set_thread_creator(Arc::new(FailingStorageThreadCreator));
    let request = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "message": "must fail closed",
                "waitForResponse": false
            })
            .to_string(),
        ))
        .unwrap();

    let response = test_router(state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"], "injected creation backend failure");
    assert!(
        state
            .threads
            .thread_store
            .list_keys(Some("thread::"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn test_chat_start_reserved_metadata_cannot_override_fresh_canonical_binding() {
    let (state, _, calls, observed) = recording_state(None, &["codex"]).await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "message": "canonical identity wins",
                "waitForResponse": false,
                "metadata": {
                    "agent_id": "codex",
                    "requested_provider_type": "codex_app_server",
                    "provider_env": {"SHOULD_NOT_SURVIVE": "secret"},
                    "model": "request-model"
                }
            })
            .to_string(),
        ))
        .unwrap();

    let response = test_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let thread_id = payload["threadId"].as_str().expect("created thread id");
    let record = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .unwrap()
        .expect("created thread");
    assert_eq!(record["agent_id"], "claude");

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider called");
    let observed = observed.lock().unwrap();
    let options = observed.first().expect("provider options");
    assert_eq!(options.thread_id, thread_id);
    assert_eq!(options.metadata["agent_id"], "claude");
    assert_eq!(options.metadata["requested_provider_type"], "claude_code");
    assert_eq!(options.metadata["model"], "request-model");
    assert!(
        options
            .metadata
            .get("provider_env")
            .and_then(Value::as_object)
            .is_none_or(|env| !env.contains_key("SHOULD_NOT_SURVIVE"))
    );
}

#[tokio::test]
async fn test_chat_start_existing_disabled_agent_thread_continues() {
    let (state, _, calls, observed) = recording_state(Some("codex"), &["claude"]).await;
    let thread_id = "thread::existing-disabled-agent";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "claude",
                "provider_type": "claude_code"
            }),
        )
        .await
        .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "threadId": thread_id,
                "message": "continue existing binding",
                "waitForResponse": false,
                "metadata": {"agent_id": "codex"}
            })
            .to_string(),
        ))
        .unwrap();

    let response = test_router(state).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider called");
    assert_eq!(observed.lock().unwrap()[0].metadata["agent_id"], "claude");
}

#[tokio::test]
async fn test_chat_start_legacy_unstamped_thread_keeps_bridge_fallback_when_all_disabled() {
    let (state, _, calls, observed) =
        recording_state(None, &["claude", "codex", "traex", "antigravity"]).await;
    let thread_id = "thread::legacy-unstamped-agent";
    state
        .threads
        .thread_store
        .set(thread_id, json!({"thread_id": thread_id}))
        .await
        .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "threadId": thread_id,
                "message": "continue legacy binding",
                "waitForResponse": false
            })
            .to_string(),
        ))
        .unwrap();

    let response = test_router(state).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("legacy thread reaches configured bridge fallback");
    let observed = observed.lock().unwrap();
    assert_eq!(observed[0].thread_id, thread_id);
    assert!(observed[0].metadata.get("agent_id").is_none());
}

#[tokio::test]
async fn test_chat_start_http_forwards_bound_reply_using_run_start_binding_snapshot() {
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let mut config = GaryxConfig::default();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider(
            "blocking-reply-provider",
            Arc::new(BlockingReplyProvider {
                started: started.clone(),
                release: release.clone(),
                text: "reply for bound channel".to_owned(),
            }),
        )
        .await;
    bridge
        .set_route("api", "main", "blocking-reply-provider")
        .await;
    bridge
        .set_default_provider_key("blocking-reply-provider")
        .await;

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .build();
    bridge.set_event_tx(state.ops.events.sender()).await;
    state.replace_channel_dispatcher(dispatcher.clone());
    state
        .threads
        .thread_store
        .set(
            "thread::bound-http",
            json!({
                "thread_id": "thread::bound-http",
                "channel": "api",
                "account_id": "main",
                "from_id": "api-user",
                "workspace_dir": "/tmp/garyx-bound-http",
                "messages": [],
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "bot1",
                    "binding_key": "test-user",
                    "chat_id": "old-chat",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "old-chat",
                    "display_label": "Test User"
                }]
            }),
        )
        .await
        .unwrap();

    let router = test_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "threadId": "thread::bound-http",
                "message": "hello",
                "waitForResponse": false
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    started.notified().await;

    state
        .ops
        .endpoint_binding_mutator
        .detach_endpoint("telegram::bot1::test-user")
        .await
        .unwrap();
    state
        .ops
        .endpoint_binding_mutator
        .bind_endpoint(
            "thread::bound-http",
            ChannelBinding {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                binding_key: "test-user".to_owned(),
                chat_id: "new-chat".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "new-chat".to_owned(),
                display_label: "Test User".to_owned(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    release.notify_one();

    let calls = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let calls = dispatcher.calls();
            if !calls.is_empty() {
                break calls;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bound channel should receive reply");

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].channel, "telegram");
    assert_eq!(calls[0].account_id, "bot1");
    assert_eq!(calls[0].chat_id, "old-chat");
    assert_eq!(calls[0].delivery_target_id, "old-chat");
    assert_eq!(calls[0].content.as_text(), Some("reply for bound channel"));
}

#[tokio::test]
async fn test_chat_start_assigns_private_workspace_to_thread_without_workspace() {
    let data_dir = tempdir().unwrap();
    let observed_workspace_dir = Arc::new(Mutex::new(None));
    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider(
            "workspace-recording-provider",
            Arc::new(WorkspaceRecordingProvider {
                observed_workspace_dir: observed_workspace_dir.clone(),
            }),
        )
        .await;
    bridge
        .set_route("api", "main", "workspace-recording-provider")
        .await;
    bridge
        .set_default_provider_key("workspace-recording-provider")
        .await;
    let state = AppStateBuilder::new(config).with_bridge(bridge).build();
    state
        .threads
        .thread_store
        .set(
            "thread::legacy-empty-workspace",
            json!({
                "thread_id": "thread::legacy-empty-workspace",
                "workspace_dir": Value::Null
            }),
        )
        .await
        .unwrap();
    let router = test_router(state.clone());

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "threadId": "thread::legacy-empty-workspace",
                "message": "hello",
                "waitForResponse": false
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let observed = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if let Some(value) = observed_workspace_dir.lock().unwrap().clone() {
                break value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("provider observed workspace");
    let stored = state
        .threads
        .thread_store
        .get("thread::legacy-empty-workspace")
        .await
        .unwrap()
        .expect("stored thread");
    let workspace_dir = stored["workspace_dir"]
        .as_str()
        .expect("persisted workspace");
    assert!(
        Path::new(workspace_dir).starts_with(data_dir.path().join("thread-workspaces")),
        "workspace_dir should be inside private thread workspace root: {workspace_dir}"
    );
    let observed = observed.expect("provider workspace");
    assert_eq!(
        Path::new(&observed).canonicalize().unwrap(),
        Path::new(workspace_dir).canonicalize().unwrap()
    );
}

#[tokio::test]
async fn test_chat_health() {
    let state = test_state_with_provider().await;
    let router = test_router(state.clone());

    let req = Request::builder()
        .uri("/api/chat/health")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["channel"], "api");
    assert_eq!(json["bridge_ready"], true);
    assert_eq!(json["deliveryCapabilities"]["dispatchAdmission"], 1);
    assert_eq!(json["deliveryCapabilities"]["atomicCreateDispatch"], 1);
    assert_eq!(json["deliveryCapabilities"]["createIntentClaim"], 1);
    assert_eq!(json["deliveryCapabilities"]["promptAttachmentLifecycle"], 1);
    assert_eq!(
        json["deliveryCapabilities"]["explicitScopeRequiredForRecovery"],
        true
    );
}

#[tokio::test]
async fn test_chat_health_no_bridge() {
    let state = test_state_no_bridge();
    let router = test_router(state);

    let req = Request::builder()
        .uri("/api/chat/health")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["bridge_ready"], false);
}

#[tokio::test]
async fn chat_health_omits_delivery_capabilities_for_unrelated_custom_store() {
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_thread_store(Arc::new(InMemoryThreadStore::new()))
        .build();
    let response = test_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/chat/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json.get("deliveryCapabilities").is_none());
}

#[tokio::test]
async fn test_chat_interrupt_http_aborts_active_thread_run() {
    let (state, bridge) = test_state_with_slow_provider().await;
    let thread_id = "thread::chat-interrupt-http";
    let run_id = "run-chat-interrupt-http";
    state
        .threads
        .thread_store
        .set(thread_id, json!({}))
        .await
        .unwrap();
    let admitted = AdmittedRun::thread_bound(
        state.threads.thread_store.clone(),
        AgentRunRequest::new(
            thread_id,
            "keep running",
            run_id,
            "api",
            "main",
            Default::default(),
        ),
    )
    .await
    .unwrap();

    bridge.dispatch(admitted, None).await.unwrap();
    let router = test_router(state);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(bridge.is_run_active(run_id).await);

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/interrupt")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "threadId": thread_id })).unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "interrupted");
    assert_eq!(json["threadId"], thread_id);
    assert_eq!(json["abortedRuns"], json!([run_id]));
    assert!(!bridge.is_run_active(run_id).await);
}

#[tokio::test]
async fn test_chat_stream_input_http_returns_no_active_session_without_run() {
    let state = test_state_with_provider().await;
    let router = test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/stream-input")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "threadId": "thread::chat-stream-input-http",
                "clientIntentId": "intent-1",
                "message": "follow up"
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "no_active_session");
    assert_eq!(json["threadId"], "thread::chat-stream-input-http");
    assert_eq!(json["clientIntentId"], "intent-1");
}

#[test]
fn test_chat_request_defaults() {
    let req: ChatRequest = serde_json::from_value(json!({ "message": "hello" })).unwrap();
    assert_eq!(req.message, "hello");
    assert_eq!(req.from_id, "api-user");
    assert_eq!(req.account_id, "main");
    assert!(req.wait_for_response);
    assert!(req.thread_id.is_none());
    assert!(req.bot.is_none());
    assert!(req.workspace_path.is_none());
    assert!(req.images.is_empty());
}

#[test]
fn test_chat_request_custom_fields() {
    let req: ChatRequest = serde_json::from_value(json!({
        "message": "hi",
        "threadId": "thread::custom",
        "fromId": "user-42",
        "accountId": "secondary",
        "waitForResponse": false,
        "timeoutSeconds": 60,
        "workspacePath": "/tmp/custom-workspace",
        "images": [
            {
                "data": "abc123==",
                "media_type": "image/png"
            }
        ],
        "providerMetadata": {
            "provider_env": {
                "CLAUDE_CODE_OAUTH_TOKEN": "token-123"
            }
        }
    }))
    .unwrap();

    assert_eq!(req.thread_id.as_deref(), Some("thread::custom"));
    assert_eq!(req.from_id, "user-42");
    assert_eq!(req.account_id, "secondary");
    assert!(!req.wait_for_response);
    assert_eq!(req.workspace_path.as_deref(), Some("/tmp/custom-workspace"));
    assert_eq!(req.images.len(), 1);
    assert_eq!(req.images[0].media_type, "image/png");
    assert_eq!(req.images[0].data, "abc123==");
    // `providerMetadata` is no longer a request field: legacy payloads still
    // deserialize, but client env can never enter run metadata. Provider env
    // resolution is server-side only (agent/thread snapshot via the bridge).
}

#[test]
fn test_chat_request_accepts_bot_selector() {
    let req: ChatRequest = serde_json::from_value(json!({
        "message": "hi",
        "bot": "telegram:main"
    }))
    .unwrap();
    assert_eq!(req.bot.as_deref(), Some("telegram:main"));
    assert!(req.thread_id.is_none());
}

#[test]
fn test_chat_request_accepts_thread_id_alias() {
    let req: ChatRequest = serde_json::from_value(json!({
        "message": "hi",
        "threadId": "thread::custom"
    }))
    .unwrap();
    assert_eq!(req.thread_id.as_deref(), Some("thread::custom"));
}

#[test]
fn test_chat_request_accepts_client_intent_id_alias() {
    let req: ChatRequest = serde_json::from_value(json!({
        "message": "hi",
        "clientIntentId": "00000000-0000-0000-0000-000000000001"
    }))
    .unwrap();
    assert_eq!(
        req.client_intent_id.as_deref(),
        Some("00000000-0000-0000-0000-000000000001")
    );
}

#[test]
fn test_interrupt_request_accepts_thread_id_alias() {
    let req: InterruptRequest = serde_json::from_value(json!({
        "thread_id": "thread::custom"
    }))
    .unwrap();
    assert_eq!(req.thread_id.as_deref(), Some("thread::custom"));
}

#[test]
fn test_stream_input_request_accepts_thread_id_alias() {
    let req: StreamInputRequest = serde_json::from_value(json!({
        "threadId": "thread::custom",
        "clientIntentId": "intent-1",
        "message": "hello"
    }))
    .unwrap();
    assert_eq!(req.thread_id.as_deref(), Some("thread::custom"));
    assert_eq!(req.client_intent_id.as_deref(), Some("intent-1"));
}

fn committed_ws_control(thread_id: &str, run_id: &str, seq: u64, kind: &str) -> Value {
    json!({
        "type": "committed_message",
        "thread_id": thread_id,
        "run_id": run_id,
        "seq": seq,
        "message": {
            "role": "system",
            "kind": "control",
            "internal": true,
            "internal_kind": "control",
            "control": {
                "kind": kind,
                "thread_id": thread_id,
                "run_id": run_id,
                "at": "2026-06-20T00:00:00Z",
                "status": "interrupted"
            }
        }
    })
}

#[test]
fn chat_ws_terminal_detection_reads_committed_control_kind() {
    let terminal = committed_ws_control(
        "thread::chat-ws-terminal-detect",
        "run::chat-ws-terminal-detect",
        1,
        "run_complete",
    );
    assert!(is_terminal_bus_record_for_run(
        &terminal,
        "run::chat-ws-terminal-detect"
    ));

    let top_level = json!({
        "type": "run_complete",
        "thread_id": "thread::chat-ws-terminal-detect",
        "run_id": "run::chat-ws-terminal-detect"
    });
    assert!(
        !is_terminal_bus_record_for_run(&top_level, "run::chat-ws-terminal-detect"),
        "top-level lifecycle shapes are not produced on the committed bus"
    );

    let done = committed_ws_control(
        "thread::chat-ws-terminal-detect",
        "run::chat-ws-terminal-detect",
        2,
        "done",
    );
    assert!(!is_terminal_bus_record_for_run(
        &done,
        "run::chat-ws-terminal-detect"
    ));
}

#[test]
fn chat_ws_forward_outcome_distinguishes_gap_from_closed_client() {
    let (open_tx, _open_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut sent_payloads = HashMap::new();
    let mut last_sent_seq = 1;
    let gap = committed_ws_control("thread::chat-ws-forward", "run::chat-ws-forward", 3, "done");
    assert_eq!(
        forward_chat_ws_committed_value(&open_tx, &gap, &mut sent_payloads, &mut last_sent_seq),
        ChatWsForwardOutcome::Gap
    );
    assert_eq!(last_sent_seq, 1, "gap does not advance the cursor");

    let (closed_tx, closed_rx) = tokio::sync::mpsc::unbounded_channel();
    drop(closed_rx);
    let mut sent_payloads = HashMap::new();
    let mut last_sent_seq = 0;
    let terminal = committed_ws_control(
        "thread::chat-ws-forward",
        "run::chat-ws-forward",
        1,
        "run_complete",
    );
    assert_eq!(
        forward_chat_ws_committed_value(
            &closed_tx,
            &terminal,
            &mut sent_payloads,
            &mut last_sent_seq
        ),
        ChatWsForwardOutcome::Closed
    );
}

#[tokio::test]
async fn chat_ws_committed_stream_forwards_terminal_record_then_exits() {
    let state = test_state_with_provider().await;
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel();
    let thread_id = "thread::chat-ws-terminal-forward".to_owned();
    let run_id = "run::chat-ws-terminal-forward".to_owned();
    let handle =
        spawn_chat_ws_committed_stream(state.clone(), out_tx, thread_id.clone(), run_id.clone());
    let terminal = committed_ws_control(&thread_id, &run_id, 1, "run_complete");

    state
        .ops
        .events
        .sender()
        .send(terminal.to_string())
        .expect("event bus has an active committed stream subscriber");

    let forwarded = tokio::time::timeout(std::time::Duration::from_secs(1), out_rx.recv())
        .await
        .expect("terminal committed record should be forwarded")
        .expect("stream output should remain open until terminal is sent");
    assert_eq!(forwarded, terminal);

    tokio::time::timeout(std::time::Duration::from_secs(1), handle)
        .await
        .expect("terminal committed record should stop the WS stream task")
        .expect("WS stream task should not panic");
}
