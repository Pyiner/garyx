use super::*;
use async_trait::async_trait;
use garyx_bridge::{AgentLoopProvider, BridgeError};
use garyx_channels::{ChannelDispatcher, ChannelDispatcherImpl, ChannelInfo, OutboundMessage};
use garyx_models::config::ActiveHoursConfig;
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, StreamBoundaryKind};
use garyx_models::thread_logs::NoopThreadLogSink;
use garyx_router::ThreadStore;
use tempfile::TempDir;

#[derive(Default)]
struct RecordingDispatcher {
    calls: std::sync::Mutex<Vec<OutboundMessage>>,
    message_ids: std::sync::Mutex<Vec<String>>,
}

struct FailingClearProvider;

struct RecordingClearProvider {
    cleared_sessions: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl AgentLoopProvider for FailingClearProvider {
    fn provider_type(&self) -> garyx_models::provider::ProviderType {
        garyx_models::provider::ProviderType::ClaudeCode
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
        _options: &ProviderRunOptions,
        _on_chunk: garyx_bridge::provider_trait::StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        Err(BridgeError::Internal(
            "not used in heartbeat cleanup test".to_owned(),
        ))
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
    }

    async fn clear_session(&self, _session_key: &str) -> bool {
        false
    }
}

#[async_trait]
impl AgentLoopProvider for RecordingClearProvider {
    fn provider_type(&self) -> garyx_models::provider::ProviderType {
        garyx_models::provider::ProviderType::ClaudeCode
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
        _options: &ProviderRunOptions,
        _on_chunk: garyx_bridge::provider_trait::StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        Err(BridgeError::Internal(
            "not used in heartbeat cleanup test".to_owned(),
        ))
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
    }

    async fn clear_session(&self, session_key: &str) -> bool {
        self.cleared_sessions
            .lock()
            .unwrap()
            .push(session_key.to_owned());
        true
    }
}

impl RecordingDispatcher {
    fn with_message_ids(ids: Vec<String>) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            message_ids: std::sync::Mutex::new(ids),
        }
    }

    fn calls(&self) -> Vec<OutboundMessage> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .clone()
    }
}

#[async_trait]
impl ChannelDispatcher for RecordingDispatcher {
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, garyx_channels::ChannelError> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .push(request);
        Ok(SendMessageResult {
            message_ids: self
                .message_ids
                .lock()
                .expect("recording dispatcher message_ids lock poisoned")
                .clone(),
        })
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        vec![ChannelInfo {
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            is_running: true,
        }]
    }
}

#[test]
fn parse_interval_variants() {
    assert_eq!(HeartbeatService::parse_interval("30s").as_secs(), 30);
    assert_eq!(HeartbeatService::parse_interval("5m").as_secs(), 300);
    assert_eq!(HeartbeatService::parse_interval("3h").as_secs(), 10800);
    assert_eq!(HeartbeatService::parse_interval("1d").as_secs(), 86400);
    assert_eq!(HeartbeatService::parse_interval("500ms").as_millis(), 500);
    assert_eq!(HeartbeatService::parse_interval("0.5s").as_millis(), 500);
    assert_eq!(HeartbeatService::parse_interval("3600").as_secs(), 216000);
    // Fallback for garbage input.
    assert_eq!(
        HeartbeatService::parse_interval("garbage").as_secs(),
        3 * 3600
    );
}

#[test]
fn active_hours_no_config_always_active() {
    let config = HeartbeatConfig {
        active_hours: None,
        ..HeartbeatConfig::default()
    };
    assert!(HeartbeatService::is_within_active_hours(&config));
}

#[test]
fn active_hours_wide_range() {
    // 00:00 - 23:59 should always be active.
    let config = HeartbeatConfig {
        active_hours: Some(ActiveHoursConfig {
            start: "00:00".to_owned(),
            end: "23:59".to_owned(),
            timezone: "utc".to_owned(),
        }),
        ..HeartbeatConfig::default()
    };
    assert!(HeartbeatService::is_within_active_hours(&config));
}

#[test]
fn active_hours_timezone_iana() {
    let config = HeartbeatConfig {
        active_hours: Some(ActiveHoursConfig {
            start: "00:00".to_owned(),
            end: "23:59".to_owned(),
            timezone: "Asia/Shanghai".to_owned(),
        }),
        ..HeartbeatConfig::default()
    };
    assert!(HeartbeatService::is_within_active_hours(&config));
}

#[test]
fn active_hours_equal_bounds_mean_always_active() {
    let config = HeartbeatConfig {
        active_hours: Some(ActiveHoursConfig {
            start: "09:00".to_owned(),
            end: "09:00".to_owned(),
            timezone: "utc".to_owned(),
        }),
        ..HeartbeatConfig::default()
    };
    assert!(HeartbeatService::is_within_active_hours(&config));
}

#[test]
fn active_hours_invalid_time_falls_back_to_active() {
    let config = HeartbeatConfig {
        active_hours: Some(ActiveHoursConfig {
            start: "invalid".to_owned(),
            end: "23:00".to_owned(),
            timezone: "utc".to_owned(),
        }),
        ..HeartbeatConfig::default()
    };
    assert!(HeartbeatService::is_within_active_hours(&config));
}

#[test]
fn active_hours_supports_24_00_end() {
    use chrono::TimeZone;

    let config = HeartbeatConfig {
        active_hours: Some(ActiveHoursConfig {
            start: "00:00".to_owned(),
            end: "24:00".to_owned(),
            timezone: "utc".to_owned(),
        }),
        ..HeartbeatConfig::default()
    };
    let fixed = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
    assert!(HeartbeatService::is_within_active_hours_at(&config, fixed));
}

#[tokio::test]
async fn start_stop_lifecycle() {
    let config = HeartbeatConfig {
        enabled: true,
        every: "1s".to_owned(),
        ..HeartbeatConfig::default()
    };
    let mut svc = HeartbeatService::new(config);
    svc.start();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    svc.stop().await;
}

#[tokio::test]
async fn start_is_idempotent() {
    let config = HeartbeatConfig {
        enabled: true,
        every: "1s".to_owned(),
        ..HeartbeatConfig::default()
    };
    let mut svc = HeartbeatService::new(config);

    svc.start();
    let first_sender = svc.stop_tx.clone();
    assert!(first_sender.is_some());

    // Second start should be ignored and keep current run loop.
    svc.start();
    assert!(svc.stop_tx.is_some());

    svc.stop().await;
    assert!(svc.stop_tx.is_none());
}

#[tokio::test]
async fn disabled_does_not_start() {
    let config = HeartbeatConfig {
        enabled: false,
        ..HeartbeatConfig::default()
    };
    let mut svc = HeartbeatService::new(config);
    svc.start();
    // stop_tx should be None because start() returned early.
    assert!(svc.stop_tx.is_none());
}

#[tokio::test]
async fn trigger_fires_event() {
    let config = HeartbeatConfig::default();
    let (tx, mut rx) = broadcast::channel(16);
    let mut svc = HeartbeatService::new(config);
    svc.set_event_tx(tx);
    svc.trigger().await;
    let msg = rx.recv().await.unwrap();
    assert!(msg.contains("heartbeat_fired"));
    assert!(msg.contains("\"thread_id\""));
    assert!(!msg.contains("\"session_key\""));
}

#[tokio::test]
async fn test_build_scheduled_response_callback_sends_final_message() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let router = Arc::new(Mutex::new(MessageRouter::new(
        Arc::new(garyx_router::InMemoryThreadStore::new()),
        garyx_models::config::GaryxConfig::default(),
    )));
    let callback = build_scheduled_response_callback(
        dispatcher.clone(),
        router,
        ScheduledResponseContext {
            thread_id: "main::heartbeat::morning".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            delivery_thread_id: Some("42_t100".to_owned()),
            thread_log_id: None,
        },
    );

    callback(StreamEvent::Delta {
        text: "hello ".to_owned(),
    });
    callback(StreamEvent::Delta {
        text: "world".to_owned(),
    });
    callback(StreamEvent::Done);

    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].text, "#main::heartbeat::morning\nhello world");
    assert_eq!(calls[0].channel, "telegram");
    assert_eq!(calls[0].account_id, "main");
    assert_eq!(calls[0].chat_id, "42");
}

#[tokio::test]
async fn test_build_scheduled_response_callback_records_reply_routing() {
    let dispatcher = Arc::new(RecordingDispatcher::with_message_ids(vec![
        "hb_msg_1".to_owned(),
    ]));
    let store = Arc::new(garyx_router::InMemoryThreadStore::new());
    store
        .set(
            "main::heartbeat::morning",
            serde_json::json!({
                "thread_id": "main::heartbeat::morning",
            }),
        )
        .await;
    let router = Arc::new(Mutex::new(MessageRouter::new(
        store.clone(),
        garyx_models::config::GaryxConfig::default(),
    )));
    let callback = build_scheduled_response_callback(
        dispatcher,
        router.clone(),
        ScheduledResponseContext {
            thread_id: "main::heartbeat::morning".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            delivery_thread_id: Some("42_t100".to_owned()),
            thread_log_id: None,
        },
    );

    callback(StreamEvent::Delta {
        text: "hello".to_owned(),
    });
    callback(StreamEvent::Done);
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    {
        let router_guard = router.lock().await;
        assert_eq!(
            router_guard.resolve_reply_thread_for_chat(
                "telegram",
                "main",
                Some("42"),
                Some("42_t100"),
                "hb_msg_1",
            ),
            Some("main::heartbeat::morning")
        );
    }

    let thread_state = store.get("main::heartbeat::morning").await.unwrap();
    assert_eq!(
        thread_state["outbound_message_ids"][0]["thread_binding_key"],
        Value::String("42_t100".to_owned())
    );
    assert_eq!(
        thread_state["outbound_message_ids"][0]["message_id"],
        Value::String("hb_msg_1".to_owned())
    );
}

#[tokio::test]
async fn test_build_scheduled_response_callback_preserves_assistant_segments() {
    let dispatcher = Arc::new(RecordingDispatcher::with_message_ids(vec![
        "hb_msg_1".to_owned(),
    ]));
    let store = Arc::new(garyx_router::InMemoryThreadStore::new());
    let router = Arc::new(Mutex::new(MessageRouter::new(
        store,
        garyx_models::config::GaryxConfig::default(),
    )));
    let callback = build_scheduled_response_callback(
        dispatcher.clone(),
        router,
        ScheduledResponseContext {
            thread_id: "main::heartbeat::morning".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            delivery_thread_id: None,
            thread_log_id: None,
        },
    );

    callback(StreamEvent::Delta {
        text: "first".to_owned(),
    });
    callback(StreamEvent::Boundary {
        kind: StreamBoundaryKind::AssistantSegment,
        pending_input_id: None,
    });
    callback(StreamEvent::Delta {
        text: "second".to_owned(),
    });
    callback(StreamEvent::Done);
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].text, "#main::heartbeat::morning\nfirst\n\nsecond");
}

#[tokio::test]
async fn test_build_scheduled_response_callback_stops_after_user_ack_boundary() {
    let dispatcher = Arc::new(RecordingDispatcher::with_message_ids(vec![
        "hb_msg_1".to_owned(),
    ]));
    let store = Arc::new(garyx_router::InMemoryThreadStore::new());
    let router = Arc::new(Mutex::new(MessageRouter::new(
        store,
        garyx_models::config::GaryxConfig::default(),
    )));
    let callback = build_scheduled_response_callback(
        dispatcher.clone(),
        router,
        ScheduledResponseContext {
            thread_id: "main::heartbeat::morning".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            delivery_thread_id: None,
            thread_log_id: None,
        },
    );

    callback(StreamEvent::Delta {
        text: "first".to_owned(),
    });
    callback(StreamEvent::Boundary {
        kind: StreamBoundaryKind::UserAck,
        pending_input_id: None,
    });
    callback(StreamEvent::Delta {
        text: "second".to_owned(),
    });
    callback(StreamEvent::Done);
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].text, "#main::heartbeat::morning\nfirst");
}

#[tokio::test]
async fn test_dispatch_heartbeat_recovers_delivery_target_from_store() {
    let store = Arc::new(garyx_router::InMemoryThreadStore::new());
    let thread_id = "bot1::main::u1";
    store
        .set(
            thread_id,
            serde_json::json!({
                "lastChannel": "telegram",
                "lastTo": "42",
                "lastAccountId": "main",
                "lastUpdatedAt": "2026-03-01T12:00:00Z",
            }),
        )
        .await;
    let router = Arc::new(Mutex::new(MessageRouter::new(
        store,
        garyx_models::config::GaryxConfig::default(),
    )));
    let runtime = HeartbeatDispatchRuntime {
        router,
        bridge: Arc::new(MultiProviderBridge::new()),
        channel_dispatcher: Arc::new(ChannelDispatcherImpl::new()),
        thread_logs: Arc::new(NoopThreadLogSink),
        managed_mcp_servers: HashMap::new(),
    };
    let config = HeartbeatConfig {
        target: format!("thread:{thread_id}"),
        ..HeartbeatConfig::default()
    };

    let err = dispatch_heartbeat(&config, &runtime)
        .await
        .expect_err("bridge has no providers in this test");
    assert!(err.contains("bridge dispatch error"));
    assert!(err.contains("channel=telegram"));
    assert!(!err.contains("no delivery context found"));
}

#[tokio::test]
async fn records_persist_and_reload() {
    let tmp = TempDir::new().unwrap();
    let mut svc = HeartbeatService::new(HeartbeatConfig::default());
    svc.set_data_dir(tmp.path().to_path_buf());
    svc.trigger().await;

    let mut svc2 = HeartbeatService::new(HeartbeatConfig::default());
    svc2.set_data_dir(tmp.path().to_path_buf());
    svc2.load_persisted_records().await.unwrap();
    let records = svc2.recent_records().await;
    assert_eq!(records.len(), 1);
    assert!(records[0].thread_id.is_none());
    let json = serde_json::to_value(&records[0]).unwrap();
    assert!(json.get("session_key").is_none());
}

#[tokio::test]
async fn stale_heartbeat_cleanup_keeps_thread_when_provider_clear_fails() {
    let store = Arc::new(garyx_router::InMemoryThreadStore::new());
    let thread_id = "default::heartbeat::stale";
    store
        .set(
            thread_id,
            serde_json::json!({
                "provider_key": "p1",
                "_updated_at": "2020-01-01T00:00:00Z",
            }),
        )
        .await;

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("p1", Arc::new(FailingClearProvider))
        .await;
    bridge.set_thread_affinity(thread_id, "p1").await;
    bridge
        .set_thread_workspace_binding(thread_id, Some("/tmp/hb-stale".to_owned()))
        .await;

    let router = Arc::new(Mutex::new(MessageRouter::new(
        store.clone(),
        garyx_models::config::GaryxConfig::default(),
    )));
    let svc = HeartbeatService::new(HeartbeatConfig {
        target: "thread:noop".to_owned(),
        ..HeartbeatConfig::default()
    });
    svc.set_dispatch_runtime(
        router,
        bridge.clone(),
        Arc::new(ChannelDispatcherImpl::new()),
        Arc::new(NoopThreadLogSink),
        HashMap::new(),
    )
    .await;

    svc.trigger().await;

    assert!(store.get(thread_id).await.is_some());
    assert_eq!(
        bridge
            .resolve_provider_for_thread(thread_id, "telegram", "main")
            .await,
        Some("p1".to_owned())
    );
    assert_eq!(
        bridge
            .thread_workspace_bindings_snapshot()
            .await
            .get(thread_id)
            .map(String::as_str),
        Some("/tmp/hb-stale")
    );
}

#[tokio::test]
async fn stale_heartbeat_cleanup_uses_bridge_affinity_without_provider_key_field() {
    let store = Arc::new(garyx_router::InMemoryThreadStore::new());
    let thread_id = "default::heartbeat::stale-affinity";
    store
        .set(
            thread_id,
            serde_json::json!({
                "_updated_at": "2020-01-01T00:00:00Z",
            }),
        )
        .await;

    let provider = Arc::new(RecordingClearProvider {
        cleared_sessions: std::sync::Mutex::new(Vec::new()),
    });
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge.register_provider("p1", provider.clone()).await;
    bridge.set_thread_affinity(thread_id, "p1").await;

    let router = Arc::new(Mutex::new(MessageRouter::new(
        store.clone(),
        garyx_models::config::GaryxConfig::default(),
    )));
    let svc = HeartbeatService::new(HeartbeatConfig {
        target: "thread:noop".to_owned(),
        ..HeartbeatConfig::default()
    });
    svc.set_dispatch_runtime(
        router,
        bridge.clone(),
        Arc::new(ChannelDispatcherImpl::new()),
        Arc::new(NoopThreadLogSink),
        HashMap::new(),
    )
    .await;

    svc.trigger().await;

    assert!(store.get(thread_id).await.is_none());
    assert_eq!(
        provider.cleared_sessions.lock().unwrap().as_slice(),
        [thread_id]
    );
    assert!(bridge.thread_affinity_for(thread_id).await.is_none());
}
