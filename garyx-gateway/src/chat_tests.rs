use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_models::config::{ApiAccount, GaryxConfig};
use garyx_models::provider::{
    AgentRunRequest, ProviderRunOptions, ProviderRunResult, ProviderType,
};
use serde_json::{Value, json};
use std::path::Path;
use tempfile::tempdir;
use tower::ServiceExt;

use crate::application::chat::contracts::{ChatRequest, InterruptRequest, StreamInputRequest};
use crate::server::{AppState, AppStateBuilder};

use super::{chat_health, chat_interrupt, chat_start, chat_stream_input};

struct ReadyProvider;
struct SlowProvider {
    delay_ms: u64,
}
struct WorkspaceRecordingProvider {
    observed_workspace_dir: Arc<Mutex<Option<Option<String>>>>,
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
            "desktop_claude_env": {
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
        req.provider_metadata["desktop_claude_env"]["CLAUDE_CODE_OAUTH_TOKEN"],
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
