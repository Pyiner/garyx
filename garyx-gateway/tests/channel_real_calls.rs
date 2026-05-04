use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_gateway::api::thread_history;
use garyx_gateway::automation::{
    automation_activity, create_automation, delete_automation, get_automation, list_automations,
    run_automation_now, update_automation,
};
use garyx_gateway::chat::{chat_health, chat_ws};
use garyx_gateway::routes::{
    bind_channel_endpoint, create_thread, delete_thread, detach_channel_endpoint, get_thread,
    list_channel_endpoints, list_threads, update_thread,
};
use garyx_gateway::server::{AppState, AppStateBuilder};
use garyx_models::config::{ApiAccount, GaryxConfig, TelegramAccount};
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

#[derive(Debug, Clone)]
struct ProviderCall {
    thread_id: String,
    message: String,
    metadata: HashMap<String, Value>,
    workspace_dir: Option<String>,
}

struct RecordingProvider {
    call_count: AtomicUsize,
    calls: Mutex<Vec<ProviderCall>>,
}

impl RecordingProvider {
    fn new() -> Self {
        Self {
            call_count: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AgentLoopProvider for RecordingProvider {
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
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.calls
            .lock()
            .expect("calls mutex poisoned")
            .push(ProviderCall {
                thread_id: options.thread_id.clone(),
                message: options.message.clone(),
                metadata: options.metadata.clone(),
                workspace_dir: options.workspace_dir.clone(),
            });

        let response = format!("provider-e2e: {}", options.message);
        on_chunk(StreamEvent::Delta {
            text: response.clone(),
        });
        on_chunk(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id: "channel-real-calls".to_owned(),
            thread_id: options.thread_id.clone(),
            response,
            session_messages: Vec::new(),
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

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

fn test_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/threads",
            axum::routing::get(list_threads).post(create_thread),
        )
        .route("/api/threads/history", axum::routing::get(thread_history))
        .route(
            "/api/threads/{key}",
            axum::routing::get(get_thread)
                .patch(update_thread)
                .delete(delete_thread),
        )
        .route(
            "/api/channel-endpoints",
            axum::routing::get(list_channel_endpoints),
        )
        .route(
            "/api/channel-bindings/bind",
            axum::routing::post(bind_channel_endpoint),
        )
        .route(
            "/api/channel-bindings/detach",
            axum::routing::post(detach_channel_endpoint),
        )
        .route(
            "/api/automations",
            axum::routing::get(list_automations).post(create_automation),
        )
        .route(
            "/api/automations/{id}",
            axum::routing::get(get_automation)
                .patch(update_automation)
                .delete(delete_automation),
        )
        .route(
            "/api/automations/{id}/run-now",
            axum::routing::post(run_automation_now),
        )
        .route(
            "/api/automations/{id}/activity",
            axum::routing::get(automation_activity),
        )
        .route("/api/chat/ws", axum::routing::get(chat_ws))
        .route("/api/chat/health", axum::routing::get(chat_health))
        .with_state(state)
}

async fn spawn_http_server(
    state: Arc<AppState>,
) -> (String, oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
    let router = test_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener local addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve test router");
    });
    (format!("http://{}", addr), shutdown_tx, handle)
}

async fn shutdown_http_server(
    shutdown_tx: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
) {
    let _ = shutdown_tx.send(());
    handle.await.expect("join test server");
}

async fn wait_until<F, Fut>(timeout: std::time::Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if check().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn fetch_thread_history(client: &reqwest::Client, base_url: &str, thread_id: &str) -> Value {
    client
        .get(format!("{base_url}/api/threads/history"))
        .query(&[
            ("thread_id", thread_id),
            ("limit", "10"),
            ("include_tool_messages", "false"),
        ])
        .send()
        .await
        .expect("history response")
        .json()
        .await
        .expect("history json")
}

type TestWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn connect_chat_ws(base_url: &str) -> TestWebSocket {
    let ws_url = format!("{}/api/chat/ws", base_url.replacen("http://", "ws://", 1));
    let (ws, _) = connect_async(&ws_url).await.expect("ws connect");
    ws
}

async fn recv_ws_json(ws: &mut TestWebSocket) -> Value {
    let next = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("timeout waiting for ws frame")
        .expect("ws stream closed")
        .expect("ws read error");
    let text = next.into_text().expect("expected text frame");
    serde_json::from_str(&text).expect("expected json frame")
}

async fn run_chat_start(ws: &mut TestWebSocket, payload: Value) -> (String, Vec<Value>) {
    let initial_thread_id = payload
        .get("threadId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    ws.send(Message::Text(payload.to_string().into()))
        .await
        .expect("send ws start");

    let mut thread_id = initial_thread_id;
    let mut events = Vec::new();
    loop {
        let event = recv_ws_json(ws).await;
        if thread_id.is_empty()
            && let Some(value) = event.get("threadId").and_then(Value::as_str)
        {
            thread_id = value.to_owned();
        }
        let done = matches!(
            event.get("type").and_then(Value::as_str),
            Some("done" | "error")
        );
        events.push(event);
        if done {
            break;
        }
    }

    (thread_id, events)
}

async fn make_state_with_recording_provider(provider: Arc<RecordingProvider>) -> Arc<AppState> {
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

    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "bot1".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&TelegramAccount {
                token: "fake-token".to_owned(),
                enabled: true,
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                groups: Default::default(),
            }),
        );

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("recording-provider", provider)
        .await;
    bridge.set_default_provider_key("recording-provider").await;
    bridge.set_route("api", "main", "recording-provider").await;
    bridge
        .set_route("telegram", "bot1", "recording-provider")
        .await;

    let cron_data_dir =
        std::env::temp_dir().join(format!("gary-automation-test-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(cron_data_dir.join("cron").join("jobs"))
        .await
        .expect("create cron jobs dir");
    let cron_service = Arc::new(garyx_gateway::CronService::new(cron_data_dir));

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge)
        .with_cron_service(cron_service)
        .build();
    state
        .ops
        .cron_service
        .as_ref()
        .expect("cron service configured")
        .set_dispatch_runtime(
            state.threads.thread_store.clone(),
            state.threads.router.clone(),
            state.integration.bridge.clone(),
            state.channel_dispatcher(),
            state.ops.thread_logs.clone(),
            HashMap::new(),
            state.ops.custom_agents.clone(),
            state.ops.agent_teams.clone(),
        )
        .await;
    state
        .integration
        .bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    state
        .integration
        .bridge
        .set_event_tx(state.ops.events.sender())
        .await;
    state
}

#[tokio::test]
async fn test_api_channel_ws_real_http_call() {
    let provider = Arc::new(RecordingProvider::new());
    let state = make_state_with_recording_provider(provider.clone()).await;
    let (base_url, shutdown_tx, handle) = spawn_http_server(state).await;
    let client = reqwest::Client::new();
    let mut ws = connect_chat_ws(&base_url).await;

    let (thread_id, events) = run_chat_start(
        &mut ws,
        json!({
            "op": "start",
            "message": "hello api",
            "accountId": "main",
            "fromId": "api-e2e-user"
        }),
    )
    .await;
    assert!(thread_id.starts_with("thread::"));
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "accepted" && event["threadId"] == thread_id)
    );
    assert!(events.iter().any(|event| event["type"] == "assistant_delta"
        && event["delta"] == Value::String("provider-e2e: hello api".to_owned())));
    assert!(events.iter().any(|event| event["type"] == "done"));

    let history_ready = wait_until(std::time::Duration::from_secs(5), || {
        let client = client.clone();
        let base_url = base_url.clone();
        let thread_id = thread_id.clone();
        async move {
            let history = fetch_thread_history(&client, &base_url, &thread_id).await;
            history["message_stats"]["returned_messages"]
                .as_u64()
                .unwrap_or(0)
                >= 2
        }
    })
    .await;
    assert!(history_ready, "api chat history was not persisted");

    let history = fetch_thread_history(&client, &base_url, &thread_id).await;
    assert_eq!(history["ok"], true);
    assert_eq!(
        history["thread"]["thread_id"],
        Value::String(thread_id.clone())
    );
    assert_eq!(
        history["session"]["thread_id"],
        Value::String(thread_id.clone())
    );
    assert!(
        history["message_stats"]["total_messages_in_thread"]
            .as_u64()
            .unwrap_or(0)
            >= 2
    );
    assert!(
        history["message_stats"]["total_messages_in_session"]
            .as_u64()
            .unwrap_or(0)
            >= 2
    );
    assert_eq!(history["message_stats"]["returned_messages"], 2);

    {
        let calls = provider.calls.lock().expect("calls mutex poisoned");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].message, "hello api");
        assert_eq!(
            calls[0].metadata.get("channel").and_then(Value::as_str),
            Some("api")
        );
        assert_eq!(
            calls[0].metadata.get("account_id").and_then(Value::as_str),
            Some("main")
        );
        assert_eq!(
            calls[0].metadata.get("from_id").and_then(Value::as_str),
            Some("api-e2e-user")
        );
        assert_eq!(calls[0].thread_id, thread_id);
    }

    shutdown_http_server(shutdown_tx, handle).await;
}

#[tokio::test]
async fn test_thread_lifecycle_real_http_api_e2e() {
    let provider = Arc::new(RecordingProvider::new());
    let state = make_state_with_recording_provider(provider).await;
    let (base_url, shutdown_tx, handle) = spawn_http_server(state).await;
    let client = reqwest::Client::new();
    let mut ws = connect_chat_ws(&base_url).await;

    let threads_before: Value = client
        .get(format!("{base_url}/api/threads"))
        .send()
        .await
        .expect("list threads before")
        .json()
        .await
        .expect("threads before json");
    assert_eq!(threads_before["count"], 0);

    let (auto_thread_id, auto_events) = run_chat_start(
        &mut ws,
        json!({
            "op": "start",
            "message": "hello thread lifecycle",
            "accountId": "main",
            "fromId": "api-e2e-user",
            "workspacePath": "/tmp/api-thread-lifecycle"
        }),
    )
    .await;
    assert!(auto_thread_id.starts_with("thread::"));
    assert!(
        auto_events
            .iter()
            .any(|event| event["type"] == "assistant_delta"
                && event["delta"]
                    == Value::String("provider-e2e: hello thread lifecycle".to_owned()))
    );

    let threads_after_chat: Value = client
        .get(format!("{base_url}/api/threads"))
        .send()
        .await
        .expect("list threads after chat")
        .json()
        .await
        .expect("threads after chat json");
    assert_eq!(threads_after_chat["count"], 1);
    assert_eq!(
        threads_after_chat["threads"][0]["thread_id"],
        Value::String(auto_thread_id.clone())
    );
    assert_eq!(
        threads_after_chat["threads"][0]["workspace_dir"],
        Value::String("/tmp/api-thread-lifecycle".to_owned())
    );

    let auto_history_ready = wait_until(std::time::Duration::from_secs(5), || {
        let client = client.clone();
        let base_url = base_url.clone();
        let auto_thread_id = auto_thread_id.clone();
        async move {
            let history = fetch_thread_history(&client, &base_url, &auto_thread_id).await;
            history["message_stats"]["returned_messages"]
                .as_u64()
                .unwrap_or(0)
                >= 2
        }
    })
    .await;
    assert!(
        auto_history_ready,
        "auto-created thread history was not persisted"
    );

    let auto_history = fetch_thread_history(&client, &base_url, &auto_thread_id).await;
    assert_eq!(auto_history["ok"], true);
    assert_eq!(auto_history["message_stats"]["returned_messages"], 2);
    assert_eq!(
        auto_history["thread"]["workspace_dir"],
        Value::String("/tmp/api-thread-lifecycle".to_owned())
    );
    assert_eq!(
        auto_history["session"]["workspace_dir"],
        Value::String("/tmp/api-thread-lifecycle".to_owned())
    );

    ws.send(Message::Text(
        json!({
            "op": "start",
            "message": "should fail",
            "accountId": "main",
            "threadId": "main::main::legacy-user"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send legacy start");
    let legacy_payload = recv_ws_json(&mut ws).await;
    assert_eq!(legacy_payload["type"], "error");
    assert_eq!(
        legacy_payload["error"],
        Value::String("threadId must be a canonical thread id".to_owned())
    );

    let created_thread: Value = client
        .post(format!("{base_url}/api/threads"))
        .json(&json!({
            "label": "Manual Thread"
        }))
        .send()
        .await
        .expect("create thread response")
        .json()
        .await
        .expect("create thread json");
    let manual_thread_id = created_thread["thread_id"]
        .as_str()
        .expect("manual thread id")
        .to_owned();
    assert!(manual_thread_id.starts_with("thread::"));
    assert_eq!(
        created_thread["label"],
        Value::String("Manual Thread".to_owned())
    );

    let updated_thread: Value = client
        .patch(format!("{base_url}/api/threads/{manual_thread_id}"))
        .json(&json!({
            "label": "Renamed Thread",
            "workspaceDir": "/tmp/manual-thread"
        }))
        .send()
        .await
        .expect("update thread response")
        .json()
        .await
        .expect("update thread json");
    assert_eq!(
        updated_thread["label"],
        Value::String("Renamed Thread".to_owned())
    );
    assert_eq!(
        updated_thread["workspace_dir"],
        Value::String("/tmp/manual-thread".to_owned())
    );

    let fetched_thread: Value = client
        .get(format!("{base_url}/api/threads/{manual_thread_id}"))
        .send()
        .await
        .expect("get thread response")
        .json()
        .await
        .expect("get thread json");
    assert_eq!(
        fetched_thread["label"],
        Value::String("Renamed Thread".to_owned())
    );
    assert_eq!(
        fetched_thread["workspace_dir"],
        Value::String("/tmp/manual-thread".to_owned())
    );

    let delete_response: Value = client
        .delete(format!("{base_url}/api/threads/{manual_thread_id}"))
        .send()
        .await
        .expect("delete thread response")
        .json()
        .await
        .expect("delete thread json");
    assert_eq!(delete_response["deleted"], true);
    assert_eq!(
        delete_response["thread_id"],
        Value::String(manual_thread_id.clone())
    );

    let threads_after_delete: Value = client
        .get(format!("{base_url}/api/threads"))
        .send()
        .await
        .expect("list threads after delete")
        .json()
        .await
        .expect("threads after delete json");
    assert_eq!(threads_after_delete["count"], 1);
    assert_eq!(
        threads_after_delete["threads"][0]["thread_id"],
        Value::String(auto_thread_id)
    );

    shutdown_http_server(shutdown_tx, handle).await;
}

#[tokio::test]
async fn test_automation_flow_real_http_api_e2e() {
    let provider = Arc::new(RecordingProvider::new());
    let state = make_state_with_recording_provider(provider.clone()).await;
    let (base_url, shutdown_tx, handle) = spawn_http_server(state).await;
    let client = reqwest::Client::new();

    let created: Value = client
        .post(format!("{base_url}/api/automations"))
        .json(&json!({
            "label": "Daily Repo Check",
            "prompt": "Summarize code health issues",
            "workspaceDir": "/tmp/automation-workspace",
            "schedule": {
                "kind": "daily",
                "time": "09:00",
                "weekdays": ["mo", "we", "fr"],
                "timezone": "Asia/Shanghai"
            }
        }))
        .send()
        .await
        .expect("create automation response")
        .json()
        .await
        .expect("create automation json");
    let automation_id = created["id"].as_str().expect("automation id").to_owned();
    assert!(automation_id.starts_with("automation::"));
    assert!(created["threadId"].is_null());
    assert_eq!(created["workspaceDir"], "/tmp/automation-workspace");

    let list: Value = client
        .get(format!("{base_url}/api/automations"))
        .send()
        .await
        .expect("list automations response")
        .json()
        .await
        .expect("list automations json");
    assert_eq!(list["automations"].as_array().unwrap().len(), 1);

    let thread_list: Value = client
        .get(format!("{base_url}/api/threads"))
        .send()
        .await
        .expect("list threads response")
        .json()
        .await
        .expect("list threads json");
    assert_eq!(thread_list["count"], 0);

    let run_now: Value = client
        .post(format!(
            "{base_url}/api/automations/{automation_id}/run-now"
        ))
        .send()
        .await
        .expect("run now response")
        .json()
        .await
        .expect("run now json");
    let first_thread_id = run_now["threadId"]
        .as_str()
        .expect("first run thread id")
        .to_owned();
    assert!(first_thread_id.starts_with("thread::"));
    assert!(!run_now["runId"].as_str().unwrap_or_default().is_empty());

    let activity_ready = wait_until(std::time::Duration::from_secs(5), || {
        let client = client.clone();
        let base_url = base_url.clone();
        let automation_id = automation_id.clone();
        async move {
            let activity: Value = match client
                .get(format!(
                    "{base_url}/api/automations/{automation_id}/activity"
                ))
                .send()
                .await
            {
                Ok(resp) => match resp.json().await {
                    Ok(json) => json,
                    Err(_) => return false,
                },
                Err(_) => return false,
            };
            activity["items"]
                .as_array()
                .is_some_and(|items| !items.is_empty() && items[0]["excerpt"].is_string())
        }
    })
    .await;
    if !activity_ready {
        let failed_activity: Value = client
            .get(format!(
                "{base_url}/api/automations/{automation_id}/activity"
            ))
            .send()
            .await
            .expect("failed activity response")
            .json()
            .await
            .expect("failed activity json");
        let failed_history = fetch_thread_history(&client, &base_url, &first_thread_id).await;
        let failed_calls = provider.calls.lock().expect("calls mutex poisoned").clone();
        panic!(
            "automation activity did not capture excerpt\nactivity={failed_activity}\nhistory={failed_history}\nprovider_calls={failed_calls:#?}"
        );
    }

    let activity: Value = client
        .get(format!(
            "{base_url}/api/automations/{automation_id}/activity"
        ))
        .send()
        .await
        .expect("activity response")
        .json()
        .await
        .expect("activity json");
    assert_eq!(activity["count"], 1);
    assert_eq!(
        activity["items"][0]["threadId"],
        Value::String(first_thread_id.clone())
    );
    assert_eq!(
        activity["items"][0]["excerpt"],
        Value::String("provider-e2e: Summarize code health issues".to_owned())
    );

    let history_ready = wait_until(std::time::Duration::from_secs(5), || {
        let client = client.clone();
        let base_url = base_url.clone();
        let thread_id = first_thread_id.clone();
        async move {
            let history = fetch_thread_history(&client, &base_url, &thread_id).await;
            history["message_stats"]["returned_messages"]
                .as_u64()
                .unwrap_or(0)
                >= 2
        }
    })
    .await;
    assert!(history_ready, "automation thread history was not persisted");

    let history = fetch_thread_history(&client, &base_url, &first_thread_id).await;
    assert_eq!(history["ok"], true);
    assert_eq!(
        history["thread"]["thread_id"],
        Value::String(first_thread_id.clone())
    );
    assert_eq!(
        history["session"]["thread_id"],
        Value::String(first_thread_id.clone())
    );
    assert!(
        history["message_stats"]["total_messages_in_thread"]
            .as_u64()
            .unwrap_or(0)
            >= 2
    );
    assert!(
        history["message_stats"]["total_messages_in_session"]
            .as_u64()
            .unwrap_or(0)
            >= 2
    );

    let updated: Value = client
        .patch(format!("{base_url}/api/automations/{automation_id}"))
        .json(&json!({
            "label": "Repo Check Updated",
            "prompt": "Summarize release blockers",
            "workspaceDir": "/tmp/automation-updated",
            "enabled": true,
            "schedule": {
                "kind": "interval",
                "hours": 6
            }
        }))
        .send()
        .await
        .expect("update automation response")
        .json()
        .await
        .expect("update automation json");
    assert_eq!(updated["label"], "Repo Check Updated");
    assert_eq!(updated["workspaceDir"], "/tmp/automation-updated");
    assert_eq!(updated["enabled"], true);
    assert_eq!(updated["schedule"]["kind"], "interval");
    assert_eq!(updated["threadId"], Value::String(first_thread_id.clone()));

    let updated_thread_list: Value = client
        .get(format!("{base_url}/api/threads"))
        .send()
        .await
        .expect("list updated threads response")
        .json()
        .await
        .expect("list updated threads json");
    assert_eq!(updated_thread_list["count"], 1);
    assert_eq!(
        updated_thread_list["threads"][0]["thread_id"],
        Value::String(first_thread_id.clone())
    );
    assert_eq!(
        updated_thread_list["threads"][0]["label"],
        Value::String("Daily Repo Check".to_owned())
    );
    assert_eq!(
        updated_thread_list["threads"][0]["workspace_dir"],
        Value::String("/tmp/automation-workspace".to_owned())
    );

    let rerun: Value = client
        .post(format!(
            "{base_url}/api/automations/{automation_id}/run-now"
        ))
        .send()
        .await
        .expect("rerun automation response")
        .json()
        .await
        .expect("rerun automation json");
    let second_thread_id = rerun["threadId"]
        .as_str()
        .expect("second run thread id")
        .to_owned();
    assert!(second_thread_id.starts_with("thread::"));
    assert_ne!(second_thread_id, first_thread_id);

    let rerun_ready = wait_until(std::time::Duration::from_secs(5), || {
        let provider = provider.clone();
        async move { provider.calls.lock().expect("calls mutex poisoned").len() >= 2 }
    })
    .await;
    assert!(rerun_ready, "updated automation run did not reach provider");

    {
        let calls = provider.calls.lock().expect("calls mutex poisoned");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].thread_id, first_thread_id);
        assert_eq!(
            calls[0].workspace_dir.as_deref(),
            Some("/tmp/automation-workspace")
        );
        assert_eq!(
            calls[0]
                .metadata
                .get("automation_id")
                .and_then(Value::as_str),
            Some(automation_id.as_str())
        );
        assert_eq!(
            calls[1].workspace_dir.as_deref(),
            Some("/tmp/automation-updated")
        );
        assert_eq!(calls[1].thread_id, second_thread_id);
    }

    let rerun_activity: Value = client
        .get(format!(
            "{base_url}/api/automations/{automation_id}/activity"
        ))
        .send()
        .await
        .expect("rerun activity response")
        .json()
        .await
        .expect("rerun activity json");
    assert_eq!(rerun_activity["count"], 2);
    assert_eq!(
        rerun_activity["threadId"],
        Value::String(
            rerun["threadId"]
                .as_str()
                .expect("rerun thread id payload")
                .to_owned()
        )
    );
    assert_eq!(
        rerun_activity["items"][0]["threadId"],
        rerun["threadId"].clone()
    );
    assert_eq!(
        rerun_activity["items"][1]["threadId"],
        run_now["threadId"].clone()
    );

    let rerun_thread_list: Value = client
        .get(format!("{base_url}/api/threads"))
        .send()
        .await
        .expect("list rerun threads response")
        .json()
        .await
        .expect("list rerun threads json");
    assert_eq!(rerun_thread_list["count"], 2);
    assert!(
        rerun_thread_list["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["thread_id"] == rerun["threadId"])
    );
    assert!(
        rerun_thread_list["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["thread_id"] == run_now["threadId"])
    );

    let deleted: Value = client
        .delete(format!("{base_url}/api/automations/{automation_id}"))
        .send()
        .await
        .expect("delete automation response")
        .json()
        .await
        .expect("delete automation json");
    assert_eq!(deleted["deleted"], true);

    let list_after_delete: Value = client
        .get(format!("{base_url}/api/automations"))
        .send()
        .await
        .expect("list after delete response")
        .json()
        .await
        .expect("list after delete json");
    assert!(
        list_after_delete["automations"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let threads_after_delete: Value = client
        .get(format!("{base_url}/api/threads"))
        .send()
        .await
        .expect("sessions after delete response")
        .json()
        .await
        .expect("sessions after delete json");
    assert_eq!(threads_after_delete["count"], 2);

    shutdown_http_server(shutdown_tx, handle).await;
}

#[tokio::test]
async fn test_one_time_automation_real_http_api_e2e() {
    let provider = Arc::new(RecordingProvider::new());
    let state = make_state_with_recording_provider(provider).await;
    let (base_url, shutdown_tx, handle) = spawn_http_server(state).await;
    let client = reqwest::Client::new();

    let created: Value = client
        .post(format!("{base_url}/api/automations"))
        .json(&json!({
            "label": "One-time Repo Check",
            "prompt": "Check the repo once for release blockers",
            "workspaceDir": "/tmp/automation-once",
            "schedule": {
                "kind": "once",
                "at": "2030-05-01T08:30"
            }
        }))
        .send()
        .await
        .expect("create one-time automation response")
        .json()
        .await
        .expect("create one-time automation json");
    let automation_id = created["id"].as_str().expect("automation id").to_owned();
    assert_eq!(created["schedule"]["kind"], "once");
    assert_eq!(created["schedule"]["at"], "2030-05-01T08:30");

    let listed: Value = client
        .get(format!("{base_url}/api/automations"))
        .send()
        .await
        .expect("list one-time automations response")
        .json()
        .await
        .expect("list one-time automations json");
    assert_eq!(listed["automations"].as_array().unwrap().len(), 1);
    assert_eq!(listed["automations"][0]["schedule"]["kind"], "once");

    let run_now: Value = client
        .post(format!(
            "{base_url}/api/automations/{automation_id}/run-now"
        ))
        .send()
        .await
        .expect("run one-time automation response")
        .json()
        .await
        .expect("run one-time automation json");
    assert!(run_now["threadId"].as_str().is_some());

    let fetched: Value = client
        .get(format!("{base_url}/api/automations/{automation_id}"))
        .send()
        .await
        .expect("get one-time automation response")
        .json()
        .await
        .expect("get one-time automation json");
    assert_eq!(fetched["schedule"]["kind"], "once");
    assert_eq!(fetched["schedule"]["at"], "2030-05-01T08:30");
    assert_eq!(fetched["enabled"], false);

    shutdown_http_server(shutdown_tx, handle).await;
}
