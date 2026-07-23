use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{BridgeError, ProviderRuntime, StreamCallback};
use garyx_gateway::garyx_db::GaryxDbService;
use garyx_gateway::server::{AppState, AppStateBuilder};
use garyx_models::config::{
    ApiAccount, GaryxConfig, OwnerTargetConfig, TelegramAccount, telegram_account_to_plugin_entry,
};
use garyx_models::provider::{
    ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput, StreamBoundaryKind,
    StreamEvent,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const TEST_GATEWAY_TOKEN: &str = "chat-ws-test-token";

struct WsTestProvider;
struct WsCountingProvider {
    calls: Arc<AtomicUsize>,
}

type SharedStreamCallback = Arc<dyn Fn(StreamEvent) + Send + Sync>;

struct WsAckBeforeInputResponseProvider {
    callback: Mutex<Option<SharedStreamCallback>>,
    callback_ready: Notify,
    release_run: Notify,
}

impl WsAckBeforeInputResponseProvider {
    fn new() -> Self {
        Self {
            callback: Mutex::new(None),
            callback_ready: Notify::new(),
            release_run: Notify::new(),
        }
    }

    async fn wait_for_callback(&self) {
        if self.callback.lock().await.is_some() {
            return;
        }
        self.callback_ready.notified().await;
    }

    fn release_run(&self) {
        self.release_run.notify_one();
    }
}

#[async_trait]
impl ProviderRuntime for WsTestProvider {
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
            thread_title: None,
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

#[async_trait]
impl ProviderRuntime for WsCountingProvider {
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
        Ok(ProviderRunResult {
            run_id: "chat-ws-counting-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: String::new(),
            session_messages: vec![],
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
impl ProviderRuntime for WsAckBeforeInputResponseProvider {
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
        let callback: SharedStreamCallback = Arc::from(on_chunk);
        *self.callback.lock().await = Some(callback.clone());
        callback(StreamEvent::Delta {
            text: "initial".to_owned(),
        });
        self.callback_ready.notify_waiters();
        self.release_run.notified().await;
        callback(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id: "chat-ws-ack-before-response-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "initial".to_owned(),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 1,
            output_tokens: 1,
            cost: 0.0,
            duration_ms: 1,
        })
    }

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn add_streaming_input(&self, _thread_id: &str, input: QueuedUserInput) -> bool {
        let callback = self.callback.lock().await.clone();
        let Some(callback) = callback else {
            return false;
        };
        callback(StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: input.pending_input_id,
        });
        true
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

async fn start_test_gateway() -> SocketAddr {
    start_test_gateway_with_provider("api-test-provider", Arc::new(WsTestProvider)).await
}

async fn start_test_gateway_with_provider(
    provider_key: &str,
    provider: Arc<dyn ProviderRuntime>,
) -> SocketAddr {
    start_test_gateway_with_provider_context(provider_key, provider)
        .await
        .0
}

async fn start_test_gateway_with_provider_context(
    provider_key: &str,
    provider: Arc<dyn ProviderRuntime>,
) -> (SocketAddr, Arc<MultiProviderBridge>, Arc<AppState>) {
    let mut config = GaryxConfig::default();
    config.gateway.auth_token = TEST_GATEWAY_TOKEN.to_owned();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: None,
            workspace_dir: None,
            workspace_mode: None,
        },
    );
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            telegram_account_to_plugin_entry(&TelegramAccount {
                token: "test-token".to_owned(),
                enabled: true,
                name: Some("Test Bot".to_owned()),
                agent_id: None,
                workspace_dir: None,
                owner_target: Some(OwnerTargetConfig {
                    target_type: "chat_id".to_owned(),
                    target_id: "1000000001".to_owned(),
                }),
                groups: std::collections::HashMap::new(),
            }),
        );

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge.register_provider(provider_key, provider).await;
    bridge.set_route("api", "main", provider_key).await;
    bridge.set_default_provider_key(provider_key).await;

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .with_garyx_db(Arc::new(GaryxDbService::memory().expect("memory garyx db")))
        .build();
    for thread_id in [
        "thread::ws-start",
        "thread::ws-codex-like-queued-input",
        "thread::ws-recover",
    ] {
        state
            .threads
            .thread_store
            .set(thread_id, json!({"thread_id": thread_id}))
            .await
            .expect("seed legacy test thread");
    }
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;

    let router = garyx_gateway::build_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (addr, bridge, state)
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

fn committed_control_kind(payload: &Value) -> Option<&str> {
    (payload.get("type").and_then(Value::as_str) == Some("committed_message"))
        .then(|| {
            payload
                .pointer("/message/control/kind")
                .and_then(Value::as_str)
        })
        .flatten()
}

fn committed_assistant_text(payload: &Value) -> Option<&str> {
    if payload.get("type").and_then(Value::as_str) != Some("committed_message")
        || payload.pointer("/message/role").and_then(Value::as_str) != Some("assistant")
    {
        return None;
    }
    payload
        .pointer("/message/text")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/message/content").and_then(Value::as_str))
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
    let mut assistant_text = Vec::new();
    for _ in 0..5 {
        let payload = recv_json(&mut ws).await;
        let kind = payload["type"].as_str().unwrap_or_default().to_owned();
        if let Some(text) = committed_assistant_text(&payload) {
            assistant_text.push(text.to_owned());
        }
        seen.push(kind.clone());
        if committed_control_kind(&payload) == Some("run_complete") {
            break;
        }
    }

    assert!(seen.iter().any(|item| item == "accepted"));
    assert!(seen.iter().any(|item| item == "committed_message"));
    assert_eq!(assistant_text, vec!["ws-e2e: hello ws"]);
}

#[tokio::test]
async fn chat_ws_missing_explicit_thread_is_not_found_with_all_agents_disabled() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (addr, bridge, _) = start_test_gateway_with_provider_context(
        "counting-provider",
        Arc::new(WsCountingProvider {
            calls: calls.clone(),
        }),
    )
    .await;
    let client = reqwest::Client::new();
    for agent_id in ["claude", "codex", "traex", "antigravity", "grok"] {
        let response = client
            .patch(format!("http://{addr}/api/custom-agents/{agent_id}/toggle"))
            .bearer_auth(TEST_GATEWAY_TOKEN)
            .json(&json!({"enabled": false}))
            .send()
            .await
            .expect("disable agent");
        assert!(response.status().is_success(), "disable {agent_id}");
    }
    let agents: Value = client
        .get(format!("http://{addr}/api/custom-agents"))
        .bearer_auth(TEST_GATEWAY_TOKEN)
        .send()
        .await
        .expect("list agents")
        .json()
        .await
        .expect("agent list json");
    assert!(agents["effective_default_agent_id"].is_null());

    let (mut ws, _) = connect_async(authed_ws_url(addr))
        .await
        .expect("ws connect");
    let thread_id = "thread::ws-missing-all-disabled";
    ws.send(Message::Text(
        json!({
            "op": "start",
            "message": "must not run",
            "threadId": thread_id,
            "accountId": "main",
            "fromId": "ws-test"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let payload = recv_json(&mut ws).await;
    assert_eq!(payload["type"], "error");
    assert_eq!(payload["threadId"], thread_id);
    assert_eq!(payload["error"], "thread not found");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(bridge.thread_affinity_for(thread_id).await.is_none());

    ws.send(Message::Text(
        json!({
            "op": "start",
            "message": "must not create",
            "accountId": "main",
            "fromId": "ws-no-enabled"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let payload = recv_json(&mut ws).await;
    assert_eq!(payload["type"], "error");
    assert_eq!(payload["error"], "no enabled standalone agent is available");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stale_bot_endpoint_reenters_fresh_gate_for_http_and_ws_without_bridge_calls() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (addr, bridge, state) = start_test_gateway_with_provider_context(
        "stale-bot-counting-provider",
        Arc::new(WsCountingProvider {
            calls: calls.clone(),
        }),
    )
    .await;
    let stale_thread_id = "thread::stale-bot-endpoint";
    state
        .threads
        .thread_store
        .set(
            stale_thread_id,
            json!({
                "thread_id": stale_thread_id,
                "agent_id": "claude",
                "channel": "telegram",
                "account_id": "main",
                "from_id": "1000000001",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "1000000001",
                    "chat_id": "1000000001",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "1000000001",
                    "display_label": "Test User"
                }]
            }),
        )
        .await
        .expect("seed stale bot thread");
    let cached = state
        .cached_channel_endpoints()
        .await
        .expect("prime endpoint cache");
    assert!(cached.iter().any(|endpoint| {
        endpoint.thread_id.as_deref() == Some(stale_thread_id)
            && endpoint.endpoint_key == "telegram::main::1000000001"
    }));
    assert!(
        state
            .threads
            .thread_store
            .delete(stale_thread_id)
            .await
            .expect("delete cached endpoint thread")
    );

    let client = reqwest::Client::new();
    for agent_id in ["claude", "codex", "traex", "antigravity", "grok"] {
        let response = client
            .patch(format!("http://{addr}/api/custom-agents/{agent_id}/toggle"))
            .bearer_auth(TEST_GATEWAY_TOKEN)
            .json(&json!({"enabled": false}))
            .send()
            .await
            .expect("disable agent");
        assert!(response.status().is_success(), "disable {agent_id}");
    }

    let response = client
        .post(format!("http://{addr}/api/chat/start"))
        .bearer_auth(TEST_GATEWAY_TOKEN)
        .json(&json!({
            "bot": "telegram:main",
            "message": "must re-enter the fresh binding gate",
            "waitForResponse": false
        }))
        .send()
        .await
        .expect("HTTP chat start");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let payload: Value = response.json().await.expect("HTTP error json");
    assert_eq!(payload["error"], "no enabled standalone agent is available");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(bridge.thread_affinity_for(stale_thread_id).await.is_none());

    let (mut ws, _) = connect_async(authed_ws_url(addr))
        .await
        .expect("ws connect");
    ws.send(Message::Text(
        json!({
            "op": "start",
            "bot": "telegram:main",
            "message": "must re-enter the same fresh binding gate"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send WS start");
    let payload = recv_json(&mut ws).await;
    assert_eq!(payload["type"], "error");
    assert_eq!(payload["error"], "no enabled standalone agent is available");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(bridge.thread_affinity_for(stale_thread_id).await.is_none());
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
async fn chat_ws_codex_like_user_ack_can_arrive_before_stream_input_response() {
    let provider = Arc::new(WsAckBeforeInputResponseProvider::new());
    let addr = start_test_gateway_with_provider("codex-like-provider", provider.clone()).await;
    let url = authed_ws_url(addr);
    let (mut ws, _) = connect_async(&url).await.expect("ws connect");
    let thread_id = "thread::ws-codex-like-queued-input";

    ws.send(Message::Text(
        json!({
            "op": "start",
            "message": "start long run",
            "threadId": thread_id,
            "accountId": "main",
            "fromId": "ws-test"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    tokio::time::timeout(Duration::from_secs(5), provider.wait_for_callback())
        .await
        .expect("run should be active before queuing input");

    ws.send(Message::Text(
        json!({
            "op": "input",
            "threadId": thread_id,
            "clientIntentId": "intent-follow-up-1",
            "message": "follow-up"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let mut ack_payload = None;
    let mut stream_input_payload = None;
    for _ in 0..10 {
        let payload = recv_json(&mut ws).await;
        if committed_control_kind(&payload) == Some("user_ack") {
            ack_payload = Some(payload);
        } else if payload["type"] == "stream_input"
            && payload["clientIntentId"] == "intent-follow-up-1"
        {
            stream_input_payload = Some(payload);
        }
        if ack_payload.is_some() && stream_input_payload.is_some() {
            break;
        }
    }
    let ack_payload = ack_payload.expect("expected committed user_ack");
    let stream_input_payload = stream_input_payload.expect("expected stream_input response");
    assert_eq!(stream_input_payload["type"], "stream_input");
    assert_eq!(stream_input_payload["status"], "queued");
    assert_eq!(stream_input_payload["clientIntentId"], "intent-follow-up-1");
    assert_eq!(ack_payload["thread_id"], thread_id);
    assert_eq!(stream_input_payload["threadId"], thread_id);
    assert_eq!(
        ack_payload["message"]["control"]["pending_input_id"],
        stream_input_payload["pendingInputId"],
        "Codex-style ack and stream_input response should describe the same queued input"
    );

    provider.release_run();
    let mut saw_run_complete = false;
    for _ in 0..5 {
        let payload = recv_json(&mut ws).await;
        if committed_control_kind(&payload) == Some("run_complete") {
            saw_run_complete = true;
            break;
        }
    }
    assert!(
        saw_run_complete,
        "run should finish after releasing the provider"
    );
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
        if kind == "accepted"
            && let Some(id) = payload["threadId"].as_str()
        {
            active_thread_id = id.to_owned();
        }
        if committed_control_kind(&payload) == Some("run_complete") || kind == "error" {
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
