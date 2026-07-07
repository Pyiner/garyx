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

#[test]
fn single_bound_endpoint_identity_uses_persisted_binding_key() {
    let thread = json!({
        "channel_bindings": [{
            "channel": "telegram",
            "account_id": "bot1",
            "binding_key": "user42",
            "chat_id": "user42"
        }]
    });

    assert_eq!(
        single_bound_endpoint_identity(&thread).as_deref(),
        Some("telegram::bot1::user42")
    );
}

#[test]
fn single_bound_endpoint_identity_does_not_guess_with_multiple_bindings() {
    let thread = json!({
        "channel_bindings": [
            {
                "channel": "telegram",
                "account_id": "bot1",
                "binding_key": "user42",
                "chat_id": "user42"
            },
            {
                "channel": "feishu",
                "account_id": "bot2",
                "binding_key": "ou_test",
                "chat_id": "oc_test"
            }
        ]
    });

    assert!(single_bound_endpoint_identity(&thread).is_none());
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
            thread_title: None,
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
async fn test_dispatch_internal_message_to_thread_expands_bound_agent_runtime_metadata() {
    let bridge = Arc::new(MultiProviderBridge::new());
    let provider = Arc::new(RecordingProvider::default());
    bridge
        .register_provider("test-provider", provider.clone())
        .await;
    bridge.set_route("api", "main", "test-provider").await;
    bridge.set_default_provider_key("test-provider").await;

    // Seed the reviewer agent into the store *before* `build()`: the builder
    // spawns an async task that pushes the boot-time catalog snapshot into
    // the bridge's profile registry, so a post-build `upsert` +
    // `replace_agent_profiles` races that task and can be silently
    // overwritten by the (agent-less) boot snapshot. Seeding first makes the
    // boot snapshot itself carry the agent — no ordering dependency, and no
    // reliance on whatever agents exist in the developer's real
    // `~/.garyx/data/custom-agents.json`.
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents
        .upsert_agent_for_test(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::ClaudeCode,
            model: Some("claude-sonnet-4-6".to_owned()),
            model_reasoning_effort: Some("xhigh".to_owned()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: Some("Review carefully.".to_owned()),
        })
        .await
        .expect("custom agent");
    let state = crate::server::AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(custom_agents)
        .with_bridge(bridge.clone())
        .build();
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;
    // Production keeps the bridge's profile registry in sync on bootstrap and
    // on every agent write; the bridge chokepoint backfill reads from it.
    bridge
        .replace_agent_profiles(state.ops.custom_agents.list_agents().await)
        .await;

    state
        .threads
        .thread_store
        .set(
            "thread::agent-task",
            json!({
                "thread_id": "thread::agent-task",
                "channel": "api",
                "account_id": "main",
                "from_id": "loop",
                "is_group": false,
                "agent_id": "reviewer",
                "provider_type": "claude_code",
                "messages": [],
                "channel_bindings": []
            }),
        )
        .await;

    dispatch_internal_message_to_thread(
        &state,
        "thread::agent-task",
        "run-task-auto",
        "review this work",
        InternalDispatchOptions::default(),
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

    let metadata = &calls[0].2;
    assert_eq!(
        metadata["model"],
        Value::String("claude-sonnet-4-6".to_owned()),
        "the bound agent's model override must reach the provider"
    );
    assert_eq!(
        metadata["model_reasoning_effort"],
        Value::String("xhigh".to_owned())
    );
    assert_eq!(
        metadata["system_prompt"],
        Value::String("Review carefully.".to_owned())
    );
    assert_eq!(metadata["agent_id"], Value::String("reviewer".to_owned()));

    // A per-thread override chosen at thread creation must beat the agent
    // profile default on this path too.
    state
        .threads
        .thread_store
        .set(
            "thread::agent-task-override",
            json!({
                "thread_id": "thread::agent-task-override",
                "channel": "api",
                "account_id": "main",
                "from_id": "loop",
                "is_group": false,
                "agent_id": "reviewer",
                "provider_type": "claude_code",
                "metadata": {
                    "model_override": "claude-haiku-4-5",
                    "model_reasoning_effort_override": "low"
                },
                "messages": [],
                "channel_bindings": []
            }),
        )
        .await;
    dispatch_internal_message_to_thread(
        &state,
        "thread::agent-task-override",
        "run-task-auto-override",
        "review this work",
        InternalDispatchOptions::default(),
    )
    .await
    .unwrap();
    let calls = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let maybe_calls = {
                let calls = provider.calls.lock().unwrap();
                if calls.len() == 2 {
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
    .expect("provider should receive override dispatch");
    let metadata = &calls[1].2;
    assert_eq!(
        metadata["model"],
        Value::String("claude-haiku-4-5".to_owned()),
        "the thread-level override must beat the agent profile default"
    );
    assert_eq!(
        metadata["model_reasoning_effort"],
        Value::String("low".to_owned())
    );
    assert_eq!(
        metadata["system_prompt"],
        Value::String("Review carefully.".to_owned()),
        "fields without a thread override still come from the agent profile"
    );
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
