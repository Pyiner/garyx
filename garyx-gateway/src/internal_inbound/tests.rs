use super::*;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_models::config::GaryxConfig;
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent};
use garyx_router::MessageRouter;
use serde_json::json;

type ProviderCall = (String, String, HashMap<String, Value>);

#[derive(Default)]
struct RecordingProvider {
    calls: StdMutex<Vec<ProviderCall>>,
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
        self.calls.lock().unwrap().push((
            options.thread_id.clone(),
            options.message.clone(),
            options.metadata.clone(),
        ));
        on_chunk(StreamEvent::Delta {
            text: "ok".to_owned(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "recording-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "ok".to_owned(),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            success: true,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            duration_ms: 0,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
    }
}

#[tokio::test]
async fn test_dispatch_internal_message_to_thread_uses_explicit_thread() {
    let bridge = Arc::new(MultiProviderBridge::new());
    let provider = Arc::new(RecordingProvider::default());
    bridge
        .register_provider("test-provider", provider.clone())
        .await;
    bridge.set_route("telegram", "bot1", "test-provider").await;
    bridge.set_default_provider_key("test-provider").await;

    let state = crate::server::create_app_state_with_bridge(GaryxConfig::default(), bridge.clone());
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;

    state
        .threads
        .thread_store
        .set(
            "thread::old-thread",
            json!({
                "thread_id": "thread::old-thread",
                "channel": "telegram",
                "account_id": "bot1",
                "from_id": "user42",
                "is_group": false,
                "messages": [],
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "bot1",
                    "binding_key": "user42",
                    "chat_id": "user42",
                    "display_label": "user42"
                }],
                "delivery_context": {
                    "channel": "telegram",
                    "account_id": "bot1",
                    "chat_id": "user42",
                    "user_id": "user42",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "user42",
                    "thread_id": "user42",
                    "metadata": {}
                }
            }),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            "thread::current-thread",
            json!({
                "thread_id": "thread::current-thread",
                "channel": "telegram",
                "account_id": "bot1",
                "from_id": "user42",
                "is_group": false,
                "messages": [],
                "channel_bindings": []
            }),
        )
        .await;

    {
        let mut router = state.threads.router.lock().await;
        let user_key =
            MessageRouter::build_account_user_key("telegram", "bot1", "user42", false, None);
        router.switch_to_thread(&user_key, "thread::current-thread");
    }

    dispatch_internal_message_to_thread(
        &state,
        "thread::old-thread",
        "run-loop-hook",
        "continue working",
        InternalDispatchOptions {
            extra_metadata: HashMap::from([("loop_continuation".to_owned(), Value::Bool(true))]),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let calls = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let maybe_calls = {
                let calls = provider.calls.lock().unwrap();
                if calls.len() == 1 {
                    Some(calls.clone())
                } else {
                    None
                }
            };
            if let Some(calls) = maybe_calls {
                break calls;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("provider should receive internal dispatch");

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "thread::old-thread");
    assert_eq!(calls[0].1, "continue working");
    assert_eq!(calls[0].2["loop_continuation"], Value::Bool(true));
    assert_eq!(calls[0].2["internal_dispatch"], Value::Bool(true));
    assert_eq!(
        calls[0].2["thread_binding_key"],
        Value::String("user42".to_owned())
    );
    assert!(calls[0].2["delivery_thread_id"].is_null());

    let saved = state
        .threads
        .thread_store
        .get("thread::old-thread")
        .await
        .expect("thread should remain persisted");
    assert!(saved["delivery_context"]["thread_id"].is_null());

    let events = state
        .threads
        .message_ledger
        .list_events_for_thread("thread::old-thread", 10)
        .await
        .unwrap();
    assert!(!events.is_empty());
    assert_eq!(events.last().unwrap().bot_id, "telegram:bot1");
    assert_eq!(
        events.last().unwrap().run_id.as_deref(),
        Some("run-loop-hook")
    );
    let statuses = events.iter().map(|event| event.status).collect::<Vec<_>>();
    assert!(statuses.contains(&garyx_models::MessageLifecycleStatus::RunStarted));
    assert!(statuses.contains(&garyx_models::MessageLifecycleStatus::ThreadResolved));

    let current = {
        let router = state.threads.router.lock().await;
        router
            .get_current_thread_id_for_account("telegram", "bot1", "user42", false, None)
            .map(str::to_owned)
    };
    assert_eq!(current.as_deref(), Some("thread::current-thread"));
}

#[tokio::test]
async fn test_dispatch_internal_message_to_thread_restores_missing_dm_binding() {
    let bridge = Arc::new(MultiProviderBridge::new());
    let provider = Arc::new(RecordingProvider::default());
    bridge
        .register_provider("test-provider", provider.clone())
        .await;
    bridge.set_route("telegram", "bot1", "test-provider").await;
    bridge.set_default_provider_key("test-provider").await;

    let state = crate::server::create_app_state_with_bridge(GaryxConfig::default(), bridge.clone());
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;

    state
        .threads
        .thread_store
        .set(
            "thread::restore-binding",
            json!({
                "thread_id": "thread::restore-binding",
                "channel": "telegram",
                "account_id": "bot1",
                "from_id": "user42",
                "is_group": false,
                "messages": [],
                "channel_bindings": [],
                "delivery_context": {
                    "channel": "telegram",
                    "account_id": "bot1",
                    "chat_id": "user42",
                    "user_id": "user42",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "user42",
                    "thread_id": null,
                    "metadata": {}
                }
            }),
        )
        .await;

    dispatch_internal_message_to_thread(
        &state,
        "thread::restore-binding",
        "run-restore-binding",
        "continue working",
        InternalDispatchOptions::default(),
    )
    .await
    .unwrap();

    let saved = state
        .threads
        .thread_store
        .get("thread::restore-binding")
        .await
        .expect("thread should remain persisted");
    let bindings = saved["channel_bindings"]
        .as_array()
        .expect("channel_bindings should be an array");
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0]["channel"], "telegram");
    assert_eq!(bindings[0]["account_id"], "bot1");
    assert_eq!(bindings[0]["binding_key"], "user42");
    assert_eq!(bindings[0]["chat_id"], "user42");
    assert!(saved["delivery_context"]["thread_id"].is_null());
}
