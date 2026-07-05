use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_channels::{
    ChannelDispatcher, ChannelInfo, OutboundMessage, SendMessageResult, StreamDispatchCallback,
    StreamingDispatchTarget,
};
use garyx_models::config::{ApiAccount, GaryxConfig};
use garyx_models::provider::{
    AgentRunRequest, ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent,
};
use serde_json::{Value, json};
use std::path::Path;
use tempfile::tempdir;
use tower::ServiceExt;

use crate::application::chat::contracts::{ChatRequest, InterruptRequest, StreamInputRequest};
use crate::server::{AppState, AppStateBuilder};

use super::{
    ChatWsForwardOutcome, chat_health, chat_interrupt, chat_start, chat_stream_input,
    forward_chat_ws_committed_value, is_terminal_bus_record_for_run,
    spawn_chat_ws_committed_stream,
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
struct WorkspaceRecordingProvider {
    observed_workspace_dir: Arc<Mutex<Option<Option<String>>>>,
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
        _router: Arc<tokio::sync::Mutex<garyx_router::MessageRouter>>,
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
impl AgentLoopProvider for ReadyProvider {
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
impl AgentLoopProvider for BlockingReplyProvider {
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
impl AgentLoopProvider for SlowProvider {
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
impl AgentLoopProvider for WorkspaceRecordingProvider {
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

async fn test_state_with_provider() -> Arc<AppState> {
    let mut config = GaryxConfig::default();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
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
            agent_id: "claude".to_owned(),
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

fn test_state_no_bridge() -> Arc<AppState> {
    AppStateBuilder::new(GaryxConfig::default()).build()
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

#[tokio::test]
async fn test_chat_start_http_returns_accepted() {
    let state = test_state_with_provider().await;
    let router = test_router(state);

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
            agent_id: "claude".to_owned(),
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
        .await;

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
                    "chat_id": "new-chat",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "new-chat",
                    "display_label": "Test User"
                }]
            }),
        )
        .await;
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
            agent_id: "claude".to_owned(),
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
        .await;
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
    let router = test_router(state);

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
async fn test_chat_interrupt_http_aborts_active_thread_run() {
    let (state, bridge) = test_state_with_slow_provider().await;
    let router = test_router(state);
    let thread_id = "thread::chat-interrupt-http";
    let run_id = "run-chat-interrupt-http";

    bridge
        .start_agent_run(
            AgentRunRequest::new(
                thread_id,
                "keep running",
                run_id,
                "api",
                "main",
                Default::default(),
            ),
            None,
        )
        .await
        .unwrap();
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
    assert!(req.provider_metadata.is_empty());
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
    assert_eq!(
        req.provider_metadata["provider_env"]["CLAUDE_CODE_OAUTH_TOKEN"],
        "token-123"
    );
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
