use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_models::config::{ApiAccount, GaryxConfig};
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::application::chat::contracts::{ChatRequest, InterruptRequest, StreamInputRequest};
use crate::server::{AppState, AppStateBuilder};

use super::chat_health;

struct ReadyProvider;

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

fn test_state_no_bridge() -> Arc<AppState> {
    AppStateBuilder::new(GaryxConfig::default()).build()
}

fn test_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/chat/health", axum::routing::get(chat_health))
        .with_state(state)
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
        "message": "hello"
    }))
    .unwrap();
    assert_eq!(req.thread_id.as_deref(), Some("thread::custom"));
}

#[test]
fn test_parse_agent_team_delta_prefix_extracts_speaker_metadata() {
    let (speaker, delta) =
        super::parse_agent_team_delta_prefix("[junie] say hi back").expect("speaker prefix");
    assert_eq!(speaker.agent_id, "junie");
    assert_eq!(speaker.agent_display_name, "junie");
    assert_eq!(delta, "say hi back");
}
