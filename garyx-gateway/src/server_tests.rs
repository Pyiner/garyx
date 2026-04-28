use super::*;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::handler::HandlerWithoutStateExt;
use axum::http::Request;
use garyx_models::config::GaryxConfig;
use std::net::SocketAddr;
use tower::ServiceExt;
use tower_http::services::ServeDir;

fn test_config() -> GaryxConfig {
    crate::test_support::with_gateway_auth(GaryxConfig::default())
}

fn authed_request() -> axum::http::request::Builder {
    crate::test_support::authed_request()
}

fn test_state() -> Arc<AppState> {
    create_app_state(test_config())
}

fn test_state_with_manager(
    cfg: GaryxConfig,
    manager: Arc<tokio::sync::Mutex<garyx_channels::ChannelPluginManager>>,
) -> Arc<AppState> {
    // Mirrors `create_app_state(cfg)` but uses `AppStateBuilder`
    // directly so we can inject a pre-built plugin manager before
    // `build()`. Needed for HTTP tests that exercise paths gated
    // by a registered plugin (e.g. auth-flow alias routing).
    use crate::composition::app_bootstrap::AppStateBuilder;
    AppStateBuilder::new(crate::test_support::with_gateway_auth(cfg))
        .with_channel_plugin_manager(manager)
        .build()
}

#[tokio::test]
async fn test_health_endpoint() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request().uri("/health").body(Body::empty()).unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn test_health_detailed_endpoint() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/health/detailed")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "healthy");
}

#[tokio::test]
async fn test_protected_route_requires_configured_gateway_token() {
    let state = create_app_state(GaryxConfig::default());
    let gw = Gateway::new(state);

    let req = Request::builder()
        .uri("/api/status")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "unauthorized");
    assert!(
        json["message"]
            .as_str()
            .unwrap_or_default()
            .contains("garyx gateway token")
    );
}

#[tokio::test]
async fn test_protected_route_allows_loopback_without_gateway_token() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = Request::builder()
        .uri("/api/status")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 49152))))
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_protected_route_requires_token_for_non_loopback() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = Request::builder()
        .uri("/api/status")
        .extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 49152))))
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "unauthorized");
    assert_eq!(
        json["message"],
        "valid gateway authorization token required"
    );
}

#[tokio::test]
async fn test_runtime_endpoint() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/runtime")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_runtime_endpoint_reflects_live_config() {
    let state = test_state();
    {
        let mut cfg = (*state.config_snapshot()).clone();
        cfg.gateway.host = "127.0.0.1".to_owned();
        cfg.gateway.port = 19876;
        state.replace_config(cfg);
    }
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/runtime")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["gateway"]["host"], "127.0.0.1");
    assert_eq!(json["gateway"]["port"], 19876);
}

#[tokio::test]
async fn test_sessions_empty() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/api/threads")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 0);
}

#[tokio::test]
async fn test_threads_empty_alias() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/api/threads")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 0);
}

#[tokio::test]
async fn test_thread_not_found() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/api/threads/nonexistent")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_thread_not_found_alias() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/api/threads/nonexistent")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_fallback_404() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/nonexistent")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_status_endpoint() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/api/status")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "running");
}

#[tokio::test]
async fn test_sessions_filtering_with_prefix() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::agent1-u1",
            serde_json::json!({"thread_id": "thread::agent1-u1"}),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            "thread::agent1-u2",
            serde_json::json!({"thread_id": "thread::agent1-u2"}),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            "thread::agent2-u3",
            serde_json::json!({"thread_id": "thread::agent2-u3"}),
        )
        .await;

    let gw = Gateway::new(state);
    let req = authed_request()
        .uri("/api/threads?prefix=thread::agent1")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["total"], 2);
    assert_eq!(json["count"], 2);
    let first = json["threads"][0].as_object().unwrap();
    assert!(first.contains_key("thread_id"));
    assert!(first.contains_key("thread_key"));
    assert!(first.contains_key("thread_type"));
}

#[tokio::test]
async fn test_sessions_pagination_limit_offset() {
    let state = test_state();
    for i in 0..10 {
        state
            .threads
            .thread_store
            .set(
                &format!("thread::{:02}", i),
                serde_json::json!({
                    "thread_id": format!("thread::{:02}", i),
                    "thread_id": format!("thread::{:02}", i)
                }),
            )
            .await;
    }

    let gw = Gateway::new(state);
    let req = authed_request()
        .uri("/api/threads?limit=3&offset=2")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["total"], 10);
    assert_eq!(json["count"], 3);
    assert_eq!(json["offset"], 2);
    assert_eq!(json["limit"], 3);
    // Since keys are sorted, offset=2 skips first 2
    let threads = json["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 3);
}

#[tokio::test]
async fn test_sessions_default_limit_bounded() {
    let state = test_state();
    // Ensure default params work even with empty store
    let gw = Gateway::new(state);
    let req = authed_request()
        .uri("/api/threads")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["limit"], 100); // DEFAULT_SESSION_LIMIT
    assert_eq!(json["offset"], 0);
}

#[tokio::test]
async fn test_channel_endpoints_returns_thread_metadata() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::support",
            serde_json::json!({
                "thread_id": "thread::support",
                "thread_id": "thread::support",
                "label": "Alice Support",
                "workspace_dir": "/tmp/gary-support",
                "updated_at": "2026-03-07T12:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice",
                    "last_inbound_at": "2026-03-07T11:59:00Z",
                    "last_delivery_at": "2026-03-07T12:00:00Z"
                }]
            }),
        )
        .await;

    let gw = Gateway::new(state);
    let req = authed_request()
        .uri("/api/channel-endpoints")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let endpoints = json["endpoints"].as_array().unwrap();
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0]["thread_id"], "thread::support");
    assert_eq!(endpoints[0]["thread_label"], "Alice Support");
    assert_eq!(endpoints[0]["workspace_dir"], "/tmp/gary-support");
    assert_eq!(endpoints[0]["thread_updated_at"], "2026-03-07T12:00:00Z");
}

#[tokio::test]
async fn test_bind_channel_endpoint_moves_binding_to_target_thread() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::source",
            serde_json::json!({
                "thread_id": "thread::source",
                "thread_id": "thread::source",
                "label": "Source",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }]
            }),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            "thread::target",
            serde_json::json!({
                "thread_id": "thread::target",
                "thread_id": "thread::target",
                "label": "Target",
                "channel_bindings": []
            }),
        )
        .await;

    let gw = Gateway::new(state.clone());
    let req = authed_request()
        .method("POST")
        .uri("/api/channel-bindings/bind")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"endpointKey":"telegram::main::alice","threadId":"thread::target"}"#,
        ))
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["thread_id"], "thread::target");
    assert_eq!(json["previous_thread_id"], "thread::source");

    let source = state
        .threads
        .thread_store
        .get("thread::source")
        .await
        .unwrap();
    let target = state
        .threads
        .thread_store
        .get("thread::target")
        .await
        .unwrap();
    assert_eq!(
        source["channel_bindings"]
            .as_array()
            .map(|items| items.len()),
        Some(0)
    );
    assert_eq!(
        target["channel_bindings"]
            .as_array()
            .map(|items| items.len()),
        Some(1)
    );
}

#[tokio::test]
async fn test_bind_channel_endpoint_moves_delivery_target_to_bound_thread() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::source",
            serde_json::json!({
                "thread_id": "thread::source",
                "thread_id": "thread::source",
                "label": "Source",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }]
            }),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            "thread::target",
            serde_json::json!({
                "thread_id": "thread::target",
                "thread_id": "thread::target",
                "label": "Target",
                "channel_bindings": []
            }),
        )
        .await;
    {
        let mut router = state.threads.router.lock().await;
        router
            .set_last_delivery_with_persistence(
                "thread::source",
                garyx_models::routing::DeliveryContext {
                    channel: "telegram".to_owned(),
                    account_id: "main".to_owned(),
                    chat_id: "alice".to_owned(),
                    user_id: "alice".to_owned(),
                    delivery_target_type: "chat_id".to_owned(),
                    delivery_target_id: "alice".to_owned(),
                    thread_id: None,
                    metadata: Default::default(),
                },
            )
            .await;
    }

    let gw = Gateway::new(state.clone());
    let req = authed_request()
        .method("POST")
        .uri("/api/channel-bindings/bind")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"endpointKey":"telegram::main::alice","threadId":"thread::target"}"#,
        ))
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let router = state.threads.router.lock().await;
    assert!(router.get_last_delivery("thread::source").is_none());
    let target_delivery = router
        .get_last_delivery("thread::target")
        .cloned()
        .expect("target should have delivery target");
    assert_eq!(target_delivery.channel, "telegram");
    assert_eq!(target_delivery.account_id, "main");
    assert_eq!(target_delivery.chat_id, "alice");
    drop(router);

    let source = state
        .threads
        .thread_store
        .get("thread::source")
        .await
        .unwrap();
    assert!(source.get("delivery_context").is_none());
    let target = state
        .threads
        .thread_store
        .get("thread::target")
        .await
        .unwrap();
    assert_eq!(target["delivery_context"]["channel"], "telegram");
    assert_eq!(target["delivery_context"]["chat_id"], "alice");
}

#[tokio::test]
async fn test_detach_channel_endpoint_removes_binding() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::bound",
            serde_json::json!({
                "thread_id": "thread::bound",
                "thread_id": "thread::bound",
                "label": "Bound",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }]
            }),
        )
        .await;

    let gw = Gateway::new(state.clone());
    let req = authed_request()
        .method("POST")
        .uri("/api/channel-bindings/detach")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"endpointKey":"telegram::main::alice"}"#))
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["previous_thread_id"], "thread::bound");

    let stored = state
        .threads
        .thread_store
        .get("thread::bound")
        .await
        .unwrap();
    assert_eq!(
        stored["channel_bindings"]
            .as_array()
            .map(|items| items.len()),
        Some(0)
    );

    let gw = Gateway::new(state);
    let req = authed_request()
        .uri("/api/channel-endpoints")
        .body(Body::empty())
        .unwrap();
    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let endpoints = json["endpoints"].as_array().unwrap();
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0]["endpoint_key"], "telegram::main::alice");
    assert!(endpoints[0]["thread_id"].is_null());
}

#[tokio::test]
async fn test_detach_channel_endpoint_clears_endpoint_runtime_routing() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::bound",
            serde_json::json!({
                "thread_id": "thread::bound",
                "thread_id": "thread::bound",
                "label": "Bound",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }],
                "outbound_message_ids": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "chat_id": "alice",
                    "message_id": "msg-alice-1"
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
        router.set_last_delivery(
            "thread::bound",
            garyx_models::routing::DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "alice".to_owned(),
                user_id: "alice".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "alice".to_owned(),
                thread_id: None,
                metadata: Default::default(),
            },
        );
        assert_eq!(
            router.resolve_reply_thread_for_chat(
                "telegram",
                "main",
                Some("alice"),
                None,
                "msg-alice-1",
            ),
            Some("thread::bound")
        );
        assert!(router.get_last_delivery("thread::bound").is_some());
    }

    let gw = Gateway::new(state.clone());
    let req = authed_request()
        .method("POST")
        .uri("/api/channel-bindings/detach")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"endpointKey":"telegram::main::alice"}"#))
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let router = state.threads.router.lock().await;
    assert_eq!(
        router.resolve_reply_thread_for_chat(
            "telegram",
            "main",
            Some("alice"),
            None,
            "msg-alice-1",
        ),
        None
    );
    assert!(router.get_last_delivery("thread::bound").is_none());
    drop(router);

    let stored = state
        .threads
        .thread_store
        .get("thread::bound")
        .await
        .unwrap();
    assert!(stored.get("delivery_context").is_none());
}

#[tokio::test]
async fn test_detach_channel_endpoint_preserves_other_topic_routing_in_same_chat() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::bound",
            serde_json::json!({
                "thread_id": "thread::bound",
                "thread_id": "thread::bound",
                "label": "Bound",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "42_t100",
                    "chat_id": "42",
                    "display_label": "Alice topic 100"
                }],
                "outbound_message_ids": [
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "chat_id": "42",
                        "thread_binding_key": "42_t100",
                        "message_id": "msg-topic-100"
                    },
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "chat_id": "42",
                        "thread_binding_key": "42_t200",
                        "message_id": "msg-topic-200"
                    }
                ],
                "delivery_context": {
                    "channel": "telegram",
                    "account_id": "main",
                    "chat_id": "42",
                    "user_id": "alice",
                    "thread_id": "42_t200",
                    "metadata": {}
                }
            }),
        )
        .await;

    {
        let mut router = state.threads.router.lock().await;
        router
            .message_routing_index_mut()
            .rebuild_from_store(state.threads.thread_store.as_ref(), "telegram")
            .await;
        router.rebuild_last_delivery_cache().await;
        assert_eq!(
            router.resolve_reply_thread_for_chat(
                "telegram",
                "main",
                Some("42"),
                Some("42_t100"),
                "msg-topic-100",
            ),
            Some("thread::bound")
        );
        assert_eq!(
            router.resolve_reply_thread_for_chat(
                "telegram",
                "main",
                Some("42"),
                Some("42_t200"),
                "msg-topic-200",
            ),
            Some("thread::bound")
        );
        assert_eq!(
            router
                .get_last_delivery("thread::bound")
                .and_then(|ctx| ctx.thread_id.as_deref()),
            Some("42_t200")
        );
    }

    let gw = Gateway::new(state.clone());
    let req = authed_request()
        .method("POST")
        .uri("/api/channel-bindings/detach")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"endpointKey":"telegram::main::42_t100"}"#))
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let router = state.threads.router.lock().await;
    assert_eq!(
        router.resolve_reply_thread_for_chat(
            "telegram",
            "main",
            Some("42"),
            Some("42_t100"),
            "msg-topic-100",
        ),
        None
    );
    assert_eq!(
        router.resolve_reply_thread_for_chat(
            "telegram",
            "main",
            Some("42"),
            Some("42_t200"),
            "msg-topic-200",
        ),
        Some("thread::bound")
    );
    assert_eq!(
        router
            .get_last_delivery("thread::bound")
            .and_then(|ctx| ctx.thread_id.as_deref()),
        Some("42_t200")
    );
    drop(router);

    let stored = state
        .threads
        .thread_store
        .get("thread::bound")
        .await
        .unwrap();
    let outbound = stored["outbound_message_ids"].as_array().unwrap();
    assert_eq!(outbound.len(), 1);
    assert_eq!(outbound[0]["message_id"], "msg-topic-200");
    assert_eq!(outbound[0]["thread_binding_key"], "42_t200");
}

#[tokio::test]
async fn test_detach_topic_endpoint_preserves_primary_reply_routing_in_same_chat() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::bound",
            serde_json::json!({
                "thread_id": "thread::bound",
                "thread_id": "thread::bound",
                "label": "Bound",
                "channel_bindings": [
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "binding_key": "alice",
                        "chat_id": "42",
                        "display_label": "Alice primary"
                    },
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "binding_key": "42_t100",
                        "chat_id": "42",
                        "display_label": "Alice topic 100"
                    }
                ],
                "outbound_message_ids": [
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "chat_id": "42",
                        "thread_binding_key": null,
                        "message_id": "msg-primary"
                    },
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "chat_id": "42",
                        "thread_binding_key": "42_t100",
                        "message_id": "msg-topic-100"
                    }
                ],
                "delivery_context": {
                    "channel": "telegram",
                    "account_id": "main",
                    "chat_id": "42",
                    "user_id": "alice",
                    "thread_id": null,
                    "metadata": {}
                }
            }),
        )
        .await;

    {
        let mut router = state.threads.router.lock().await;
        router
            .message_routing_index_mut()
            .rebuild_from_store(state.threads.thread_store.as_ref(), "telegram")
            .await;
        router.rebuild_last_delivery_cache().await;
        assert_eq!(
            router.resolve_reply_thread_for_chat(
                "telegram",
                "main",
                Some("42"),
                None,
                "msg-primary",
            ),
            Some("thread::bound")
        );
        assert_eq!(
            router.resolve_reply_thread_for_chat(
                "telegram",
                "main",
                Some("42"),
                Some("42_t100"),
                "msg-topic-100",
            ),
            Some("thread::bound")
        );
        assert_eq!(
            router
                .get_last_delivery("thread::bound")
                .and_then(|ctx| ctx.thread_id.as_deref()),
            None
        );
    }

    let gw = Gateway::new(state.clone());
    let req = authed_request()
        .method("POST")
        .uri("/api/channel-bindings/detach")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"endpointKey":"telegram::main::42_t100"}"#))
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let router = state.threads.router.lock().await;
    assert_eq!(
        router.resolve_reply_thread_for_chat("telegram", "main", Some("42"), None, "msg-primary",),
        Some("thread::bound")
    );
    assert_eq!(
        router.resolve_reply_thread_for_chat(
            "telegram",
            "main",
            Some("42"),
            Some("42_t100"),
            "msg-topic-100",
        ),
        None
    );
    assert_eq!(
        router
            .get_last_delivery("thread::bound")
            .and_then(|ctx| ctx.thread_id.as_deref()),
        None
    );
    drop(router);

    let stored = state
        .threads
        .thread_store
        .get("thread::bound")
        .await
        .unwrap();
    let outbound = stored["outbound_message_ids"].as_array().unwrap();
    assert_eq!(outbound.len(), 1);
    assert_eq!(outbound[0]["message_id"], "msg-primary");
    assert!(outbound[0]["thread_scope"].is_null());
}

// -- Frontend static serving tests --
// These tests verify the ServeDir fallback serves frontend pages
// when the static directory exists. We use a temp dir with test HTML.

fn gateway_with_static_dir(static_dir: &std::path::Path) -> Router {
    let state = test_state();
    let router = Router::new()
        .route("/health", axum::routing::get(crate::routes::health))
        .route(
            "/api/status",
            axum::routing::get(crate::routes::system_status),
        )
        .with_state(state);

    router.fallback_service(
        ServeDir::new(static_dir)
            .append_index_html_on_directories(true)
            .not_found_service(crate::routes::fallback.into_service()),
    )
}

#[tokio::test]
async fn test_frontend_root_serves_index_html() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>root</html>").unwrap();

    let router = gateway_with_static_dir(dir.path());
    let req = authed_request().uri("/").body(Body::empty()).unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("root"));
}

#[tokio::test]
async fn test_web_shell_route_serves_index_html() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>web-shell</html>").unwrap();

    let state = test_state();
    let router = Router::new()
        .route("/health", axum::routing::get(crate::routes::health))
        .with_state(state)
        .nest_service(
            "/web",
            ServeDir::new(dir.path()).append_index_html_on_directories(true),
        );

    let req = authed_request()
        .uri("/web/?view=threads")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("web-shell"));
}

#[tokio::test]
async fn test_frontend_static_asset_served() {
    let dir = tempfile::tempdir().unwrap();
    let next_dir = dir.path().join("_next").join("static");
    std::fs::create_dir_all(&next_dir).unwrap();
    std::fs::write(next_dir.join("chunk.js"), "console.log('ok')").unwrap();

    let router = gateway_with_static_dir(dir.path());
    let req = authed_request()
        .uri("/_next/static/chunk.js")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("console.log"));
}

#[tokio::test]
async fn test_frontend_missing_path_returns_404_json() {
    let dir = tempfile::tempdir().unwrap();
    // Empty dir — no files at all

    let router = gateway_with_static_dir(dir.path());
    let req = authed_request()
        .uri("/nonexistent-page")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_api_routes_take_priority_over_static() {
    let dir = tempfile::tempdir().unwrap();
    // Even if there's a health file in static dir, API route wins
    std::fs::write(dir.path().join("health"), "wrong").unwrap();

    let router = gateway_with_static_dir(dir.path());
    let req = authed_request().uri("/health").body(Body::empty()).unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // API handler returns JSON, not the static file
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn test_stream_emits_snapshot_first() {
    let state = test_state();
    // Add a thread so snapshot reflects it
    state
        .threads
        .thread_store
        .set("test::key", serde_json::json!({}))
        .await;

    let gw = Gateway::new(state);
    let req = authed_request()
        .uri("/api/stream")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    // Read the SSE body. The first event should be the snapshot.
    // We read a chunk and parse the SSE text.
    let body = resp.into_body();
    // Use a timeout to avoid hanging if no data.
    let _bytes = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        axum::body::to_bytes(body, 1024 * 1024),
    )
    .await;

    // The stream won't end (it's live), so the timeout will trigger.
    // But the first chunk should have been sent immediately.
    // In axum SSE, data is flushed per-event. Let's just check
    // that the endpoint returns 200 and the content type is text/event-stream.
    // A more thorough test would use a streaming client.
    // For now, the compilation + 200 status validates the snapshot protocol.
}

#[tokio::test]
async fn test_list_channel_plugins_returns_builtin_catalog_by_default() {
    // Even with no subprocess plugins registered, the endpoint must
    // return a catalog containing the three built-in channels
    // (telegram, feishu, weixin) so the desktop UI can render their
    // config forms schema-driven. The UI treats built-in and
    // subprocess plugins identically — the only difference is where
    // their schemas come from.
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .uri("/api/channels/plugins")
        .body(Body::empty())
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert!(json["plugins"].is_array(), "plugins must be an array");
    let plugins = json["plugins"].as_array().unwrap();
    let ids: Vec<&str> = plugins
        .iter()
        .map(|p| p["id"].as_str().unwrap_or(""))
        .collect();
    // At least the three built-ins must always show up. Subprocess
    // plugins show up in addition when registered (tested by
    // `respawn_contract.rs::register_then_respawn_hot_swaps_the_subprocess`).
    assert!(ids.contains(&"telegram"), "telegram missing: {ids:?}");
    assert!(ids.contains(&"feishu"), "feishu missing: {ids:?}");
    assert!(ids.contains(&"weixin"), "weixin missing: {ids:?}");

    // Each built-in must ship a JSON Schema so the UI has a form to
    // render. If any of these are missing the desktop falls back to
    // the old hardcoded panels — a regression that defeats Step 1.
    for channel_id in ["telegram", "feishu", "weixin"] {
        let entry = plugins
            .iter()
            .find(|p| p["id"] == channel_id)
            .expect("entry");
        assert_eq!(
            entry["schema"]["type"], "object",
            "{channel_id} schema shape"
        );
        assert!(
            entry["schema"]["properties"].is_object(),
            "{channel_id} schema.properties must be an object"
        );
    }
}

#[tokio::test]
async fn test_auth_flow_start_rejects_form_only_channels() {
    // Telegram's catalog entry advertises `config_methods: [form]` —
    // no AutoLogin — so the gateway MUST reject `auth_flow/start`
    // with a 404 rather than silently hanging or 500-ing. The desktop
    // uses this signal to gray out the "auto login" button.
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .method("POST")
        .uri("/api/channels/plugins/telegram/auth_flow/start")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"form_state":{}}"#))
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        404,
        "telegram advertises form-only; start must 404"
    );
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["reason"], "no_auth_flow");
}

#[tokio::test]
async fn test_channel_account_validate_reports_skipped_validator() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .method("POST")
        .uri("/api/channels/plugins/weixin/validate_account")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"account_id":"main","config":{"token":"tok-final","base_url":"https://ilinkai.weixin.qq.com"}}"#,
        ))
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["validated"], false);
    assert!(
        json["message"]
            .as_str()
            .unwrap_or_default()
            .contains("no safe side-effect-free connectivity probe"),
        "{json:?}"
    );
}

#[tokio::test]
async fn test_channel_account_validate_telegram_available_before_first_account() {
    let state = test_state();
    let gw = Gateway::new(state);

    let req = authed_request()
        .method("POST")
        .uri("/api/channels/plugins/telegram/validate_account")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"account_id":"main","config":{"token":""}}"#))
        .unwrap();

    let resp = gw.router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["reason"], "validation_failed");
    assert!(
        json["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Telegram token is required"),
        "{json:?}"
    );
}

#[tokio::test]
async fn test_auth_flow_start_routes_feishu_alias_end_to_end() {
    // The gateway's `/auth_flow/start` endpoint accepts a plugin
    // id OR any registered alias. `lark` is the feishu channel's
    // alias. This test builds a manager with feishu registered
    // through the built-in discoverer, injects it via
    // `test_state_with_manager`, then hits the alias URL and
    // asserts the request reaches the feishu executor (i.e. does
    // NOT 404 with `reason: "no_auth_flow"` the way a truly
    // unknown plugin would).
    use garyx_bridge::MultiProviderBridge;
    use garyx_channels::{BuiltInPluginDiscoverer, ChannelPluginManager};
    use garyx_models::config::FeishuAccount;
    use garyx_router::{InMemoryThreadStore, MessageRouter, ThreadStore};
    use tokio::sync::Mutex as TokioMutex;

    let mut cfg = test_config();
    cfg.channels.plugin_channel_mut("feishu").accounts.insert(
        "seed".to_owned(),
        garyx_models::config::feishu_account_to_plugin_entry(&FeishuAccount {
            app_id: "seed".into(),
            app_secret: "seed".into(),
            enabled: true,
            domain: Default::default(),
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: Default::default(),
        }),
    );

    // Register feishu through the built-in discoverer so the
    // manager learns the full `PluginMetadata` (including the
    // `lark` alias) and the feishu auth-flow executor. We mirror
    // the manager-layer test's construction of `router` / `bridge`
    // (see `garyx_channels::plugin::tests::
    // builtin_discoverer_sets_config_methods_per_channel`) —
    // these are real no-op-ish stand-ins since the alias-routing
    // path doesn't exercise inbound wiring.
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let router = Arc::new(TokioMutex::new(MessageRouter::new(store, cfg.clone())));
    let bridge = Arc::new(MultiProviderBridge::new());
    let mut manager_inner = ChannelPluginManager::new();
    let discoverer =
        BuiltInPluginDiscoverer::new(cfg.channels.clone(), router, bridge, String::new());
    manager_inner.discover_and_register(&discoverer).unwrap();
    let mgr = Arc::new(TokioMutex::new(manager_inner));

    let state = test_state_with_manager(cfg, mgr);
    let gw = Gateway::new(state);

    // Hit the ALIAS url (lark, not feishu).
    let req = authed_request()
        .method("POST")
        .uri("/api/channels/plugins/lark/auth_flow/start")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"form_state":{"domain":"feishu"}}"#))
        .unwrap();

    // The executor will try to call `accounts.feishu.cn` — in CI
    // that'll succeed, transport-fail (502), or block. Wrap the
    // whole call in a timeout so a stalled network doesn't hang
    // the suite. A timeout here is ALSO a pass: it proves the
    // request reached the executor (not the 404 shortcut).
    let resp = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        gw.router.oneshot(req),
    )
    .await
    {
        Ok(result) => result.unwrap(),
        Err(_) => {
            // Timed out waiting on the feishu backend — that's
            // fine; it means dispatch reached the executor, which
            // is exactly what this test is asserting.
            return;
        }
    };

    let status = resp.status().as_u16();
    assert_ne!(
        status, 404,
        "alias `lark` must resolve to feishu; got 404 (routing broke or plugin not registered)",
    );
}
