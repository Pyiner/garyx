use super::*;
use crate::app_bootstrap::{AppStateBuilder, create_app_state};
use axum::body::Body;
use garyx_models::config::{GaryxConfig, TelegramAccount};
use garyx_router::{InMemoryThreadStore, ThreadStore};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

fn test_state() -> Arc<AppState> {
    create_app_state(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
}

#[tokio::test]
async fn test_live_config_cell_supports_concurrent_snapshot_and_replace() {
    let state = test_state();
    let writer = {
        let state = state.clone();
        tokio::spawn(async move {
            for i in 0..500_u16 {
                let mut cfg = (*state.config_snapshot()).clone();
                cfg.gateway.port = if i % 2 == 0 { 4100 } else { 4200 };
                state.replace_config(cfg);
                tokio::task::yield_now().await;
            }
        })
    };

    let mut readers = Vec::new();
    for _ in 0..8 {
        let state = state.clone();
        readers.push(tokio::spawn(async move {
            for _ in 0..500 {
                let cfg = state.config_snapshot();
                assert!(
                    cfg.gateway.port == 4100
                        || cfg.gateway.port == 4200
                        || cfg.gateway.port == 8080
                );
                tokio::task::yield_now().await;
            }
        }));
    }

    writer.await.unwrap();
    for reader in readers {
        reader.await.unwrap();
    }
}

#[tokio::test]
async fn test_apply_runtime_config_reconciles_bridge_and_dispatcher() {
    let state = test_state();
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&TelegramAccount {
                token: "token".to_owned(),
                enabled: true,
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                groups: HashMap::new(),
            }),
        );

    state.apply_runtime_config(config.clone()).await.unwrap();
    let resolved_enabled = state
        .integration
        .bridge
        .resolve_provider_for_thread("sess::x", "telegram", "main")
        .await;
    assert!(resolved_enabled.is_some());
    let channels = state.channel_dispatcher().available_channels();
    assert!(
        channels
            .iter()
            .any(|item| item.channel == "telegram" && item.account_id == "main")
    );

    config
        .channels
        .plugins
        .get_mut("telegram")
        .unwrap()
        .accounts
        .get_mut("main")
        .unwrap()
        .enabled = false;
    state.apply_runtime_config(config).await.unwrap();

    let resolved_disabled = state
        .integration
        .bridge
        .resolve_provider_for_thread("sess::x", "telegram", "main")
        .await;
    let default_key = state.integration.bridge.default_provider_key().await;
    assert_eq!(resolved_disabled, default_key);
    let channels = state.channel_dispatcher().available_channels();
    assert!(
        !channels
            .iter()
            .any(|item| item.channel == "telegram" && item.account_id == "main")
    );
}

#[tokio::test]
async fn test_event_stream_hub_records_history() {
    let state = test_state();

    tokio::time::timeout(std::time::Duration::from_millis(200), async {
        loop {
            if state
                .ops
                .events
                .sender()
                .send(r#"{"type":"test-event"}"#.to_owned())
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("event sender should get an active subscriber");

    tokio::time::timeout(std::time::Duration::from_millis(200), async {
        loop {
            if state.ops.events.history_len().await >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("history recorder should ingest events");

    let snapshot = state.ops.events.history_snapshot(10).await;
    assert!(snapshot.iter().any(|event| event.contains("test-event")));
}

#[tokio::test]
async fn test_app_state_builder_shares_thread_store() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());

    store
        .set(
            "thread::test-key",
            serde_json::json!({
                "thread_id": "thread::test-key",
                "msg": "hello"
            }),
        )
        .await;

    let bridge = Arc::new(garyx_bridge::MultiProviderBridge::new());
    let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
    .with_thread_store(store)
    .with_bridge(bridge)
    .build();

    let router = crate::route_graph::build_router(state.clone());
    let req = crate::test_support::authed_request()
        .uri("/api/threads/thread::test-key")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["thread_id"], "thread::test-key");

    let mut router = state.threads.router.lock().await;
    let resolved = router
        .resolve_or_create_inbound_thread("api", "main", "api-user", &HashMap::new())
        .await;
    drop(router);
    assert!(resolved.starts_with("thread::"));
    assert_eq!(
        state
            .threads
            .thread_store
            .get("thread::test-key")
            .await
            .unwrap()["msg"],
        "hello"
    );
}
