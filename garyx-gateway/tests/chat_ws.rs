use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_gateway::server::AppStateBuilder;
use garyx_models::config::{ApiAccount, GaryxConfig};
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const TEST_GATEWAY_TOKEN: &str = "chat-ws-test-token";

struct WsTestProvider;

#[async_trait]
impl AgentLoopProvider for WsTestProvider {
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
            text: format!("ws-e2e: {}", options.message),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "chat-ws-test-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "ok".to_owned(),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            success: true,
            error: None,
            input_tokens: 1,
            output_tokens: 1,
            cost: 0.0,
            duration_ms: 1,
        })
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

async fn start_test_gateway() -> SocketAddr {
    let mut config = GaryxConfig::default();
    config.gateway.auth_token = TEST_GATEWAY_TOKEN.to_owned();
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
        .register_provider("api-test-provider", Arc::new(WsTestProvider))
        .await;
    bridge.set_route("api", "main", "api-test-provider").await;
    bridge.set_default_provider_key("api-test-provider").await;

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .build();
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;

    let router = garyx_gateway::build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    addr
}

fn authed_ws_url(addr: SocketAddr) -> String {
    format!("ws://{addr}/api/chat/ws?token={TEST_GATEWAY_TOKEN}")
}

async fn recv_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let next = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("timeout waiting for ws message")
        .expect("ws stream closed")
        .expect("ws read error");
    let text = next.into_text().expect("expected ws text frame");
    serde_json::from_str(&text).expect("expected json frame")
}

#[tokio::test]
async fn chat_ws_start_streams_events() {
    let addr = start_test_gateway().await;
    let url = authed_ws_url(addr);
    let (mut ws, _) = connect_async(&url).await.expect("ws connect");

    ws.send(Message::Text(
        json!({
            "op": "start",
            "message": "hello ws",
            "threadId": "thread::ws-start",
            "accountId": "main",
            "fromId": "ws-test"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let mut seen = Vec::new();
    for _ in 0..5 {
        let payload = recv_json(&mut ws).await;
        let kind = payload["type"].as_str().unwrap_or_default().to_owned();
        seen.push(kind.clone());
        if kind == "done" {
            break;
        }
    }

    assert!(seen.iter().any(|item| item == "accepted"));
    assert!(seen.iter().any(|item| item == "assistant_delta"));
    assert!(seen.iter().any(|item| item == "done"));
}

#[tokio::test]
async fn chat_ws_supports_input_and_interrupt_ops() {
    let addr = start_test_gateway().await;
    let url = authed_ws_url(addr);
    let (mut ws, _) = connect_async(&url).await.expect("ws connect");

    ws.send(Message::Text(
        json!({
            "op": "input",
            "threadId": "thread::ws-control",
            "message": "follow-up"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let input_payload = recv_json(&mut ws).await;
    assert_eq!(input_payload["type"], "stream_input");
    assert_eq!(input_payload["status"], "no_active_session");
    assert_eq!(input_payload["threadId"], "thread::ws-control");

    ws.send(Message::Text(
        json!({
            "op": "interrupt",
            "threadId": "thread::ws-control"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let interrupt_payload = recv_json(&mut ws).await;
    assert_eq!(interrupt_payload["type"], "interrupt");
    assert_eq!(interrupt_payload["status"], "not_found");
    assert_eq!(interrupt_payload["threadId"], "thread::ws-control");
}

#[tokio::test]
async fn chat_ws_recover_returns_thread_snapshot() {
    let addr = start_test_gateway().await;
    let url = authed_ws_url(addr);
    let (mut ws, _) = connect_async(&url).await.expect("ws connect");

    let thread_id = "thread::ws-recover";
    ws.send(Message::Text(
        json!({
            "op": "start",
            "message": "reconnect proof",
            "threadId": thread_id,
            "accountId": "main",
            "fromId": "ws-test"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let mut active_thread_id = thread_id.to_owned();
    loop {
        let payload = recv_json(&mut ws).await;
        let kind = payload["type"].as_str().unwrap_or_default();
        if kind == "accepted" {
            if let Some(id) = payload["threadId"].as_str() {
                active_thread_id = id.to_owned();
            }
        }
        if kind == "done" || kind == "error" {
            break;
        }
    }

    ws.send(Message::Text(
        json!({
            "op": "recover",
            "threadId": active_thread_id,
            "limit": 50,
            "includeToolMessages": true
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let snapshot_payload = recv_json(&mut ws).await;
    assert_eq!(snapshot_payload["type"], "snapshot");
    assert_eq!(snapshot_payload["threadId"], active_thread_id);
    assert_eq!(snapshot_payload["payload"]["ok"], true);
    assert!(
        snapshot_payload["payload"]["messages"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false),
        "expected non-empty history snapshot"
    );
}
