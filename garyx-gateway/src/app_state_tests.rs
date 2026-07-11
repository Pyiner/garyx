use super::*;
use crate::app_bootstrap::{AppStateBuilder, create_app_state};
use crate::recent_thread_projection::ActiveRunProbe;
use async_trait::async_trait;
use axum::body::Body;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_models::config::{GaryxConfig, TelegramAccount};
use garyx_models::provider::{
    AgentRunRequest, ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent,
};
use garyx_router::{InMemoryThreadStore, RunTranscriptRecordDraft, ThreadStore};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Notify;
use tower::ServiceExt;

fn test_state() -> Arc<AppState> {
    create_app_state(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
}

#[tokio::test]
async fn provider_runtime_ready_waiter_resolves_after_mark_ready() {
    let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
    .with_provider_runtime_ready(false)
    .build();
    assert!(!state.provider_runtime_ready());

    let waiter_state = state.clone();
    let waiter = tokio::spawn(async move {
        waiter_state
            .wait_for_provider_runtime_ready(std::time::Duration::from_secs(1))
            .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    state.mark_provider_runtime_ready();

    assert!(waiter.await.unwrap());
    assert!(state.provider_runtime_ready());
}

struct NeverActiveRunProbe;

#[async_trait]
impl ActiveRunProbe for NeverActiveRunProbe {
    async fn is_run_active(&self, _run_id: &str) -> bool {
        false
    }
}

async fn append_run_start(state: &Arc<AppState>, thread_id: &str, run_id: &str) {
    state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some(run_id),
            &[RunTranscriptRecordDraft::with_timestamp(
                serde_json::json!({
                    "role": "system",
                    "kind": "control",
                    "internal": true,
                    "internal_kind": "control",
                    "control": {
                        "kind": "run_start",
                        "thread_id": thread_id,
                        "run_id": run_id,
                        "at": "2026-06-18T12:00:00Z"
                    }
                }),
                "2026-06-18T12:00:00Z",
            )],
        )
        .await
        .expect("run_start control should append");
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
        .await
        .unwrap();

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
            .unwrap()
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
async fn startup_warmup_clears_dangling_orphan_run() {
    let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
    .with_active_run_probe(Arc::new(NeverActiveRunProbe))
    .build();
    append_run_start(&state, "thread::cold-running", "run::cold-running").await;
    state
        .threads
        .thread_store
        .set(
            "thread::cold-running",
            serde_json::json!({
                "thread_id": "thread::cold-running",
                "label": "Cold Running",
                "updated_at": "2026-01-01T00:00:01Z",
                "history": {
                    "message_count": 1,
                    "recent_committed_run_ids": ["earlier-run"]
                }
            }),
        )
        .await
        .unwrap();
    state.spawn_gateway_sync_cache_warmup();

    // Startup settles orphaned running rows with one SQL pass: the bridge
    // run index is empty at boot, so the projected active run left by the
    // previous process must resolve to completed (#TASK-1864).
    let record = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let records = state
                .ops
                .garyx_db
                .list_recent_threads(10, 0)
                .expect("list recent threads");
            if let Some(record) = records
                .iter()
                .find(|record| record.thread_id == "thread::cold-running")
                && record.active_run_id.is_none()
            {
                break record.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("warmup should settle the dangling run as an orphan");

    assert_eq!(record.active_run_id, None);
    assert_eq!(record.run_state, "completed");
}

// Reproduction (state-driven, no UI): a streaming run that is aborted must append
// a terminal committed control row so the transcript reducer returns to idle.
#[tokio::test]
async fn aborting_a_streaming_run_commits_terminal_control() {
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

    bridge
        .start_agent_run(
            AgentRunRequest::new(
                "thread::aborted-run",
                "do work",
                "run::aborted",
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
        .expect("provider should stream a partial");
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Ok(run_state) = state
                .threads
                .history
                .transcript_store()
                .run_state("thread::aborted-run")
                .await
                && run_state.busy
                && run_state.active_run_id.as_deref() == Some("run::aborted")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run_start control should project the run as busy");

    assert!(
        bridge.abort_run("run::aborted").await,
        "abort should cancel the active run"
    );
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while bridge.is_run_active("run::aborted").await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("aborted run should leave the active set");

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let run_state = state
                .threads
                .history
                .transcript_store()
                .run_state("thread::aborted-run")
                .await
                .expect("run state should reduce");
            if !run_state.busy {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("abort terminal control should make committed run state idle");

    if let Some(record) = state
        .ops
        .garyx_db
        .list_recent_threads(50, 0)
        .unwrap()
        .into_iter()
        .find(|record| record.thread_id == "thread::aborted-run")
    {
        assert_ne!(
            record.run_state, "running",
            "an aborted run must not leave the thread projected as running"
        );
        assert_eq!(record.active_run_id, None);
    }
}
