use super::*;
use crate::app_bootstrap::{AppStateBuilder, create_app_state};
use async_trait::async_trait;
use axum::body::Body;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_models::config::{GaryxConfig, TelegramAccount};
use garyx_models::provider::{
    AgentRunRequest, ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent,
};
use garyx_router::{InMemoryThreadStore, ThreadStore};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Notify;
use tower::ServiceExt;

fn test_state() -> Arc<AppState> {
    create_app_state(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
}

struct HoldingProvider {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl AgentLoopProvider for HoldingProvider {
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
            text: "partial reply".to_owned(),
        });
        self.started.notify_waiters();
        self.release.notified().await;
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "provider-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "partial reply".to_owned(),
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

#[tokio::test]
async fn test_app_state_builder_wires_bridge_thread_store_for_recent_projection() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider(
            "projection-provider",
            Arc::new(HoldingProvider {
                started: started.clone(),
                release: release.clone(),
            }),
        )
        .await;
    bridge.set_default_provider_key("projection-provider").await;

    let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
    .with_bridge(bridge.clone())
    .build();

    bridge
        .start_agent_run(
            AgentRunRequest::new(
                "thread::recent-projection-run",
                "keep projection current",
                "run::recent-projection",
                "api",
                "main",
                HashMap::new(),
            ),
            None,
        )
        .await
        .expect("run should start");

    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .expect("provider should stream a partial reply");

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let records = state
                .ops
                .garyx_db
                .list_recent_threads(10, 0)
                .expect("list recent threads");
            if records.iter().any(|record| {
                record.thread_id == "thread::recent-projection-run"
                    && record.active_run_id.as_deref() == Some("run::recent-projection")
                    && record.run_state == "running"
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("partial persistence should project active run state");

    release.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run::recent-projection").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run should complete");

    let records = state
        .ops
        .garyx_db
        .list_recent_threads(10, 0)
        .expect("list recent threads after completion");
    let record = records
        .iter()
        .find(|record| record.thread_id == "thread::recent-projection-run")
        .expect("projected recent thread should exist");
    assert!(record.active_run_id.is_none());
    assert_eq!(record.run_state, "completed");
}

#[tokio::test]
async fn startup_repair_clears_stale_active_run_snapshot_but_keeps_live_runs() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider(
            "holding-provider",
            Arc::new(HoldingProvider {
                started: started.clone(),
                release: release.clone(),
            }),
        )
        .await;
    bridge.set_default_provider_key("holding-provider").await;

    let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
    .with_bridge(bridge.clone())
    .build();

    // (a) A thread whose active_run_snapshot references a run that is NOT live —
    // the shape an abandoned run (gateway restart / a run that never reached its
    // terminal) leaves behind, surfaced as a phantom "running"/tail "Thinking".
    state
        .threads
        .thread_store
        .set(
            "thread::stale",
            serde_json::json!({
                "thread_id": "thread::stale",
                "label": "Stale",
                "updated_at": "2026-01-01T00:00:01Z",
                "history": {
                    "active_run_snapshot": { "run_id": "abandoned-run-1" },
                    "recent_committed_run_ids": ["earlier-run"]
                }
            }),
        )
        .await;

    // (b) A thread with a genuinely live run, held open so its snapshot is real.
    bridge
        .start_agent_run(
            AgentRunRequest::new(
                "thread::live-run",
                "hold",
                "run::live",
                "api",
                "main",
                HashMap::new(),
            ),
            None,
        )
        .await
        .expect("run should start");
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .expect("provider should stream a partial reply");
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Some(blob) = state.threads.thread_store.get("thread::live-run").await
                && blob
                    .pointer("/history/active_run_snapshot/run_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("run::live")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("live run snapshot should persist");

    let repaired = crate::api::repair_stale_active_run_snapshots(&state).await;

    assert_eq!(repaired, 1, "only the stale (non-live) snapshot should repair");
    let stale = state
        .threads
        .thread_store
        .get("thread::stale")
        .await
        .unwrap();
    assert!(
        stale.pointer("/history/active_run_snapshot").is_none(),
        "the stale active_run_snapshot must be cleared"
    );
    let live = state
        .threads
        .thread_store
        .get("thread::live-run")
        .await
        .unwrap();
    assert_eq!(
        live.pointer("/history/active_run_snapshot/run_id")
            .and_then(serde_json::Value::as_str),
        Some("run::live"),
        "a genuinely live run's snapshot must be preserved"
    );

    release.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run::live").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("held run should complete after release");
}
