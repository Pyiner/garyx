use super::*;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_models::config::GaryxConfig;
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent};
use garyx_router::MessageRouter;

use crate::server::create_app_state_with_bridge;

#[derive(Default)]
struct RecordingProvider {
    calls: StdMutex<Vec<(String, String, HashMap<String, Value>)>>,
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
async fn dispatch_loop_continuation_uses_explicit_thread_hook() {
    let bridge = Arc::new(MultiProviderBridge::new());
    let provider = Arc::new(RecordingProvider::default());
    bridge
        .register_provider("test-provider", provider.clone())
        .await;
    bridge.set_route("telegram", "bot1", "test-provider").await;
    bridge.set_default_provider_key("test-provider").await;

    let state = create_app_state_with_bridge(GaryxConfig::default(), bridge.clone());
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;

    state
        .threads
        .thread_store
        .set(
            "thread::loop-session",
            json!({
                "thread_id": "thread::loop-session",
                "channel": "telegram",
                "account_id": "bot1",
                "from_id": "user42",
                "is_group": false,
                "loop_enabled": true,
                "messages": [],
                "channel_bindings": []
            }),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            "thread::current-session",
            json!({
                "thread_id": "thread::current-session",
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
        router.switch_to_thread(&user_key, "thread::current-session");
    }

    dispatch_loop_continuation(&state, "thread::loop-session", 3)
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
    .expect("provider should receive loop continuation dispatch");

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "thread::loop-session");
    assert_eq!(calls[0].1, LOOP_CONTINUATION_MESSAGE);
    assert_eq!(calls[0].2["loop_iteration"], json!(3));
    assert_eq!(calls[0].2["loop_continuation"], Value::Bool(true));
    assert_eq!(calls[0].2["internal_dispatch"], Value::Bool(true));

    let current = {
        let router = state.threads.router.lock().await;
        router
            .get_current_thread_id_for_account("telegram", "bot1", "user42", false, None)
            .map(str::to_owned)
    };
    assert_eq!(current.as_deref(), Some("thread::current-session"));
}

#[tokio::test]
async fn dispatch_loop_continuation_skips_disabled_thread() {
    let bridge = Arc::new(MultiProviderBridge::new());
    let provider = Arc::new(RecordingProvider::default());
    bridge
        .register_provider("test-provider", provider.clone())
        .await;
    bridge.set_route("telegram", "bot1", "test-provider").await;
    bridge.set_default_provider_key("test-provider").await;

    let state = create_app_state_with_bridge(GaryxConfig::default(), bridge.clone());
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;

    state
        .threads
        .thread_store
        .set(
            "thread::loop-session",
            json!({
                "thread_id": "thread::loop-session",
                "channel": "telegram",
                "account_id": "bot1",
                "from_id": "user42",
                "is_group": false,
                "loop_enabled": false,
                "messages": [],
                "channel_bindings": []
            }),
        )
        .await;

    dispatch_loop_continuation(&state, "thread::loop-session", 4)
        .await
        .unwrap();

    let calls = provider.calls.lock().unwrap();
    assert!(calls.is_empty());
}
