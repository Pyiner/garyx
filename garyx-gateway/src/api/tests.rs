#![allow(clippy::needless_update)]

use super::*;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use garyx_models::config::GaryxConfig;
use tempfile::tempdir;
use tower::ServiceExt;

fn test_state() -> Arc<AppState> {
    test_state_with_config(GaryxConfig::default())
}

fn test_state_with_config(config: GaryxConfig) -> Arc<AppState> {
    use crate::composition::app_bootstrap::AppStateBuilder;
    // Use in-memory stores to avoid filesystem races between concurrent tests.
    AppStateBuilder::new(config)
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .build()
}

async fn seed_transcript_backed_thread(state: &Arc<AppState>, thread_id: &str, mut data: Value) {
    let messages = data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(obj) = data.as_object_mut() {
        obj.insert(
            "message_count".to_owned(),
            Value::Number(serde_json::Number::from(messages.len() as u64)),
        );
        obj.insert(
            "history".to_owned(),
            json!({
                "source": "transcript_v1",
                "message_count": messages.len(),
            }),
        );
    }
    state
        .threads
        .thread_store
        .set(thread_id, data)
        .await
        .unwrap();
    if !messages.is_empty() {
        state
            .threads
            .history
            .transcript_store()
            .rewrite_from_messages(thread_id, &messages)
            .await
            .unwrap();
    }
}

fn api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/threads/history", axum::routing::get(thread_history))
        .route(
            "/api/threads/diagnostics",
            axum::routing::get(thread_diagnostics),
        )
        .route("/api/bot/status", axum::routing::get(bot_status))
        .route("/api/bot/bind", axum::routing::post(bot_bind))
        .route("/api/bot/unbind", axum::routing::post(bot_unbind))
        .route(
            "/api/custom-agents",
            axum::routing::get(list_custom_agents).post(create_custom_agent),
        )
        .route(
            "/api/provider-models/{provider_type}",
            axum::routing::get(list_provider_models),
        )
        .route(
            "/api/custom-agents/{agent_id}",
            axum::routing::get(get_custom_agent)
                .put(update_custom_agent)
                .delete(delete_custom_agent),
        )
        .route("/api/cron/jobs", axum::routing::get(cron_jobs))
        .route("/api/cron/runs", axum::routing::get(cron_runs))
        .route(
            "/api/debug/system-cron-jobs",
            axum::routing::get(debug_system_cron_jobs),
        )
        .route(
            "/api/debug/system-cron-jobs/{id}/run",
            axum::routing::post(debug_run_system_cron_job),
        )
        .route("/api/settings", axum::routing::put(settings_update))
        .route("/api/settings/reload", axum::routing::post(settings_reload))
        .route("/api/restart", axum::routing::post(restart))
        .with_state(state)
}

#[test]
fn test_stringify_message_content_summarizes_text_and_images() {
    let content = json!([
        {
            "type": "text",
            "text": "look at this"
        },
        {
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": "abc123=="
            }
        }
    ]);

    assert_eq!(
        stringify_message_content(&content),
        "look at this\n\n[1 image]"
    );
}

#[test]
fn test_stringify_message_content_summarizes_image_only_payloads() {
    let content = json!([
        {
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": "abc123=="
            }
        },
        {
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": "def456=="
            }
        }
    ]);

    assert_eq!(stringify_message_content(&content), "[2 images]");
}

#[test]
fn test_stringify_message_content_preserves_text_beyond_5000_characters() {
    let long = "x".repeat(8_339);
    let rendered = stringify_message_content(&json!(long.clone()));
    assert_eq!(rendered.chars().count(), long.chars().count());
    assert_eq!(rendered, long);
}

#[tokio::test]
async fn test_thread_history_empty() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["total"], 0);
    assert_eq!(json["limit"], 50);
    assert!(json["threads"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_thread_history_with_data() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set("thread::agent1-user1", json!({"msg": "hello"}))
        .await
        .unwrap();
    state
        .threads
        .thread_store
        .set("thread::agent1-user2", json!({"msg": "world"}))
        .await
        .unwrap();

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?limit=1&include_messages=true")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["total"], 2);
    assert_eq!(json["limit"], 1);
    assert_eq!(json["threads"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_thread_diagnostics_returns_ledger_records() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::diagnostics-alpha",
        json!({
            "messages": [{
                "role": "user",
                "content": "hello",
                "timestamp": "2026-03-22T10:00:00Z"
            }]
        }),
    )
    .await;
    state
        .threads
        .message_ledger
        .append_event(garyx_models::MessageLedgerEvent {
            ledger_id: "ledger-1".to_owned(),
            bot_id: "telegram:main".to_owned(),
            status: garyx_models::MessageLifecycleStatus::RunInterrupted,
            created_at: "2026-03-22T10:00:01Z".to_owned(),
            thread_id: Some("thread::diagnostics-alpha".to_owned()),
            run_id: Some("run-1".to_owned()),
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            chat_id: Some("-100".to_owned()),
            from_id: Some("42".to_owned()),
            native_message_id: Some("tg-1".to_owned()),
            text_excerpt: Some("hello".to_owned()),
            terminal_reason: Some(garyx_models::MessageTerminalReason::SelfRestart),
            reply_message_id: None,
            metadata: json!({"reason":"restart"}),
        })
        .await
        .unwrap();
    state
        .threads
        .thread_store
        .set(
            "thread::diagnostics-alpha",
            json!({
                "thread_id": "thread::diagnostics-alpha",
                "provider_type": "claude_code",
                "sdk_session_id": "sdk-123",
                "history": {
                    "message_count": 0
                }
            }),
        )
        .await
        .unwrap();

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/diagnostics?thread_id=thread::diagnostics-alpha")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["thread_id"], "thread::diagnostics-alpha");
    assert_eq!(json["thread_runtime"]["provider_type"], "claude_code");
    assert_eq!(json["thread_runtime"]["provider_label"], "Claude");
    assert_eq!(json["thread_runtime"]["sdk_session_id"], "sdk-123");
    assert!(json["thread_runtime"]["active_run"].is_null());
    assert_eq!(
        json["message_ledger"]["records"][0]["terminal_reason"],
        "self_restart"
    );
}

#[tokio::test]
async fn test_bot_status_returns_current_bound_thread_only() {
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token".to_owned(),
                    enabled: true,
                    name: Some("Telegram Main".to_owned()),
                    agent_id: "codex".to_owned(),
                    workspace_dir: Some("/tmp/current-workspace".to_owned()),
                    owner_target: Some(garyx_models::config::OwnerTargetConfig {
                        target_type: "chat_id".to_owned(),
                        target_id: "42".to_owned(),
                    }),
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    let state = test_state_with_config(config);
    state
        .threads
        .thread_store
        .set(
            "thread::old-history",
            json!({
                "thread_id": "thread::old-history",
                "workspace_dir": "/tmp/old-workspace",
                "channel": "telegram",
                "account_id": "main",
                "from_id": "42",
                "channel_bindings": [],
            }),
        )
        .await
        .unwrap();
    state
        .threads
        .thread_store
        .set(
            "thread::current",
            json!({
                "thread_id": "thread::current",
                "workspace_dir": "/tmp/current-workspace",
                "provider_type": "codex_app_server",
                "channel": "telegram",
                "account_id": "main",
                "from_id": "42",
                "updated_at": "2026-05-02T12:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "42",
                    "chat_id": "42",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "42",
                    "display_label": "Telegram Main"
                }],
            }),
        )
        .await
        .unwrap();
    state
        .threads
        .message_ledger
        .append_event(garyx_models::MessageLedgerEvent {
            ledger_id: "ledger-1".to_owned(),
            bot_id: "telegram:main".to_owned(),
            status: garyx_models::MessageLifecycleStatus::RunInterrupted,
            created_at: "2026-03-22T10:00:01Z".to_owned(),
            thread_id: Some("thread::old-history".to_owned()),
            run_id: Some("run-1".to_owned()),
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            chat_id: Some("42".to_owned()),
            from_id: Some("42".to_owned()),
            native_message_id: Some("tg-1".to_owned()),
            text_excerpt: Some("hello".to_owned()),
            terminal_reason: Some(garyx_models::MessageTerminalReason::SelfRestart),
            reply_message_id: None,
            metadata: json!({}),
        })
        .await
        .unwrap();

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/bot/status?bot_id=telegram:main")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["current_thread_id"], "thread::current");
    assert_eq!(json["current_thread_status"], "bound");
    assert_eq!(
        json["main_endpoint"]["workspace_dir"],
        "/tmp/current-workspace"
    );
    assert!(json.get("recent_records").is_none());
    assert!(json.get("problem_threads").is_none());
}

#[tokio::test]
async fn test_bot_bind_rebinds_main_endpoint_to_existing_thread() {
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token".to_owned(),
                    enabled: true,
                    name: Some("Telegram Main".to_owned()),
                    agent_id: "codex".to_owned(),
                    workspace_dir: Some("/tmp/current-workspace".to_owned()),
                    owner_target: Some(garyx_models::config::OwnerTargetConfig {
                        target_type: "chat_id".to_owned(),
                        target_id: "1000000001".to_owned(),
                    }),
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    let state = test_state_with_config(config);
    state
        .threads
        .thread_store
        .set(
            "thread::source",
            json!({
                "thread_id": "thread::source",
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
                    "display_label": "Telegram Main"
                }],
            }),
        )
        .await
        .unwrap();
    state
        .threads
        .thread_store
        .set(
            "thread::target",
            json!({
                "thread_id": "thread::target",
                "channel": "telegram",
                "account_id": "main",
                "from_id": "1000000001",
                "channel_bindings": [],
            }),
        )
        .await
        .unwrap();

    let router = api_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/bot/bind")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "botId": "telegram:main",
                "threadId": "thread::target",
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["action"], "bind");
    assert_eq!(json["current_thread_id"], "thread::target");
    assert_eq!(json["previous_thread_id"], "thread::source");

    let source = state
        .threads
        .thread_store
        .get("thread::source")
        .await
        .unwrap()
        .unwrap();
    let target = state
        .threads
        .thread_store
        .get("thread::target")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source["channel_bindings"].as_array().unwrap().len(), 0);
    assert_eq!(target["channel_bindings"].as_array().unwrap().len(), 1);
    assert_eq!(
        target["channel_bindings"][0]["account_id"],
        Value::String("main".to_owned())
    );
}

#[tokio::test]
async fn test_bot_bind_rejects_cross_channel_thread() {
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token".to_owned(),
                    enabled: true,
                    name: Some("Telegram Main".to_owned()),
                    agent_id: "codex".to_owned(),
                    workspace_dir: None,
                    owner_target: Some(garyx_models::config::OwnerTargetConfig {
                        target_type: "chat_id".to_owned(),
                        target_id: "1000000001".to_owned(),
                    }),
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    let state = test_state_with_config(config);
    state
        .threads
        .thread_store
        .set(
            "thread::feishu",
            json!({
                "thread_id": "thread::feishu",
                "channel": "feishu",
                "account_id": "main",
                "from_id": "ou_1000000001",
                "channel_bindings": [],
            }),
        )
        .await
        .unwrap();

    let router = api_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/bot/bind")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "botId": "telegram:main",
                "threadId": "thread::feishu",
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["reason"], "thread-not-compatible");
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("thread belongs to channel 'feishu'")
    );
}

#[tokio::test]
async fn test_bot_unbind_clears_main_endpoint_binding() {
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token".to_owned(),
                    enabled: true,
                    name: Some("Telegram Main".to_owned()),
                    agent_id: "codex".to_owned(),
                    workspace_dir: None,
                    owner_target: Some(garyx_models::config::OwnerTargetConfig {
                        target_type: "chat_id".to_owned(),
                        target_id: "1000000001".to_owned(),
                    }),
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    let state = test_state_with_config(config);
    state
        .threads
        .thread_store
        .set(
            "thread::current",
            json!({
                "thread_id": "thread::current",
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
                    "display_label": "Telegram Main"
                }],
            }),
        )
        .await
        .unwrap();

    let router = api_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/bot/unbind")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "botId": "telegram:main",
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["action"], "unbind");
    assert_eq!(json["previous_thread_id"], "thread::current");
    assert_eq!(json["current_thread_status"], "unbound");
    assert!(json["current_thread_id"].is_null());

    let thread = state
        .threads
        .thread_store
        .get("thread::current")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(thread["channel_bindings"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_create_and_list_custom_agents() {
    let state = test_state();
    let router = api_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/custom-agents")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "agent_id": "spec-review",
                "display_name": "Spec Review",
                "role": "reviewer",
                "provider_type": "codex_app_server",
                "model": "gpt-5-codex",
                "model_reasoning_effort": "xhigh",
                "model_service_tier": "priority",
                "avatar_data_url": "data:image/png;base64,dGVzdA==",
                "system_prompt": "Review specs carefully."
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = Request::builder()
        .method("GET")
        .uri("/api/custom-agents")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["agents"].as_array().unwrap().iter().any(|agent| {
        agent["agent_id"] == "claude"
            && agent["avatar_data_url"]
                .as_str()
                .is_some_and(|value| value.starts_with("data:image/png;base64,"))
            && agent["provider_icon"]["key"] == "claude"
            && agent["provider_icon"]["provider_type"] == "claude_code"
    }));
    assert!(json["agents"].as_array().unwrap().iter().any(|agent| {
        agent["agent_id"] == "spec-review"
            && agent["provider_type"] == "codex_app_server"
            && agent["display_name"] == "Spec Review"
            && agent["model"] == "gpt-5-codex"
            && agent["model_reasoning_effort"] == "xhigh"
            && agent["model_service_tier"] == "priority"
            && agent["avatar_data_url"] == "data:image/png;base64,dGVzdA=="
            && agent["provider_icon"]["key"] == "codex"
    }));
}

#[tokio::test]
async fn test_create_custom_agent_allows_omitted_model() {
    let state = test_state();
    let router = api_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/custom-agents")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "agent_id": "plain-claude",
                "display_name": "Plain Claude",
                "provider_type": "claude_code",
                "system_prompt": "Work normally."
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["model"], "");
}

#[tokio::test]
async fn test_create_custom_agent_allows_omitted_system_prompt() {
    let state = test_state();
    let router = api_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/custom-agents")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "agent_id": "plain-claude",
                "display_name": "Plain Claude",
                "provider_type": "claude_code"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["system_prompt"], "");
}

#[tokio::test]
async fn test_custom_agent_optimistic_concurrency_contract() {
    let state = test_state();
    let router = api_router(state);
    let create_body = json!({
        "agent_id": "occ-agent",
        "display_name": "OCC Agent",
        "provider_type": "codex_app_server",
    });
    let create = |body: Value| {
        Request::builder()
            .method("POST")
            .uri("/api/custom-agents")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let resp = router
        .clone()
        .oneshot(create(create_body.clone()))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&created).unwrap();
    let token = created["updated_at"]
        .as_str()
        .expect("updated_at")
        .to_owned();

    // POST is strict create: the same id conflicts instead of overwriting.
    let resp = router.clone().oneshot(create(create_body)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    let put = |body: Value| {
        Request::builder()
            .method("PUT")
            .uri("/api/custom-agents/occ-agent")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };

    // PUT without the token is a 400 with guidance.
    let resp = router
        .clone()
        .oneshot(put(json!({
            "agent_id": "occ-agent",
            "display_name": "Renamed",
            "provider_type": "codex_app_server",
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // PUT with the fresh token succeeds and rotates updated_at.
    let resp = router
        .clone()
        .oneshot(put(json!({
            "agent_id": "occ-agent",
            "display_name": "Renamed",
            "provider_type": "codex_app_server",
            "expected_updated_at": token,
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let updated: Value = serde_json::from_slice(&updated).unwrap();
    let new_token = updated["updated_at"]
        .as_str()
        .expect("updated_at")
        .to_owned();
    assert_ne!(new_token, token, "updated_at must rotate on write");

    // Replaying the stale token is a 409 carrying the current updated_at.
    let resp = router
        .clone()
        .oneshot(put(json!({
            "agent_id": "occ-agent",
            "display_name": "Stale Writer",
            "provider_type": "claude_code",
            "expected_updated_at": token,
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let conflict = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let conflict: Value = serde_json::from_slice(&conflict).unwrap();
    assert_eq!(conflict["current_updated_at"], new_token.as_str());

    // Deleting then PUTting with any token is a 404 — no resurrection.
    let delete = Request::builder()
        .method("DELETE")
        .uri("/api/custom-agents/occ-agent")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(delete).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let resp = router
        .clone()
        .oneshot(put(json!({
            "agent_id": "occ-agent",
            "display_name": "Ghost",
            "provider_type": "codex_app_server",
            "expected_updated_at": new_token,
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_custom_agent_blank_system_prompt_clears_prompt() {
    let state = test_state();
    let router = api_router(state);
    let create = Request::builder()
        .method("POST")
        .uri("/api/custom-agents")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "agent_id": "plain-claude",
                "display_name": "Plain Claude",
                "provider_type": "claude_code",
                "system_prompt": "Work normally."
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.clone().oneshot(create).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&created).unwrap();
    let updated_at = created["updated_at"].as_str().expect("updated_at");

    let update = Request::builder()
        .method("PUT")
        .uri("/api/custom-agents/plain-claude")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "agent_id": "plain-claude",
                "display_name": "Plain Claude",
                "provider_type": "claude_code",
                "system_prompt": "  ",
                "expected_updated_at": updated_at
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.oneshot(update).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["system_prompt"], "");
}

#[tokio::test]
async fn test_provider_models_reports_claude_code_catalog() {
    let state = test_state();
    let router = api_router(state);
    let req = Request::builder()
        .method("GET")
        .uri("/api/provider-models/claude_code")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["provider_type"], "claude_code");
    assert_eq!(json["supports_model_selection"], true);
    assert_eq!(json["supports_reasoning_effort_selection"], true);
    assert_eq!(json["default_model"], Value::Null);
    assert!(!json["models"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_provider_models_rejects_unknown_provider() {
    let state = test_state();
    let router = api_router(state);
    let req = Request::builder()
        .method("GET")
        .uri("/api/provider-models/unknown-provider")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_thread_history_with_prefix() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set("thread::agent1-user1", json!({"msg": "a"}))
        .await
        .unwrap();
    state
        .threads
        .thread_store
        .set("thread::agent2-user2", json!({"msg": "b"}))
        .await
        .unwrap();

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?prefix=thread::agent1")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["total"], 1);
}

#[tokio::test]
async fn test_thread_history_detail_with_thread_id_and_tool_messages() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "main::main::u1",
        json!({
            "messages": [
                {"role": "user", "content": "hello", "timestamp": "2026-03-01T00:00:00Z"},
                {"role": "assistant", "content": "world", "timestamp": "2026-03-01T00:00:01Z"}
            ],
            "outbound_message_ids": [
                {"channel": "telegram", "account_id": "main", "chat_id": "u1", "message_id": 123, "timestamp": "2026-03-01T00:00:02Z"}
            ],
            "channel": "telegram",
            "account_id": "main",
            "from_id": "u1"
        }),
    )
    .await;

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?thread_id=main%3A%3Amain%3A%3Au1&limit=10&include_tool_messages=true")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["thread"]["thread_id"], "main::main::u1");
    assert_eq!(json["thread"]["thread_key"], "main::main::u1");
    assert_eq!(json["thread"]["thread_type"], "chat");
    assert_eq!(json["session"]["thread_id"], "main::main::u1");
    assert_eq!(json["session"]["thread_key"], "main::main::u1");
    assert_eq!(json["session"]["thread_type"], "chat");
    assert_eq!(json["message_stats"]["total_messages_in_thread"], 2);
    assert_eq!(json["message_stats"]["total_messages_in_session"], 2);
    assert_eq!(json["message_stats"]["returned_messages"], 2);
    assert_eq!(json["messages"].as_array().unwrap().len(), 2);
    assert_eq!(json["outbound_total"], 1);
    assert_eq!(json["messages"][0]["text"], "hello");
    assert_eq!(json["messages"][0]["message"]["role"], "user");
    assert_eq!(json["messages"][1]["text"], "world");
    assert_eq!(json["messages"][1]["message"]["content"], "world");
}

#[tokio::test]
async fn test_thread_history_preserves_complete_long_task_notification_content() {
    const CAPTURED_TEXT_CHARS: usize = 8_339;
    let state = test_state();
    let prefix = [
        r##"<garyx_task_notification event="ready_for_review" task_id="#TASK-42" status="in_review">"##,
        "Task #TASK-42 is ready for review: Synthetic renderer review",
        "",
        "# Review conclusion: FAIL",
        "",
    ]
    .join("\n");
    let suffix = [
        "",
        "View details:",
        "garyx task get #TASK-42",
        "</garyx_task_notification>",
    ]
    .join("\n");
    let padding_chars = CAPTURED_TEXT_CHARS - prefix.chars().count() - suffix.chars().count();
    let notification = format!("{prefix}{}{suffix}", "x".repeat(padding_chars));
    assert_eq!(notification.chars().count(), CAPTURED_TEXT_CHARS);

    seed_transcript_backed_thread(
        &state,
        "thread::long-task-notification",
        json!({
            "messages": [{
                "role": "user",
                "text": notification,
                "content": notification,
                "internal": true,
                "metadata": {
                    "internal_dispatch": true,
                    "task_notification": true,
                    "task_notification_event": "ready_for_review",
                    "task_id": "#TASK-42"
                }
            }]
        }),
    )
    .await;

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Along-task-notification&limit=10")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let message = &json["messages"][0];
    let text = message["text"].as_str().expect("history text");
    let content = message["content"].as_str().expect("history content");
    let nested_content = message["message"]["content"]
        .as_str()
        .expect("nested history message content");
    assert_eq!(text.chars().count(), CAPTURED_TEXT_CHARS);
    assert_eq!(content.chars().count(), CAPTURED_TEXT_CHARS);
    assert_eq!(nested_content.chars().count(), CAPTURED_TEXT_CHARS);
    assert_eq!(text, notification);
    assert_eq!(content, notification);
    assert_eq!(nested_content, notification);
    assert!(content.ends_with("</garyx_task_notification>"));
    assert!(nested_content.ends_with("</garyx_task_notification>"));
    assert!(!content.contains("[truncated:"));
    assert!(!nested_content.contains("[truncated:"));
}

#[tokio::test]
async fn test_thread_history_detail_preserves_task_thread_type() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::task-history",
        json!({
            "thread_kind": "task",
            "label": "Synthetic task run",
            "messages": [
                {"role": "assistant", "content": "task output", "timestamp": "2026-03-01T00:00:01Z"}
            ]
        }),
    )
    .await;

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Atask-history&limit=10")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["thread"]["thread_type"], "task");
    assert_eq!(json["session"]["thread_type"], "task");
}

#[tokio::test]
async fn test_thread_history_detail_defaults_missing_thread_kind_to_chat() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "cron::legacy-history",
        json!({
            "label": "Legacy cron-shaped thread",
            "messages": [
                {"role": "assistant", "content": "legacy output", "timestamp": "2026-03-01T00:00:01Z"}
            ]
        }),
    )
    .await;

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?thread_id=cron%3A%3Alegacy-history&limit=10")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["thread"]["thread_type"], "chat");
    assert_eq!(json["session"]["thread_type"], "chat");
}

#[tokio::test]
async fn test_thread_history_detail_pages_before_global_index() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::paged",
        json!({
            "messages": [
                {"role": "user", "content": "m0"},
                {"role": "assistant", "content": "m1"},
                {"role": "user", "content": "m2"},
                {"role": "assistant", "content": "m3"},
                {"role": "user", "content": "m4"}
            ]
        }),
    )
    .await;

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Apaged&limit=2&before_index=3")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["message_stats"]["total_messages_in_thread"], 5);
    assert_eq!(json["message_stats"]["returned_messages"], 2);
    assert_eq!(json["message_stats"]["returned_start_index"], 1);
    assert_eq!(json["message_stats"]["returned_end_index"], 3);
    assert_eq!(json["message_stats"]["has_more_before"], true);
    assert_eq!(json["message_stats"]["next_before_index"], 1);
    assert_eq!(json["messages"][0]["index"], 1);
    assert_eq!(json["messages"][0]["text"], "m1");
    assert_eq!(json["messages"][1]["index"], 2);
    assert_eq!(json["messages"][1]["text"], "m2");
}

#[tokio::test]
async fn test_thread_history_after_index_returns_newer_messages() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::fwd",
        json!({
            "messages": [
                {"role": "user", "content": "m0"},
                {"role": "assistant", "content": "m1"},
                {"role": "user", "content": "m2"},
                {"role": "assistant", "content": "m3"},
                {"role": "user", "content": "m4"}
            ]
        }),
    )
    .await;
    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Afwd&after_index=1&limit=10")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["message_stats"]["returned_messages"], 3);
    assert_eq!(json["message_stats"]["returned_start_index"], 2);
    assert_eq!(json["message_stats"]["has_more_after"], false);
    assert_eq!(json["message_stats"]["next_after_index"], Value::Null);
    assert_eq!(json["messages"][0]["index"], 2);
    assert_eq!(json["messages"][0]["text"], "m2");
    assert_eq!(json["messages"][2]["text"], "m4");
}

#[tokio::test]
async fn test_thread_history_after_index_caught_up_reports_no_more() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::fwd-caught-up",
        json!({
            "messages": [
                {"role": "user", "content": "m0"},
                {"role": "assistant", "content": "m1"}
            ]
        }),
    )
    .await;
    let router = api_router(state);
    // Client already at the last index → empty page must report caught up,
    // not has_more_after=true / next_after_index=0 (which would loop a full refetch).
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Afwd-caught-up&after_index=1&limit=10")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["message_stats"]["returned_messages"], 0);
    assert_eq!(json["message_stats"]["has_more_after"], false);
    assert_eq!(json["message_stats"]["next_after_index"], Value::Null);
}

#[tokio::test]
async fn test_thread_history_after_index_has_more_when_bounded() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::fwd-bounded",
        json!({
            "messages": [
                {"role": "user", "content": "m0"},
                {"role": "assistant", "content": "m1"},
                {"role": "user", "content": "m2"},
                {"role": "assistant", "content": "m3"},
                {"role": "user", "content": "m4"}
            ]
        }),
    )
    .await;
    let router = api_router(state);
    // after=0, limit=2 → m1,m2; more remain → has_more_after, next_after_index=2
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Afwd-bounded&after_index=0&limit=2")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["message_stats"]["returned_messages"], 2);
    assert_eq!(json["message_stats"]["returned_start_index"], 1);
    assert_eq!(json["message_stats"]["has_more_after"], true);
    assert_eq!(json["message_stats"]["next_after_index"], 2);
    assert_eq!(json["messages"][0]["text"], "m1");
    assert_eq!(json["messages"][1]["text"], "m2");
}

#[tokio::test]
async fn test_thread_history_detail_pages_by_user_queries() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::turns",
        json!({
            "messages": [
                {"role": "user", "content": "q0"},
                {"role": "tool_use", "content": {"name": "lookup"}},
                {"role": "assistant", "content": "a0"},
                {"role": "user", "content": "q1"},
                {"role": "tool_result", "content": "middle"},
                {"role": "assistant", "content": "a1"},
                {"role": "user", "content": "q2"},
                {"role": "assistant", "content": "a2"}
            ]
        }),
    )
    .await;

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Aturns&limit=2&user_query_limit=2")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["message_stats"]["returned_messages"], 5);
    assert_eq!(json["message_stats"]["returned_user_queries"], 2);
    assert_eq!(json["message_stats"]["returned_start_index"], 3);
    assert_eq!(json["message_stats"]["next_before_index"], 3);
    assert_eq!(json["messages"][0]["text"], "q1");
    assert_eq!(json["messages"][1]["text"], "middle");
    assert_eq!(json["messages"][2]["text"], "a1");
    assert_eq!(json["messages"][3]["text"], "q2");
    assert_eq!(json["messages"][4]["text"], "a2");
}

#[tokio::test]
async fn test_thread_history_detail_filters_tool_messages() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "main::main::u2",
        json!({
            "messages": [
                {"role": "user", "content": "hi", "timestamp": "2026-03-01T00:00:00Z"},
                {"role": "tool_use", "content": {"tool_use_id": "tool_1"}, "timestamp": "2026-03-01T00:00:01Z"},
                {"role": "assistant", "content": "done", "timestamp": "2026-03-01T00:00:02Z"}
            ]
        }),
    )
    .await;

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?thread_id=main%3A%3Amain%3A%3Au2&include_tool_messages=false")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["message_stats"]["returned_messages"], 2);
    assert_eq!(
        json["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["tool_related"].as_bool().unwrap_or(false))
            .count(),
        0
    );
}

#[tokio::test]
async fn test_thread_history_projection_preserves_control_kind_without_visible_text() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::control-projection",
        json!({
            "messages": [
                {
                    "role": "system",
                    "kind": "control",
                    "internal": true,
                    "internal_kind": "control",
                    "control": {
                        "kind": "run_start",
                        "thread_id": "thread::control-projection",
                        "run_id": "run-control",
                        "at": "2026-06-18T12:00:00Z"
                    }
                },
                {"role": "user", "content": "hi", "timestamp": "2026-06-18T12:00:01Z"}
            ]
        }),
    )
    .await;

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Acontrol-projection&include_tool_messages=false")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["message_stats"]["returned_messages"], 2);
    assert_eq!(json["messages"][0]["kind"], "control");
    assert_eq!(json["messages"][0]["tool_related"], false);
    assert_eq!(json["messages"][0]["likely_user_visible"], false);
    assert_eq!(json["messages"][0]["text"], "");
    assert_eq!(json["messages"][0]["content"], "");
    assert_eq!(
        json["messages"][0]["message"]["control"]["kind"],
        "run_start"
    );
}

#[tokio::test]
async fn test_thread_history_detail_repairs_orphaned_pending_user_inputs() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::pending-u3",
        json!({
            "messages": [
                {"role": "user", "content": "hello", "timestamp": "2026-03-01T00:00:00Z"}
            ],
            "pending_user_inputs": [
                {
                    "id": "pending-1",
                    "bridge_run_id": "run-not-active",
                    "text": "follow-up after reconnect",
                    "content": [{"type": "text", "text": "follow-up after reconnect"}],
                    "queued_at": "2026-03-01T00:00:05Z",
                    "status": "queued"
                }
            ]
        }),
    )
    .await;

    let router = api_router(state.clone());

    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Apending-u3&limit=10")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["pending_user_inputs"].as_array().unwrap().len(), 0);
    assert_eq!(json["message_stats"]["pending_user_input_count"], 0);
    assert_eq!(json["message_stats"]["active_pending_user_input_count"], 0);

    let repaired = state
        .threads
        .thread_store
        .get("thread::pending-u3")
        .await
        .unwrap()
        .expect("thread should still exist");
    assert_eq!(
        repaired["pending_user_inputs"]
            .as_array()
            .expect("pending_user_inputs should stay as an array")
            .len(),
        0
    );
}

#[tokio::test]
async fn test_cron_jobs_no_service() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/cron/jobs")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["jobs"].as_array().unwrap().is_empty());
    assert_eq!(json["count"], 0);
    assert_eq!(json["service_available"], false);
}

#[tokio::test]
async fn test_cron_jobs_with_service() {
    let state = test_state();
    let tmp = tempfile::TempDir::new().unwrap();
    let svc = crate::cron::CronService::new(tmp.path().to_path_buf());
    let _ = tokio::fs::create_dir_all(tmp.path().join("cron").join("jobs")).await;
    svc.add(garyx_models::config::CronJobConfig {
        id: "test-job".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: garyx_models::config::CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: garyx_models::config::CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    // Replace state with cron service
    let mut state_with_cron = (*state).clone_for_test();
    state_with_cron.ops.cron_service = Some(Arc::new(svc));
    let state_with_cron = Arc::new(state_with_cron);

    let router = api_router(state_with_cron);

    let req = Request::builder()
        .uri("/api/cron/jobs")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["service_available"], true);
    assert_eq!(json["jobs"][0]["id"], "test-job");
}

#[tokio::test]
async fn test_cron_runs_no_service() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/cron/runs")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["runs"].as_array().unwrap().is_empty());
    assert_eq!(json["count"], 0);
}

// ---------------------------------------------------------------------------
// GET/POST /api/debug/system-cron-jobs (AXON-692)
// ---------------------------------------------------------------------------

/// Build a CronService seeded with a mix of system + non-system jobs so the
/// debug-endpoint tests can assert filtering behavior. Returns the service
/// (caller wraps it into AppState.ops.cron_service).
async fn seed_cron_service_for_debug() -> crate::cron::CronService {
    use garyx_models::config::{
        CronAction, CronJobConfig, CronJobKind, CronSchedule, InternalDispatchJobPayload,
    };
    let tmp = tempfile::TempDir::new().unwrap();
    let svc = crate::cron::CronService::new(tmp.path().to_path_buf());
    let _ = tokio::fs::create_dir_all(tmp.path().join("cron").join("jobs")).await;

    let far_future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();

    // Two system followup jobs on different threads.
    for (id, thread) in [
        ("followup_aaa", "thread::alpha"),
        ("followup_bbb", "thread::beta"),
    ] {
        svc.add(CronJobConfig {
            id: id.to_owned(),
            kind: CronJobKind::InternalDispatch {
                payload: InternalDispatchJobPayload {
                    prompt: "resume the build poll".to_owned(),
                    reason: Some("background build finished".to_owned()),
                    originating_run_id: Some("run-test-1".to_owned()),
                    scheduled_at: chrono::Utc::now(),
                    delay_seconds_requested: 300,
                },
            },
            label: Some(format!("schedule_followup({thread})")),
            schedule: CronSchedule::Once {
                at: far_future.clone(),
            },
            ui_schedule: None,
            action: CronAction::Log,
            target: None,
            message: None,
            workspace_dir: None,
            agent_id: None,
            thread_id: Some(thread.to_owned()),
            delete_after_run: true,
            enabled: true,
            system: true,
        })
        .await
        .unwrap();
    }

    // One ordinary (non-system) automation that must NOT appear in the debug list.
    svc.add(CronJobConfig {
        id: "user-automation".to_owned(),
        kind: Default::default(),
        label: Some("daily standup".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: Some("thread::alpha".to_owned()),
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    svc
}

#[tokio::test]
async fn test_debug_system_cron_jobs_no_service() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/debug/system-cron-jobs")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["service_available"], false);
    assert_eq!(json["count"], 0);
}

#[tokio::test]
async fn test_debug_system_cron_jobs_lists_only_system() {
    let state = test_state();
    let svc = seed_cron_service_for_debug().await;
    let mut state_with_cron = (*state).clone_for_test();
    state_with_cron.ops.cron_service = Some(Arc::new(svc));
    let router = api_router(Arc::new(state_with_cron));

    let req = Request::builder()
        .uri("/api/debug/system-cron-jobs")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["service_available"], true);
    // Two system jobs; the user-automation is filtered out.
    assert_eq!(json["count"], 2);
    let ids: Vec<&str> = json["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"followup_aaa"));
    assert!(ids.contains(&"followup_bbb"));
    assert!(!ids.contains(&"user-automation"));
    // Each job carries its internal_dispatch kind + a recent_runs array.
    let job = &json["jobs"][0];
    assert_eq!(job["kind"]["type"], "internal_dispatch");
    assert!(job["recent_runs"].is_array());
}

#[tokio::test]
async fn test_debug_system_cron_jobs_thread_filter() {
    let state = test_state();
    let svc = seed_cron_service_for_debug().await;
    let mut state_with_cron = (*state).clone_for_test();
    state_with_cron.ops.cron_service = Some(Arc::new(svc));
    let router = api_router(Arc::new(state_with_cron));

    let req = Request::builder()
        .uri("/api/debug/system-cron-jobs?thread_id=thread::beta")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["jobs"][0]["id"], "followup_bbb");
    assert_eq!(json["thread_id"], "thread::beta");
}

#[tokio::test]
async fn test_debug_system_cron_jobs_invalid_since_is_400() {
    let state = test_state();
    let svc = seed_cron_service_for_debug().await;
    let mut state_with_cron = (*state).clone_for_test();
    state_with_cron.ops.cron_service = Some(Arc::new(svc));
    let router = api_router(Arc::new(state_with_cron));

    let req = Request::builder()
        .uri("/api/debug/system-cron-jobs?since=not-a-date")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "invalid_since");
}

#[tokio::test]
async fn test_debug_system_cron_jobs_since_unix_filters() {
    let state = test_state();
    let svc = seed_cron_service_for_debug().await;
    let mut state_with_cron = (*state).clone_for_test();
    state_with_cron.ops.cron_service = Some(Arc::new(svc));
    let router = api_router(Arc::new(state_with_cron));

    // A `since` far in the future filters every job out (all created just now).
    let future_ts = (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp();
    let req = Request::builder()
        .uri(format!("/api/debug/system-cron-jobs?since={future_ts}"))
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 0);
}

#[tokio::test]
async fn test_debug_run_system_cron_job_missing_is_404() {
    let state = test_state();
    let svc = seed_cron_service_for_debug().await;
    let mut state_with_cron = (*state).clone_for_test();
    state_with_cron.ops.cron_service = Some(Arc::new(svc));
    let router = api_router(Arc::new(state_with_cron));

    let req = Request::builder()
        .method("POST")
        .uri("/api/debug/system-cron-jobs/does-not-exist/run")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_debug_run_system_cron_job_rejects_non_system() {
    let state = test_state();
    let svc = seed_cron_service_for_debug().await;
    let mut state_with_cron = (*state).clone_for_test();
    state_with_cron.ops.cron_service = Some(Arc::new(svc));
    let router = api_router(Arc::new(state_with_cron));

    // user-automation exists but is non-system → debug channel must 404 it,
    // never fire a user-visible automation.
    let req = Request::builder()
        .method("POST")
        .uri("/api/debug/system-cron-jobs/user-automation/run")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_settings_update_valid() {
    let state = test_state();
    let router = api_router(state);

    let config = GaryxConfig::default();
    let body_val = serde_json::to_value(&config).unwrap();

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_settings_update_invalid_json_value() {
    let state = test_state();
    let router = api_router(state);

    // A JSON array instead of an object
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(b"[1,2,3]".to_vec()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
}

#[tokio::test]
async fn test_settings_update_rejects_unknown_top_level_field() {
    let state = test_state();
    let router = api_router(state);

    let mut body_val = serde_json::to_value(GaryxConfig::default()).unwrap();
    body_val["unknown_top_level"] = json!(123);

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    let errors = json["errors"].as_array().unwrap();
    assert!(
        errors
            .iter()
            .any(|e| e.as_str().unwrap_or("").contains("$.unknown_top_level"))
    );
}

#[tokio::test]
async fn test_settings_update_rejects_unknown_nested_field() {
    let state = test_state();
    let router = api_router(state);

    let mut body_val = serde_json::to_value(GaryxConfig::default()).unwrap();
    body_val["gateway"]["unknown_nested"] = json!(true);

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(errors.iter().any(|e| {
        e.as_str()
            .unwrap_or("")
            .contains("$.gateway.unknown_nested")
    }));
}

#[tokio::test]
async fn test_settings_update_merge_true_rejects_unknown_patch_field() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings?merge=true")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "gateway": {
                    "public_urk": "https://garyx.example"
                }
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    let errors = json["errors"].as_array().unwrap();
    assert!(errors.iter().any(|error| {
        error
            .as_str()
            .unwrap_or_default()
            .contains("$.gateway.public_urk")
    }));
}

#[tokio::test]
async fn test_settings_roundtrip_persistence() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    // Write initial config
    let initial = GaryxConfig::default();
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    let state_with_path = Arc::new(state_with_path);

    let router = api_router(state_with_path.clone());

    // PUT new config with changed port
    let mut new_config = initial.clone();
    new_config.gateway.port = 9999;
    let body_val = serde_json::to_value(&new_config).unwrap();

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    // Verify file was written
    let file_content = tokio::fs::read_to_string(&config_path).await.unwrap();
    eprintln!("persisted_partial_settings={file_content}");
    let persisted: GaryxConfig = serde_json::from_str(&file_content).unwrap();
    assert_eq!(persisted.gateway.port, 9999);

    // Verify live_config was updated
    let live = state_with_path.config_snapshot();
    assert_eq!(live.gateway.port, 9999);
}

#[tokio::test]
async fn test_settings_update_merge_false_deletes_channel_account() {
    // Confirms the `merge=false` PUT flow actually DELETES an account
    // that's been omitted from the body — the default `merge=true` path
    // preserves absent fields by design, so a dedicated full-replace
    // opt-in is the only way a UI can remove an account via the HTTP
    // surface. Exercised against a generic plugin-owned channel
    // (`config.channels.plugins[id].accounts`) so the test stays
    // decoupled from any built-in channel's specific account shape.
    use garyx_models::config::{PluginAccountEntry, PluginChannelConfig};

    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    let mut initial = GaryxConfig::default();
    let mut plugin_cfg = PluginChannelConfig::default();
    plugin_cfg.accounts.insert(
        "bot-to-delete".to_owned(),
        PluginAccountEntry {
            enabled: true,
            name: Some("doomed".to_owned()),
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
            config: serde_json::json!({ "token": "secret" }),
        },
    );
    initial
        .channels
        .plugins
        .insert("sample_plugin".to_owned(), plugin_cfg);
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    state_with_path
        .apply_runtime_config(initial.clone())
        .await
        .unwrap();
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    // Build a PUT body where the plugin's accounts map is empty.
    let mut without_account = initial.clone();
    without_account
        .channels
        .plugins
        .get_mut("sample_plugin")
        .unwrap()
        .accounts
        .clear();
    let body_val = serde_json::to_value(&without_account).unwrap();

    // Default merge=true: deep-merge preserves the account — confirms
    // the pre-existing safeguard still holds.
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        state_with_path
            .config_snapshot()
            .channels
            .plugins
            .get("sample_plugin")
            .map(|cfg| cfg.accounts.contains_key("bot-to-delete"))
            .unwrap_or(false),
        "default merge=true must preserve the account"
    );

    // merge=false: the caller asserts a full-document replace, so the
    // account should actually be gone.
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings?merge=false")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        !state_with_path
            .config_snapshot()
            .channels
            .plugins
            .get("sample_plugin")
            .map(|cfg| cfg.accounts.contains_key("bot-to-delete"))
            .unwrap_or(false),
        "merge=false must let the client delete the account"
    );

    let file_content = tokio::fs::read_to_string(&config_path).await.unwrap();
    let persisted: GaryxConfig = serde_json::from_str(&file_content).unwrap();
    assert!(
        !persisted
            .channels
            .plugins
            .get("sample_plugin")
            .map(|cfg| cfg.accounts.contains_key("bot-to-delete"))
            .unwrap_or(false),
        "deletion must be persisted to disk, not just runtime"
    );
}

#[tokio::test]
async fn test_settings_update_rejects_missing_or_null_channel_account_config() {
    use garyx_models::config::{PluginAccountEntry, PluginChannelConfig};

    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    let mut initial = GaryxConfig::default();
    let mut plugin_cfg = PluginChannelConfig::default();
    plugin_cfg.accounts.insert(
        "bot".to_owned(),
        PluginAccountEntry {
            enabled: true,
            name: Some("Test Bot".to_owned()),
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
            config: serde_json::json!({ "token": "test-token" }),
        },
    );
    initial
        .channels
        .plugins
        .insert("sample_plugin".to_owned(), plugin_cfg);
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    state_with_path
        .apply_runtime_config(initial.clone())
        .await
        .unwrap();
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    let mut missing_config = serde_json::to_value(&initial).unwrap();
    missing_config["channels"]["sample_plugin"]["accounts"]["bot"]
        .as_object_mut()
        .unwrap()
        .remove("config");

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings?merge=false")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&missing_config).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["errors"].as_array().unwrap().iter().any(|error| {
        error
            .as_str()
            .unwrap_or_default()
            .contains("$.channels.sample_plugin.accounts.bot.config is required")
    }));

    let mut null_config = serde_json::to_value(&initial).unwrap();
    null_config["channels"]["sample_plugin"]["accounts"]["bot"]["config"] = Value::Null;

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings?merge=false")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&null_config).unwrap()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let file_content = tokio::fs::read_to_string(&config_path).await.unwrap();
    let persisted: Value = serde_json::from_str(&file_content).unwrap();
    assert_eq!(
        persisted["channels"]["sample_plugin"]["accounts"]["bot"]["config"]["token"], "test-token",
        "rejected settings updates must not overwrite existing account credentials"
    );
}

#[tokio::test]
async fn test_settings_update_rejects_blank_builtin_account_required_config() {
    let state = test_state();
    let router = api_router(state);

    let body_val = json!({
        "channels": {
            "feishu": {
                "accounts": {
                    "main": {
                        "enabled": true,
                        "config": {
                            "app_id": "",
                            "app_secret": "",
                            "domain": "feishu"
                        }
                    }
                }
            }
        }
    });

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings?merge=false")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(errors.iter().any(|error| {
        error
            .as_str()
            .unwrap_or_default()
            .contains("$.channels.feishu.accounts.main.config.app_id must not be blank")
    }));
    assert!(errors.iter().any(|error| {
        error
            .as_str()
            .unwrap_or_default()
            .contains("$.channels.feishu.accounts.main.config.app_secret must not be blank")
    }));
}

#[tokio::test]
async fn test_settings_update_merge_true_ignores_untouched_legacy_account_config_errors() {
    use garyx_models::config::{PluginAccountEntry, PluginChannelConfig};

    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    let mut initial = GaryxConfig::default();
    let mut weixin_cfg = PluginChannelConfig::default();
    weixin_cfg.accounts.insert(
        "test-weixin".to_owned(),
        PluginAccountEntry {
            enabled: true,
            name: Some("Test Weixin".to_owned()),
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
            config: json!({
                "token": "test-token",
                "uin": ""
            }),
        },
    );
    initial
        .channels
        .plugins
        .insert("weixin".to_owned(), weixin_cfg);
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    state_with_path
        .apply_runtime_config(initial.clone())
        .await
        .unwrap();
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings?merge=true")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "gateway": {
                    "public_url": "https://garyx.example"
                }
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let live = state_with_path.config_snapshot();
    assert_eq!(live.gateway.public_url, "https://garyx.example");

    let persisted: Value =
        serde_json::from_str(&tokio::fs::read_to_string(&config_path).await.unwrap()).unwrap();
    assert_eq!(persisted["gateway"]["public_url"], "https://garyx.example");
    assert_eq!(
        persisted["channels"]["weixin"]["accounts"]["test-weixin"]["config"]["uin"],
        ""
    );
}

#[tokio::test]
async fn test_settings_update_merge_true_rejects_touched_legacy_account_config_errors() {
    use garyx_models::config::{PluginAccountEntry, PluginChannelConfig};

    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    let mut initial = GaryxConfig::default();
    let mut weixin_cfg = PluginChannelConfig::default();
    weixin_cfg.accounts.insert(
        "test-weixin".to_owned(),
        PluginAccountEntry {
            enabled: true,
            name: Some("Test Weixin".to_owned()),
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
            config: json!({
                "token": "test-token",
                "uin": ""
            }),
        },
    );
    initial
        .channels
        .plugins
        .insert("weixin".to_owned(), weixin_cfg);
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    state_with_path
        .apply_runtime_config(initial.clone())
        .await
        .unwrap();
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings?merge=true")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "channels": {
                    "weixin": {
                        "accounts": {
                            "test-weixin": {
                                "enabled": false
                            }
                        }
                    }
                },
                "gateway": {
                    "public_url": "https://garyx.example"
                }
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["errors"].as_array().unwrap().iter().any(|error| {
        error
            .as_str()
            .unwrap_or_default()
            .contains("$.channels.weixin.accounts.test-weixin.config.uin must not be blank")
    }));
    assert!(
        state_with_path
            .config_snapshot()
            .gateway
            .public_url
            .is_empty(),
        "rejected patch must not touch the live config"
    );
}

#[tokio::test]
async fn test_settings_reload_applies_config_from_disk() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    let mut initial = GaryxConfig::default();
    initial.gateway.port = 31337;
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    state_with_path
        .apply_runtime_config(initial.clone())
        .await
        .unwrap();
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    let mut updated = initial.clone();
    updated.gateway.port = 42424;
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&updated).unwrap())
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/settings/reload")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["message"], "config reloaded");
    assert_eq!(state_with_path.config_snapshot().gateway.port, 42424);
}

#[tokio::test]
async fn test_settings_update_partial_payload_preserves_existing_sections() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    let mut initial = GaryxConfig::default();
    initial.gateway.port = 4242;
    initial.gateway.search.api_key = "search-secret".to_owned();
    initial
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "telegram-secret".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: None,
                    owner_target: None,
                    groups: std::collections::HashMap::new(),
                },
            ),
        );

    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    state_with_path
        .apply_runtime_config(initial.clone())
        .await
        .unwrap();
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    let partial_update = json!({
        "commands": [],
    });

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&partial_update).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let file_content = tokio::fs::read_to_string(&config_path).await.unwrap();
    let persisted: GaryxConfig = serde_json::from_str(&file_content).unwrap();
    assert_eq!(persisted.commands.len(), 0);
    assert_eq!(persisted.gateway.port, 4242);
    assert_eq!(persisted.gateway.search.api_key, "search-secret");
    let telegram = persisted
        .channels
        .plugins
        .get("telegram")
        .and_then(|channel| channel.accounts.get("main"))
        .unwrap();
    assert_eq!(telegram.config["token"], "telegram-secret");
}

#[tokio::test]
async fn test_settings_update_strips_legacy_agent_defaults() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    tokio::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&GaryxConfig::default()).unwrap(),
    )
    .await
    .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    let mut body_val = serde_json::to_value(GaryxConfig::default()).unwrap();
    body_val["agent_defaults"] = json!({
        "workspace_dir": "~/gary"
    });
    body_val["gateway"]["port"] = json!(4242);

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let persisted =
        serde_json::from_str::<Value>(&tokio::fs::read_to_string(&config_path).await.unwrap())
            .unwrap();
    assert_eq!(persisted["gateway"]["port"], 4242);
    assert!(persisted.get("agent_defaults").is_none());

    let live = serde_json::to_value(state_with_path.config_snapshot()).unwrap();
    assert!(live.get("agent_defaults").is_none());
}

#[tokio::test]
async fn test_restart_ok() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_restart_cooldown() {
    let state = test_state();

    // Simulate a recent restart
    {
        let mut tracker = state.ops.restart_tracker.lock().await;
        tracker.last_restart = Some(Instant::now());
    }

    let router = api_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 429);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["reason"], "cooldown");
}

#[tokio::test]
async fn test_restart_auth_required_no_token() {
    let state = test_state();
    // Create state with auth tokens configured
    let mut state_with_auth = (*state).clone_for_test();
    state_with_auth.ops.restart_tokens = vec!["secret-token-123".to_owned()];
    let state_with_auth = Arc::new(state_with_auth);

    let router = api_router(state_with_auth);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 403);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["reason"], "unauthorized");
}

#[tokio::test]
async fn test_restart_unauthorized_contract_shape() {
    let state = test_state();
    let mut state_with_auth = (*state).clone_for_test();
    state_with_auth.ops.restart_tokens = vec!["secret-token-123".to_owned()];
    let state_with_auth = Arc::new(state_with_auth);

    let router = api_router(state_with_auth);
    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 403);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let got: Value = serde_json::from_slice(&body).unwrap();
    let expected = json!({
        "ok": false,
        "reason": "unauthorized",
        "message": "valid authorization token required for restart",
    });
    assert_eq!(got, expected);
}

#[tokio::test]
async fn test_settings_unknown_field_contract_shape() {
    let state = test_state();
    let router = api_router(state);

    let mut body_val = serde_json::to_value(GaryxConfig::default()).unwrap();
    body_val["unknown_top_level"] = json!(1);

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let got: Value = serde_json::from_slice(&body).unwrap();
    let expected = json!({
        "ok": false,
        "errors": ["unknown field: $.unknown_top_level"],
    });
    assert_eq!(got, expected);
}

#[tokio::test]
async fn test_restart_auth_required_wrong_token() {
    let state = test_state();
    let mut state_with_auth = (*state).clone_for_test();
    state_with_auth.ops.restart_tokens = vec!["secret-token-123".to_owned()];
    let state_with_auth = Arc::new(state_with_auth);

    let router = api_router(state_with_auth);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .header("authorization", "Bearer wrong-token")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 403);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["reason"], "unauthorized");
}

#[tokio::test]
async fn test_restart_auth_required_valid_token() {
    let state = test_state();
    let mut state_with_auth = (*state).clone_for_test();
    state_with_auth.ops.restart_tokens = vec!["secret-token-123".to_owned()];
    let state_with_auth = Arc::new(state_with_auth);

    let router = api_router(state_with_auth);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .header("authorization", "Bearer secret-token-123")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_restart_no_auth_when_tokens_empty() {
    // No restart tokens configured = restart endpoint auth is not required.
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_thread_history_detail_with_thread_id() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::u3",
        json!({
            "messages": [
                { "role": "user", "content": "hello" }
            ]
        }),
    )
    .await;

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Au3&limit=10")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["thread"]["thread_id"], "thread::u3");
    assert_eq!(json["thread"]["thread_key"], "thread::u3");
    assert_eq!(json["session"]["thread_id"], "thread::u3");
    assert_eq!(json["session"]["thread_key"], "thread::u3");
    assert_eq!(json["message_stats"]["total_messages_in_thread"], 1);
    assert_eq!(json["message_stats"]["total_messages_in_session"], 1);
}

#[tokio::test]
async fn test_thread_history_detail_exposes_internal_loop_markers() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::loop-view",
        json!({
            "messages": [
                {
                    "role": "user",
                    "content": "The user wants you to continue working.",
                    "timestamp": "2026-03-15T10:00:00Z",
                    "internal": true,
                    "internal_kind": "loop_continuation",
                    "loop_origin": "auto_continue"
                },
                {
                    "role": "assistant",
                    "content": "当前没有剩余代码任务。",
                    "timestamp": "2026-03-15T10:00:02Z",
                    "internal": true,
                    "internal_kind": "loop_continuation",
                    "loop_origin": "auto_continue"
                }
            ]
        }),
    )
    .await;

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Aloop-view&limit=10")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["messages"][0]["internal"], true);
    assert_eq!(json["messages"][0]["internal_kind"], "loop_continuation");
    assert_eq!(json["messages"][0]["loop_origin"], "auto_continue");
    assert_eq!(json["messages"][1]["internal"], true);
    assert_eq!(json["messages"][1]["internal_kind"], "loop_continuation");
    assert_eq!(json["messages"][1]["loop_origin"], "auto_continue");
}

#[test]
fn enrich_message_content_for_history_inlines_image_path_blocks() {
    let temp = tempdir().expect("tempdir");
    let image_path = temp.path().join("probe.png");
    std::fs::write(&image_path, b"png-bytes").expect("write image");

    let enriched = enrich_message_content_for_history(&json!([{
        "type": "image",
        "path": image_path.to_string_lossy().to_string(),
        "name": "probe.png",
        "media_type": "image/png"
    }]));
    let blocks = enriched.as_array().expect("array content");
    let source = blocks[0]
        .get("source")
        .and_then(Value::as_object)
        .expect("inline image source");
    assert_eq!(source.get("type").and_then(Value::as_str), Some("base64"));
    assert_eq!(
        source.get("media_type").and_then(Value::as_str),
        Some("image/png")
    );
    assert!(
        source
            .get("data")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    );
}

#[test]
fn enrich_history_preserves_long_string_content() {
    let long = "x".repeat(20_000);
    let enriched = enrich_message_content_for_history(&json!(long.clone()));
    let value = enriched.as_str().expect("string content");
    assert_eq!(value.chars().count(), long.chars().count());
    assert_eq!(value, long);
}

#[test]
fn enrich_history_keeps_short_string_content() {
    let enriched = enrich_message_content_for_history(&json!("hello world"));
    assert_eq!(enriched.as_str(), Some("hello world"));
}

#[test]
fn enrich_history_preserves_text_inside_blocks() {
    let long = "y".repeat(20_000);
    let enriched =
        enrich_message_content_for_history(&json!([{ "type": "text", "text": long.clone() }]));
    let text = enriched[0]["text"].as_str().expect("block text");
    assert_eq!(text.chars().count(), long.chars().count());
    assert_eq!(text, long);
}

#[test]
fn enrich_history_preserves_text_nested_in_object() {
    let long = "z".repeat(20_000);
    let enriched = enrich_message_content_for_history(&json!({
        "type": "tool_result",
        "content": [{ "type": "text", "text": long.clone() }],
    }));
    let text = enriched["content"][0]["text"]
        .as_str()
        .expect("nested text");
    assert_eq!(text.chars().count(), long.chars().count());
    assert_eq!(text, long);
}

#[test]
fn enrich_history_does_not_cap_image_base64() {
    let temp = tempdir().expect("tempdir");
    let image_path = temp.path().join("big.png");
    std::fs::write(&image_path, vec![0u8; 30_000]).expect("write image");

    let enriched = enrich_message_content_for_history(&json!([{
        "type": "image",
        "path": image_path.to_string_lossy().to_string(),
        "media_type": "image/png"
    }]));
    let data = enriched[0]["source"]["data"]
        .as_str()
        .expect("inline base64 data");
    assert!(
        data.chars().count() > 30_000,
        "image base64 must not be truncated, got {} chars",
        data.chars().count()
    );
}

#[test]
fn humanize_structured_content_mentions_file_blocks() {
    let summary = humanize_structured_content(&json!([
        {
            "type": "text",
            "text": "Please inspect the attachment."
        },
        {
            "type": "file",
            "path": "/tmp/report.pdf",
            "name": "report.pdf"
        }
    ]))
    .expect("summary");
    assert!(summary.contains("Please inspect the attachment."));
    assert!(summary.contains("[File] report.pdf"));
}
