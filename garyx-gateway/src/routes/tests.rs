use super::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::Body;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_models::config::{ApiAccount, GaryxConfig};
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent};
use garyx_models::thread_logs::{ThreadLogEvent, ThreadLogSink};
use garyx_router::MessageRouter;
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;

use crate::route_graph::build_router;
use crate::server::AppStateBuilder;
use crate::thread_logs::ThreadFileLogger;

fn test_config() -> GaryxConfig {
    crate::test_support::with_gateway_auth(GaryxConfig::default())
}

fn authed_request() -> axum::http::request::Builder {
    crate::test_support::authed_request()
}

struct SlowDeleteProvider {
    ready: AtomicBool,
    delay_ms: u64,
    clear_succeeds: bool,
    cleared_sessions: std::sync::Mutex<Vec<String>>,
}

impl SlowDeleteProvider {
    fn new(delay_ms: u64) -> Self {
        Self::with_clear_result(delay_ms, true)
    }

    fn with_clear_result(delay_ms: u64, clear_succeeds: bool) -> Self {
        Self {
            ready: AtomicBool::new(true),
            delay_ms,
            clear_succeeds,
            cleared_sessions: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn cleared_sessions(&self) -> Vec<String> {
        self.cleared_sessions.lock().unwrap().clone()
    }
}

#[test]
fn endpoint_conversation_details_marks_feishu_group_with_group_name() {
    let endpoint = garyx_router::KnownChannelEndpoint {
        endpoint_key: "feishu::main::oc_group::oc_group".to_owned(),
        channel: "feishu".to_owned(),
        account_id: "main".to_owned(),
        binding_key: "oc_group".to_owned(),
        chat_id: "oc_group".to_owned(),
        delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
        delivery_target_id: "oc_group".to_owned(),
        display_label: "garyx".to_owned(),
        thread_id: Some("thread::group".to_owned()),
        thread_label: Some("garyx".to_owned()),
        workspace_dir: None,
        thread_updated_at: None,
        last_inbound_at: None,
        last_delivery_at: None,
    };

    let details = endpoint_conversation_details(
        &endpoint,
        Some(&FeishuChatSummary {
            name: Some("bot 测试".to_owned()),
            chat_mode: Some("group".to_owned()),
            chat_type: Some("private".to_owned()),
        }),
    );

    assert_eq!(details.kind, "group");
    assert_eq!(details.label, "bot 测试");
}

#[test]
fn endpoint_conversation_details_marks_feishu_topic_with_group_name() {
    let endpoint = garyx_router::KnownChannelEndpoint {
        endpoint_key: "feishu::main::ou_user::om_topic".to_owned(),
        channel: "feishu".to_owned(),
        account_id: "main".to_owned(),
        binding_key: "om_topic".to_owned(),
        chat_id: "oc_group".to_owned(),
        delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
        delivery_target_id: "oc_group".to_owned(),
        display_label: "garyx".to_owned(),
        thread_id: Some("thread::topic".to_owned()),
        thread_label: Some("garyx".to_owned()),
        workspace_dir: None,
        thread_updated_at: None,
        last_inbound_at: None,
        last_delivery_at: None,
    };

    let details = endpoint_conversation_details(
        &endpoint,
        Some(&FeishuChatSummary {
            name: Some("bot 测试".to_owned()),
            chat_mode: Some("group".to_owned()),
            chat_type: Some("private".to_owned()),
        }),
    );

    assert_eq!(details.kind, "topic");
    assert_eq!(details.label, "bot 测试");
}

#[test]
fn endpoint_conversation_details_keeps_feishu_private_as_private() {
    let endpoint = garyx_router::KnownChannelEndpoint {
        endpoint_key: "feishu::main::ou_user".to_owned(),
        channel: "feishu".to_owned(),
        account_id: "main".to_owned(),
        binding_key: "ou_user".to_owned(),
        chat_id: "oc_private".to_owned(),
        delivery_target_type: DELIVERY_TARGET_TYPE_OPEN_ID.to_owned(),
        delivery_target_id: "ou_user".to_owned(),
        display_label: "garyx".to_owned(),
        thread_id: Some("thread::private".to_owned()),
        thread_label: Some("garyx".to_owned()),
        workspace_dir: None,
        thread_updated_at: None,
        last_inbound_at: None,
        last_delivery_at: None,
    };

    let details = endpoint_conversation_details(
        &endpoint,
        Some(&FeishuChatSummary {
            name: None,
            chat_mode: Some("p2p".to_owned()),
            chat_type: Some("p2p".to_owned()),
        }),
    );

    assert_eq!(details.kind, "private");
    assert_eq!(details.label, "garyx");
}

#[async_trait::async_trait]
impl AgentLoopProvider for SlowDeleteProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
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
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        on_chunk(StreamEvent::Delta {
            text: "slow-delete".to_owned(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "slow-delete-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "slow-delete".to_owned(),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            success: true,
            error: None,
            input_tokens: 1,
            output_tokens: 1,
            cost: 0.0,
            duration_ms: self.delay_ms as i64,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }

    async fn clear_session(&self, session_key: &str) -> bool {
        self.cleared_sessions
            .lock()
            .unwrap()
            .push(session_key.to_owned());
        self.clear_succeeds
    }
}

async fn test_state() -> (Arc<AppState>, Arc<ThreadFileLogger>, TempDir) {
    let dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(dir.path()));
    let state = AppStateBuilder::new(test_config())
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .with_agent_team_store(Arc::new(crate::agent_teams::AgentTeamStore::new()))
        .with_thread_log_sink(logger.clone())
        .build();
    (state, logger, dir)
}

#[tokio::test]
async fn thread_summary_uses_transcript_when_snapshot_cache_is_empty() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::summary-transcript";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "messages": [],
                "message_count": 2,
                "history": {
                    "source": "transcript_v1",
                    "message_count": 2
                }
            }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .rewrite_from_messages(
            thread_id,
            &[
                json!({"role": "user", "content": "hello from transcript"}),
                json!({"role": "assistant", "content": "reply from transcript"}),
            ],
        )
        .await
        .unwrap();

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let summary = thread_summary_with_history(&state, thread_id, &data).await;
    assert_eq!(summary["last_user_message"], "hello from transcript");
    assert_eq!(summary["last_assistant_message"], "reply from transcript");
}

#[tokio::test]
async fn thread_logs_route_returns_full_and_delta_chunks() {
    let (state, logger, _dir) = test_state().await;
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Logs".to_owned()),
            workspace_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();
    logger
        .record_event(ThreadLogEvent::info(&thread_id, "run", "hello"))
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .uri(format!("/api/threads/{thread_id}/logs"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let cursor = payload["cursor"].as_u64().unwrap();
    assert_eq!(payload["reset"], true);
    assert!(payload["text"].as_str().unwrap().contains("hello"));

    logger
        .record_event(ThreadLogEvent::info(&thread_id, "run", "world"))
        .await;
    let request = authed_request()
        .uri(format!("/api/threads/{thread_id}/logs?cursor={cursor}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["reset"], false);
    assert!(payload["text"].as_str().unwrap().contains("world"));
}

#[tokio::test]
async fn thread_logs_route_alias_returns_full_chunk() {
    let (state, logger, _dir) = test_state().await;
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Logs".to_owned()),
            workspace_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();
    logger
        .record_event(ThreadLogEvent::info(&thread_id, "run", "hello"))
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .uri(format!("/api/threads/{thread_id}/logs"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["reset"], true);
    assert!(payload["text"].as_str().unwrap().contains("hello"));
}

#[tokio::test]
async fn create_thread_seeds_sdk_session_id() {
    let (state, _logger, _dir) = test_state().await;
    let workspace = tempdir().unwrap();
    let workspace_dir = workspace.path().to_string_lossy().to_string();
    let session_id = format!("claude-session-{}", uuid::Uuid::new_v4());
    let (thread_id, data, resolved) = create_thread_for_agent_reference(
        state.threads.thread_store.clone(),
        state.integration.bridge.clone(),
        state.ops.custom_agents.clone(),
        state.ops.agent_teams.clone(),
        ThreadEnsureOptions {
            label: Some("Resume Claude".to_owned()),
            workspace_dir: Some(workspace_dir),
            agent_id: Some("claude".to_owned()),
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: Some(session_id.clone()),
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .expect("thread created");
    let stored = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("stored thread");
    assert_eq!(resolved.provider_type(), ProviderType::ClaudeCode);
    assert_eq!(data["sdk_session_id"], session_id);
    assert_eq!(stored["provider_type"], "claude_code");
    assert_eq!(stored["sdk_session_id"], session_id);
}

#[tokio::test]
async fn create_thread_rejects_unknown_sdk_session_id() {
    let (state, _logger, _dir) = test_state().await;
    let router = build_router(state);
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "sdkSessionId": "missing-local-provider-session-for-gateway-test"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_thread_rejects_invalid_sdk_session_provider_hint() {
    let (state, _logger, _dir) = test_state().await;
    let router = build_router(state);
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "sdkSessionId": "missing-local-provider-session-for-gateway-test",
                "sdkSessionProviderHint": "wat"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Unsupported sdkSessionProviderHint")
    );
}

#[tokio::test]
async fn create_thread_rejects_unknown_sdk_session_id_for_requested_provider() {
    let (state, _logger, _dir) = test_state().await;
    let router = build_router(state);
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "sdkSessionId": "missing-local-provider-session-for-gateway-test",
                "sdkSessionProviderHint": "codex"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("No local Codex session was found")
    );
}

#[tokio::test]
async fn seed_imported_thread_history_persists_transcript_and_thread_state() {
    let (state, _logger, _dir) = test_state().await;
    let workspace = tempdir().unwrap();
    let workspace_dir = workspace.path().to_string_lossy().to_string();
    let (thread_id, mut data, _resolved) = create_thread_for_agent_reference(
        state.threads.thread_store.clone(),
        state.integration.bridge.clone(),
        state.ops.custom_agents.clone(),
        state.ops.agent_teams.clone(),
        ThreadEnsureOptions {
            label: Some("Recovered Session".to_owned()),
            workspace_dir: Some(workspace_dir),
            agent_id: Some("claude".to_owned()),
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: Some("recovered-session".to_owned()),
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .expect("thread created");

    let imported_messages = vec![
        json!({
            "role": "user",
            "content": "hello",
            "timestamp": "2026-04-14T00:00:00Z"
        }),
        json!({
            "role": "assistant",
            "content": "world",
            "timestamp": "2026-04-14T00:00:01Z"
        }),
    ];

    seed_imported_thread_history(&state, &thread_id, &mut data, &imported_messages)
        .await
        .expect("seed imported history");

    let stored = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("stored thread");
    assert_eq!(stored["history"]["message_count"], 2);
    assert_eq!(stored["message_count"], 2);
    assert_eq!(
        stored["messages"].as_array().expect("messages array").len(),
        2
    );

    let snapshot = state
        .threads
        .history
        .thread_snapshot(&thread_id, 10)
        .await
        .expect("snapshot");
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[0]["content"], "hello");
    assert_eq!(combined[1]["content"], "world");
}

#[tokio::test]
async fn create_thread_rejects_unknown_agent_id() {
    let (state, _logger, _dir) = test_state().await;
    let router = build_router(state);
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "label": "Bad thread",
                "agentId": "definitely-not-real"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_thread_removes_thread_log_file() {
    let (state, logger, _dir) = test_state().await;
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Delete".to_owned()),
            workspace_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();
    logger
        .record_event(ThreadLogEvent::info(&thread_id, "run", "to-delete"))
        .await;
    let log_path = logger.thread_log_path(&thread_id);
    assert!(log_path.exists());

    let router = build_router(state);
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!log_path.exists());
}

#[tokio::test]
async fn delete_thread_rejects_enabled_channel_binding() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&TelegramAccount {
                token: "token-main".to_owned(),
                enabled: true,
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                groups: std::collections::HashMap::new(),
            }),
        );

    let state = AppStateBuilder::new(config).build();
    let thread_id = "thread::delete-bound-enabled";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Bound Enabled",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "u1",
                    "chat_id": "u1",
                    "delivery_target_type": DELIVERY_TARGET_TYPE_CHAT_ID,
                    "delivery_target_id": "u1",
                    "display_label": "u1"
                }]
            }),
        )
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["error"],
        "cannot delete thread with active channel bindings"
    );
    assert!(state.threads.thread_store.get(thread_id).await.is_some());
}

#[tokio::test]
async fn delete_thread_allows_disabled_channel_binding() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&TelegramAccount {
                token: "token-main".to_owned(),
                enabled: false,
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                groups: std::collections::HashMap::new(),
            }),
        );

    let state = AppStateBuilder::new(config).build();
    let thread_id = "thread::delete-bound-disabled";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Bound Disabled",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "u1",
                    "chat_id": "u1",
                    "delivery_target_type": DELIVERY_TARGET_TYPE_CHAT_ID,
                    "delivery_target_id": "u1",
                    "display_label": "u1"
                }]
            }),
        )
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.threads.thread_store.get(thread_id).await.is_none());
}

#[tokio::test]
async fn delete_thread_allows_orphan_channel_binding() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::delete-bound-orphan";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Bound Orphan",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "u1",
                    "chat_id": "u1",
                    "delivery_target_type": DELIVERY_TARGET_TYPE_CHAT_ID,
                    "delivery_target_id": "u1",
                    "display_label": "u1"
                }]
            }),
        )
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.threads.thread_store.get(thread_id).await.is_none());
}

#[tokio::test]
async fn delete_thread_aborts_active_run_and_prevents_recreation() {
    let mut config = test_config();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
        },
    );

    let provider = Arc::new(SlowDeleteProvider::new(250));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("api-test-provider", provider.clone())
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

    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Delete Active".to_owned()),
            workspace_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();

    bridge
        .start_agent_run(
            garyx_models::provider::AgentRunRequest::new(
                &thread_id,
                "delete me",
                "run-delete-session",
                "api",
                "main",
                HashMap::new(),
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(bridge.is_run_active("run-delete-session").await);

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    assert!(!bridge.is_run_active("run-delete-session").await);
    assert!(state.threads.thread_store.get(&thread_id).await.is_none());
    assert_eq!(provider.cleared_sessions(), vec![thread_id]);
}

#[tokio::test]
async fn delete_thread_drops_local_state_even_when_provider_clear_fails() {
    let mut config = test_config();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
        },
    );

    let failing_provider = Arc::new(SlowDeleteProvider::with_clear_result(0, false));
    let default_provider = Arc::new(SlowDeleteProvider::with_clear_result(0, true));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("api-test-provider", failing_provider.clone())
        .await;
    bridge
        .register_provider("api-default-provider", default_provider)
        .await;
    bridge
        .set_route("api", "main", "api-default-provider")
        .await;
    bridge
        .set_default_provider_key("api-default-provider")
        .await;

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .build();
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;

    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Delete Local State".to_owned()),
            workspace_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();

    bridge
        .set_thread_affinity(&thread_id, "api-test-provider")
        .await;
    bridge
        .set_thread_workspace_binding(&thread_id, Some("/tmp/delete-thread".to_owned()))
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(state.threads.thread_store.get(&thread_id).await.is_none());
    assert_eq!(failing_provider.cleared_sessions(), vec![thread_id.clone()]);
    assert_eq!(
        bridge
            .resolve_provider_for_thread(&thread_id, "api", "main")
            .await,
        Some("api-default-provider".to_owned())
    );
    assert!(
        !bridge
            .thread_workspace_bindings_snapshot()
            .await
            .contains_key(&thread_id)
    );
}

#[tokio::test]
async fn delete_thread_clears_in_memory_reply_routing() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::reply-delete";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            serde_json::json!({
                "thread_id": thread_id,
                "thread_id": thread_id,
                "label": "Reply Delete",
                "outbound_message_ids": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "chat_id": "42",
                    "message_id": "msg-delete-1"
                }]
            }),
        )
        .await;
    {
        let mut router = state.threads.router.lock().await;
        router
            .message_routing_index_mut()
            .rebuild_from_store(state.threads.thread_store.as_ref(), "telegram")
            .await;
        assert_eq!(
            router.resolve_reply_thread_for_chat(
                "telegram",
                "main",
                Some("42"),
                None,
                "msg-delete-1",
            ),
            Some(thread_id)
        );
    }

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let router = state.threads.router.lock().await;
    assert_eq!(
        router.resolve_reply_thread_for_chat("telegram", "main", Some("42"), None, "msg-delete-1",),
        None
    );
}

#[tokio::test]
async fn delete_thread_clears_in_memory_last_delivery() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::delivery-delete";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            serde_json::json!({
                "thread_id": thread_id,
                "thread_id": thread_id,
                "label": "Delivery Delete"
            }),
        )
        .await;
    {
        let mut router = state.threads.router.lock().await;
        router.set_last_delivery(
            thread_id,
            garyx_models::routing::DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: Default::default(),
            },
        );
        assert!(router.get_last_delivery(thread_id).is_some());
    }

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let router = state.threads.router.lock().await;
    assert!(router.get_last_delivery(thread_id).is_none());
    assert!(
        router
            .resolve_delivery_target(&format!("thread:{thread_id}"))
            .is_none()
    );
}

#[tokio::test]
async fn delete_thread_clears_switched_thread_references() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::switch-delete";
    state
        .threads
        .thread_store
        .set(
            "thread::older",
            serde_json::json!({
                "thread_id": "thread::older",
                "thread_id": "thread::older",
                "label": "Older"
            }),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            thread_id,
            serde_json::json!({
                "thread_id": thread_id,
                "thread_id": thread_id,
                "label": "Switch Delete"
            }),
        )
        .await;
    {
        let mut router = state.threads.router.lock().await;
        let user_key = MessageRouter::build_account_user_key("telegram", "main", "u1", false, None);
        router.switch_to_thread(&user_key, "thread::older");
        router.switch_to_thread(&user_key, thread_id);
        assert_eq!(
            router.get_current_thread_id_for_account("telegram", "main", "u1", false, None),
            Some(thread_id)
        );
    }

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let router = state.threads.router.lock().await;
    assert_eq!(
        router.get_current_thread_id_for_account("telegram", "main", "u1", false, None),
        Some("thread::older")
    );
}

#[tokio::test]
async fn configured_bots_route_returns_only_account_workspace_bindings() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "bound".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token-a".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/bound-workspace".to_owned()),
                    owner_target: None,
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "unbound".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token-b".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: None,
                    owner_target: None,
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    // Generic plugin-owned subprocess channel — same `bots` route
    // must surface entries from `channels.plugins[id].accounts`.
    let mut plugin_cfg = garyx_models::config::PluginChannelConfig::default();
    plugin_cfg.accounts.insert(
        "main".to_owned(),
        garyx_models::config::PluginAccountEntry {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: Some("/tmp/plugin-workspace".to_owned()),
            config: serde_json::json!({
                "token": "plugin_agent_test",
                "base_url": "https://example.com",
            }),
        },
    );
    config
        .channels
        .plugins
        .insert("sample_plugin".to_owned(), plugin_cfg);

    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();
    let router = build_router(state);

    let request = authed_request()
        .uri("/api/configured-bots")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();

    let bound = bots
        .iter()
        .find(|entry| entry["account_id"] == "bound")
        .unwrap();
    let unbound = bots
        .iter()
        .find(|entry| entry["account_id"] == "unbound")
        .unwrap();
    let plugin_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "sample_plugin" && entry["account_id"] == "main")
        .unwrap();

    assert_eq!(bound["workspace_dir"], "/tmp/bound-workspace");
    assert!(unbound["workspace_dir"].is_null());
    assert_eq!(plugin_bot["workspace_dir"], "/tmp/plugin-workspace");
    assert_eq!(bound["main_endpoint_status"], "unresolved");
    assert_eq!(unbound["main_endpoint_status"], "unresolved");
    assert_eq!(plugin_bot["main_endpoint_status"], "unresolved");
    assert!(bound["default_open_endpoint"].is_null());
    assert!(plugin_bot["default_open_endpoint"].is_null());
}

#[tokio::test]
async fn configured_bots_route_exposes_resolved_main_endpoints() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "telegram_owner".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token-telegram".to_owned(),
                    enabled: true,
                    name: Some("Telegram Owner".to_owned()),
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/telegram-owner".to_owned()),
                    owner_target: Some(garyx_models::config::OwnerTargetConfig {
                        target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
                        target_id: "8592453520".to_owned(),
                    }),
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    config
        .channels
        .plugin_channel_mut("feishu")
        .accounts
        .insert(
            "feishu_owner".to_owned(),
            garyx_models::config::feishu_account_to_plugin_entry(
                &garyx_models::config::FeishuAccount {
                    app_id: "cli_test_app".to_owned(),
                    app_secret: "cli_test_secret".to_owned(),
                    enabled: true,
                    domain: garyx_models::config::FeishuDomain::Feishu,
                    name: Some("Feishu Owner".to_owned()),
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/feishu-owner".to_owned()),
                    owner_target: Some(garyx_models::config::OwnerTargetConfig {
                        target_type: DELIVERY_TARGET_TYPE_OPEN_ID.to_owned(),
                        target_id: "ou_owner_123".to_owned(),
                    }),
                    require_mention: true,
                    topic_session_mode: garyx_models::config::TopicSessionMode::Disabled,
                },
            ),
        );
    config
        .channels
        .plugin_channel_mut("weixin")
        .accounts
        .insert(
            "wechat_owner".to_owned(),
            garyx_models::config::weixin_account_to_plugin_entry(
                &garyx_models::config::WeixinAccount {
                    token: "token-wechat".to_owned(),
                    uin: String::new(),
                    enabled: true,
                    base_url: "https://ilinkai.weixin.qq.com".to_owned(),
                    name: Some("Wechat".to_owned()),
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/wechat-owner".to_owned()),
                },
            ),
        );
    let mut sample_plugin = garyx_models::config::PluginChannelConfig::default();
    sample_plugin.accounts.insert(
        "plugin_owner".to_owned(),
        garyx_models::config::PluginAccountEntry {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: Some("/tmp/plugin-owner".to_owned()),
            config: serde_json::json!({
                "token": "plugin_agent_owner",
                "base_url": "https://plugin.example.com",
            }),
        },
    );
    config
        .channels
        .plugins
        .insert("sample_plugin".to_owned(), sample_plugin);

    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();
    let router = build_router(state);

    let request = authed_request()
        .uri("/api/configured-bots")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();

    let telegram_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "telegram" && entry["account_id"] == "telegram_owner")
        .unwrap();
    assert_eq!(telegram_bot["main_endpoint_status"], "resolved");
    assert_eq!(telegram_bot["display_name"], "Telegram Owner");
    assert_eq!(telegram_bot["main_endpoint"]["source"], "owner_target");
    assert_eq!(
        telegram_bot["default_open_endpoint"]["delivery_target_id"],
        "8592453520"
    );
    assert_eq!(
        telegram_bot["main_endpoint"]["delivery_target_type"],
        DELIVERY_TARGET_TYPE_CHAT_ID
    );
    assert_eq!(
        telegram_bot["main_endpoint"]["delivery_target_id"],
        "8592453520"
    );
    assert_eq!(
        telegram_bot["main_endpoint"]["workspace_dir"],
        "/tmp/telegram-owner"
    );

    let feishu_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "feishu" && entry["account_id"] == "feishu_owner")
        .unwrap();
    assert_eq!(feishu_bot["main_endpoint_status"], "resolved");
    assert_eq!(feishu_bot["display_name"], "Feishu Owner");
    assert_eq!(feishu_bot["main_endpoint"]["source"], "owner_target");
    assert_eq!(
        feishu_bot["main_endpoint"]["delivery_target_type"],
        DELIVERY_TARGET_TYPE_OPEN_ID
    );
    assert_eq!(
        feishu_bot["main_endpoint"]["delivery_target_id"],
        "ou_owner_123"
    );
    assert_eq!(
        feishu_bot["main_endpoint"]["workspace_dir"],
        "/tmp/feishu-owner"
    );
    assert_eq!(
        feishu_bot["default_open_endpoint"]["delivery_target_id"],
        "ou_owner_123"
    );

    let weixin_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "weixin" && entry["account_id"] == "wechat_owner")
        .unwrap();
    assert_eq!(weixin_bot["display_name"], "Wechat");
    assert_eq!(weixin_bot["workspace_dir"], "/tmp/wechat-owner");
    assert_eq!(weixin_bot["main_endpoint_status"], "unresolved");
    assert!(weixin_bot["default_open_endpoint"].is_null());

    let plugin_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "sample_plugin" && entry["account_id"] == "plugin_owner")
        .unwrap();
    assert_eq!(plugin_bot["display_name"], "plugin_owner");
    assert_eq!(plugin_bot["workspace_dir"], "/tmp/plugin-owner");
    assert_eq!(plugin_bot["main_endpoint_status"], "unresolved");
    assert!(plugin_bot["default_open_endpoint"].is_null());
}

#[tokio::test]
async fn bot_consoles_route_aggregates_configured_bots_and_endpoints() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token-main".to_owned(),
                    enabled: true,
                    name: Some("Main Bot".to_owned()),
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/main-workspace".to_owned()),
                    owner_target: None,
                    groups: std::collections::HashMap::new(),
                },
            ),
        );

    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();

    state
        .threads
        .thread_store
        .set(
            "thread::support",
            serde_json::json!({
                "thread_id": "thread::support",
                "label": "Support",
                "workspace_dir": "/tmp/main-workspace",
                "updated_at": "2026-03-16T01:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice",
                    "last_inbound_at": "2026-03-16T01:00:00Z"
                }]
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/bot-consoles")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();
    let main = bots
        .iter()
        .find(|entry| entry["id"] == "telegram::main")
        .unwrap();

    assert_eq!(main["display_name"], "Main Bot");
    assert_eq!(main["workspace_dir"], "/tmp/main-workspace");
    assert_eq!(main["status"], "connected");
    assert_eq!(main["main_endpoint_status"], "resolved");
    assert_eq!(main["main_endpoint"]["thread_id"], "thread::support");
    assert_eq!(main["main_endpoint"]["delivery_target_type"], "chat_id");
    assert_eq!(main["main_endpoint"]["delivery_target_id"], "alice");
    assert_eq!(main["default_open_thread_id"], "thread::support");
    assert_eq!(main["endpoint_count"], 1);
    assert_eq!(main["bound_endpoint_count"], 1);
    assert_eq!(main["endpoints"][0]["thread_id"], "thread::support");
    assert_eq!(main["conversation_nodes"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn bot_consoles_route_ignores_unconfigured_endpoint_accounts() {
    let config = test_config();
    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();

    state
        .threads
        .thread_store
        .set(
            "thread::api-smoke",
            serde_json::json!({
                "thread_id": "thread::api-smoke",
                "label": "api/main/e2e-image-smoke",
                "workspace_dir": "/tmp/api-smoke",
                "updated_at": "2026-03-16T01:00:00Z",
                "channel_bindings": [{
                    "channel": "api",
                    "account_id": "main",
                    "peer_id": "e2e-image-smoke",
                    "chat_id": "e2e-image-smoke",
                    "display_label": "api/main/e2e-image-smoke",
                    "last_inbound_at": "2026-03-16T01:00:00Z"
                }]
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/bot-consoles")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();

    assert!(bots.iter().all(|entry| entry["id"] != "api::main"));
}

// ---------------------------------------------------------------------
// Team block in thread metadata response.
// ---------------------------------------------------------------------

async fn seed_product_ship_team(state: &Arc<AppState>) {
    use crate::agent_teams::UpsertAgentTeamRequest;
    state
        .ops
        .agent_teams
        .upsert_team(UpsertAgentTeamRequest {
            team_id: "product-ship".to_owned(),
            display_name: "Product Ship".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec![
                "planner".to_owned(),
                "coder".to_owned(),
                "reviewer".to_owned(),
            ],
            workflow_text: "Ship the product.".to_owned(),
        })
        .await
        .expect("team upsert");
}

#[tokio::test]
async fn thread_metadata_omits_team_block_for_standalone_agent_thread() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::standalone-claude";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "claude",
                "provider_type": "claude_code",
            }),
        )
        .await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let response = thread_metadata_response(&state, thread_id, &data).await;
    assert!(
        response.get("team").is_none(),
        "standalone-agent thread must not emit `team`, got: {response}"
    );
}

#[tokio::test]
async fn thread_metadata_emits_empty_child_map_when_group_never_persisted() {
    let (state, _logger, _dir) = test_state().await;
    seed_product_ship_team(&state).await;

    let thread_id = "thread::team-fresh";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "product-ship",
                "provider_type": "agent_team",
            }),
        )
        .await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let response = thread_metadata_response(&state, thread_id, &data).await;
    let team = response
        .get("team")
        .expect("team-bound thread emits `team`");
    assert_eq!(team["team_id"], "product-ship");
    assert_eq!(team["display_name"], "Product Ship");
    assert_eq!(team["leader_agent_id"], "planner");
    let members = team["member_agent_ids"].as_array().expect("members");
    assert_eq!(members.len(), 3);
    let child_map = team["child_thread_ids"]
        .as_object()
        .expect("child_thread_ids must be an object, not null");
    assert!(
        child_map.is_empty(),
        "fresh team thread has no Group yet, expected {{}} got {:?}",
        child_map
    );
}

#[tokio::test]
async fn thread_metadata_projects_known_child_thread_ids_from_group_store() {
    use garyx_bridge::providers::agent_team::Group;
    let (state, _logger, _dir) = test_state().await;
    seed_product_ship_team(&state).await;

    let thread_id = "thread::team-partial";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "product-ship",
                "provider_type": "agent_team",
            }),
        )
        .await;

    // Seed a Group that has seen `coder` but not `reviewer`.
    let mut group = Group::new(thread_id, "product-ship");
    group.record_child_thread("coder", "th::child-coder-0001");
    group.record_child_thread("ghost", "th::child-ghost-0001");
    state.ops.agent_team_group_store.save(&group).await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let response = thread_metadata_response(&state, thread_id, &data).await;
    let team = response.get("team").expect("team block present");
    let child_map = team["child_thread_ids"]
        .as_object()
        .expect("child_thread_ids object");
    assert_eq!(
        child_map.get("coder").and_then(Value::as_str),
        Some("th::child-coder-0001")
    );
    assert!(
        !child_map.contains_key("reviewer"),
        "reviewer has no child thread yet, should be absent from the map"
    );
    assert!(
        !child_map.contains_key("ghost"),
        "stale child thread from a removed team member should be filtered out"
    );
}

#[tokio::test]
async fn thread_summary_emits_team_block_for_team_bound_thread() {
    // Contract: the `/api/threads` list endpoint's per-thread summary
    // must carry the same `team` block the detail endpoint emits, so the
    // desktop client can render team branding without a second request.
    use garyx_bridge::providers::agent_team::Group;
    let (state, _logger, _dir) = test_state().await;
    seed_product_ship_team(&state).await;

    let thread_id = "thread::list-team-summary";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "product-ship",
                "provider_type": "agent_team",
            }),
        )
        .await;

    let mut group = Group::new(thread_id, "product-ship");
    group.record_child_thread("coder", "th::child-coder-42");
    state.ops.agent_team_group_store.save(&group).await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let summary = thread_summary_with_history(&state, thread_id, &data).await;
    let team = summary
        .get("team")
        .expect("team-bound thread summary must carry `team`");
    assert_eq!(team["team_id"], "product-ship");
    assert_eq!(team["display_name"], "Product Ship");
    assert_eq!(
        team["child_thread_ids"]["coder"],
        Value::String("th::child-coder-42".to_owned()),
    );
}

#[tokio::test]
async fn thread_summary_omits_team_block_for_standalone_agent_thread() {
    // Inverse of the test above: standalone-agent threads must not be
    // decorated with a phantom `team` block just because the summary
    // pipeline runs in the same function.
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::list-standalone-summary";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "claude",
                "provider_type": "claude_code",
            }),
        )
        .await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let summary = thread_summary_with_history(&state, thread_id, &data).await;
    assert!(
        summary.get("team").is_none(),
        "standalone-agent summary must not emit `team`, got: {summary}"
    );
}

#[tokio::test]
async fn task_routes_resolve_percent_encoded_qualified_refs() {
    let dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(dir.path().to_string_lossy().to_string());
    let state = AppStateBuilder::new(config).build();
    let router = build_router(state);

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "scope": {"channel": "telegram", "account_id": "main"},
                "title": "Check task routing"
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let task_ref = payload["task_ref"].as_str().unwrap();
    assert_eq!(task_ref, "#telegram/main/1");

    let request = authed_request()
        .method("GET")
        .uri(format!("/api/tasks/{}", urlencoding::encode(task_ref)))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["task_ref"], "#telegram/main/1");
    assert_eq!(payload["task"]["title"], "Check task routing");
}
