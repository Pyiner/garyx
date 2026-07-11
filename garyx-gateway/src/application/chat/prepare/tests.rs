use super::*;
use crate::custom_agents::CustomAgentStore;
use crate::server::AppStateBuilder;
use garyx_models::config::{GaryxConfig, McpServerConfig, SlashCommand};
use serde_json::json;

fn test_state() -> Arc<AppState> {
    AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(Arc::new(CustomAgentStore::new()))
        .build()
}

#[test]
fn build_provider_run_metadata_injects_managed_mcp_servers() {
    let mut config = GaryxConfig::default();
    config.mcp_servers.insert(
        "managed-proof".to_owned(),
        McpServerConfig {
            command: "python3".to_owned(),
            args: vec!["managed.py".to_owned()],
            env: HashMap::new(),
            working_dir: Some("/tmp".to_owned()),
            ..Default::default()
        },
    );
    let mut request_metadata = HashMap::new();
    request_metadata.insert(
        "remote_mcp_servers".to_owned(),
        json!({
            "runtime-proof": {
                "command": "python3",
                "args": ["runtime.py"],
                "enabled": true
            }
        }),
    );

    let metadata = build_provider_run_metadata(
        &config,
        request_metadata,
        "api",
        "main",
        "api-user",
        "run-1",
    );

    assert_eq!(metadata["channel"], "api");
    assert_eq!(
        metadata["remote_mcp_servers"]["managed-proof"]["args"],
        json!(["managed.py"])
    );
    assert_eq!(
        metadata["remote_mcp_servers"]["runtime-proof"]["args"],
        json!(["runtime.py"])
    );
}

#[test]
fn build_provider_run_metadata_uses_supplied_channel_context() {
    let metadata = build_provider_run_metadata(
        &GaryxConfig::default(),
        HashMap::new(),
        "telegram",
        "codex_bot",
        "1000000001",
        "run-1",
    );

    assert_eq!(metadata["channel"], "telegram");
    assert_eq!(metadata["account_id"], "codex_bot");
    assert_eq!(metadata["from_id"], "1000000001");
}

#[test]
fn resolve_chat_message_applies_custom_slash_command() {
    let mut config = GaryxConfig::default();
    config.commands.push(SlashCommand {
        name: "review".to_owned(),
        description: String::new(),
        prompt: Some("Please review".to_owned()),
        skill_id: None,
    });
    let mut req: ChatRequest = serde_json::from_value(json!({
        "message": "/review fix this"
    }))
    .unwrap();

    let resolved = resolve_chat_message(&config, &mut req);

    assert_eq!(resolved, "Please review");
}

#[test]
fn prompt_derived_thread_label_trims_and_truncates() {
    let label = prompt_derived_thread_label(
        "   Investigate   why   the scheduled task missed three runs after gateway restart   ",
    )
    .expect("label should be derived");

    assert_eq!(label, "Investigate why the scheduled task miss…");
}

#[test]
fn should_autoname_thread_accepts_missing_or_legacy_label() {
    assert!(should_autoname_thread(&json!({})));
    assert!(should_autoname_thread(&json!({ "label": "" })));
    assert!(should_autoname_thread(&json!({ "label": "Fresh Thread" })));
    assert!(should_autoname_thread(&json!({
        "label": "api/main/api-user",
        "channel": "api",
        "account_id": "main",
        "from_id": "api-user"
    })));
    assert!(!should_autoname_thread(&json!({ "label": "Real Title" })));
    assert!(!should_autoname_thread(&json!({
        "label": "#TASK-33 Ship thread title",
        "thread_title_source": "task"
    })));
}

#[tokio::test]
async fn persist_thread_label_marks_prompt_fallback_source() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::prompt-title",
            json!({ "thread_id": "thread::prompt-title" }),
        )
        .await
        .unwrap();

    let title_update = persist_thread_label_if_missing(
        &state,
        "thread::prompt-title",
        "Please investigate provider title events",
    )
    .await
    .expect("label persists");

    assert_eq!(
        title_update.as_deref(),
        Some("Please investigate provider title events")
    );
    let updated = state
        .threads
        .thread_store
        .get("thread::prompt-title")
        .await
        .unwrap()
        .expect("thread exists");
    assert_eq!(updated["label"], "Please investigate provider title events");
    assert_eq!(updated["thread_title_source"], "garyx_prompt");
}

#[tokio::test]
async fn prepare_chat_request_resolves_provider_and_system_prompt_from_thread_agent() {
    let state = test_state();
    state
        .ops
        .custom_agents
        .upsert_agent_for_test(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "spec-review".to_owned(),
            display_name: "Spec Review".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: Some("gpt-5-codex".to_owned()),
            model_reasoning_effort: Some("xhigh".to_owned()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: Some("Review specs carefully.".to_owned()),
        })
        .await
        .expect("custom agent saved");
    state
        .threads
        .thread_store
        .set(
            "thread::agent-bound",
            json!({
                "thread_id": "thread::agent-bound",
                "thread_mode": "single_agent",
                "agent_id": "spec-review",
                "channel": "api",
                "account_id": "main",
                "from_id": "api-user",
                "messages": [],
                "workspace_dir": "/repo",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "bot1",
                    "binding_key": "api-user",
                    "chat_id": "chat-1",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "chat-1",
                    "display_label": "API User"
                }],
                "task": {
                    "schema_version": 1,
                    "number": 4,
                    "title": "Prompt metadata",
                    "status": "todo",
                    "creator": { "kind": "human", "user_id": "api-user" },
                    "created_at": "2026-05-02T00:00:00Z",
                    "updated_at": "2026-05-02T00:00:00Z",
                    "updated_by": { "kind": "agent", "agent_id": "spec-review" },
                    "events": []
                }
            }),
        )
        .await
        .unwrap();

    let prepared = prepare_chat_request(
        &state,
        ChatRequest {
            thread_id: Some("thread::agent-bound".to_owned()),
            message: "Check this design".to_owned(),
            attachments: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
            client_intent_id: Some("00000000-0000-0000-0000-000000000001".to_owned()),
            from_id: "api-user".to_owned(),
            account_id: "main".to_owned(),
            bot: None,
            wait_for_response: true,
            workspace_path: None,
            provider_type: Some(ProviderType::ClaudeCode),
            metadata: HashMap::new(),
        },
    )
    .await
    .expect("prepare chat request");

    assert_eq!(prepared.provider_type, Some(ProviderType::CodexAppServer));
    assert_eq!(
        prepared
            .metadata
            .get("system_prompt")
            .and_then(Value::as_str),
        Some("Review specs carefully.")
    );
    assert_eq!(
        prepared.metadata.get("agent_id").and_then(Value::as_str),
        Some("spec-review")
    );
    assert_eq!(
        prepared.metadata.get("model").and_then(Value::as_str),
        Some("gpt-5-codex")
    );
    assert_eq!(
        prepared
            .metadata
            .get("client_intent_id")
            .and_then(Value::as_str),
        Some("00000000-0000-0000-0000-000000000001")
    );
    assert_eq!(
        prepared
            .metadata
            .get("model_reasoning_effort")
            .and_then(Value::as_str),
        Some("xhigh")
    );
    assert_eq!(
        prepared
            .metadata
            .get("agent_display_name")
            .and_then(Value::as_str),
        Some("Spec Review")
    );
    let runtime_context = prepared
        .metadata
        .get("runtime_context")
        .expect("runtime context");
    assert_eq!(runtime_context["thread_id"], "thread::agent-bound");
    assert_eq!(runtime_context["bot_id"], "api:main");
    let bound_bots = runtime_context["thread"]["bound_bots"]
        .as_array()
        .expect("bound bots");
    assert!(bound_bots.contains(&json!("api:main")));
    assert!(bound_bots.contains(&json!("telegram:bot1")));
    assert_eq!(runtime_context["task"]["task_id"], "#TASK-4");
    assert_eq!(runtime_context["task"]["status"], "todo");
}

#[tokio::test]
async fn prepare_chat_request_prefers_thread_snapshot_before_agent_runtime_metadata() {
    let state = test_state();
    state
        .ops
        .custom_agents
        .upsert_agent_for_test(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "snapshot-agent".to_owned(),
            display_name: "Snapshot Agent".to_owned(),
            provider_type: ProviderType::ClaudeCode,
            model: Some("agent-model-v2".to_owned()),
            model_reasoning_effort: Some("max".to_owned()),
            model_service_tier: Some("auto".to_owned()),
            provider_env: None,
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: Some("Use the agent persona.".to_owned()),
        })
        .await
        .expect("custom agent saved");
    state
        .threads
        .thread_store
        .set(
            "thread::snapshot-agent-bound",
            json!({
                "thread_id": "thread::snapshot-agent-bound",
                "thread_mode": "single_agent",
                "agent_id": "snapshot-agent",
                "channel": "api",
                "account_id": "main",
                "from_id": "api-user",
                "messages": [],
                "metadata": {
                    "model": "provider-default-v1",
                    "model_reasoning_effort": "high",
                    "model_service_tier": "flex",
                },
            }),
        )
        .await
        .unwrap();

    let prepared = prepare_chat_request(
        &state,
        ChatRequest {
            thread_id: Some("thread::snapshot-agent-bound".to_owned()),
            message: "Continue".to_owned(),
            attachments: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
            client_intent_id: None,
            from_id: "api-user".to_owned(),
            account_id: "main".to_owned(),
            bot: None,
            wait_for_response: true,
            workspace_path: None,
            provider_type: None,
            metadata: HashMap::new(),
        },
    )
    .await
    .expect("prepare chat request");

    assert_eq!(
        prepared.metadata.get("model").and_then(Value::as_str),
        Some("provider-default-v1")
    );
    assert_eq!(
        prepared
            .metadata
            .get("model_reasoning_effort")
            .and_then(Value::as_str),
        Some("high")
    );
    assert_eq!(
        prepared
            .metadata
            .get("model_service_tier")
            .and_then(Value::as_str),
        Some("flex")
    );
    assert_eq!(
        prepared
            .metadata
            .get("agent_display_name")
            .and_then(Value::as_str),
        Some("Snapshot Agent")
    );
    assert_eq!(
        prepared
            .metadata
            .get("system_prompt")
            .and_then(Value::as_str),
        Some("Use the agent persona.")
    );
}

/// External chat boundary guard: `provider_env` is reserved for server-side
/// runtime resolution. A chat request that smuggles it through `metadata`
/// must have it stripped in prepare — otherwise the bridge's existing-wins
/// backfill would let the client value silently block the agent/thread
/// snapshot env (the same failure class as the removed `providerMetadata`
/// channel, through the front door).
#[tokio::test]
async fn prepare_chat_request_strips_reserved_provider_env_from_request_metadata() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::reserved-env",
            json!({
                "thread_id": "thread::reserved-env",
                "channel": "api",
                "account_id": "main",
                "from_id": "api-user",
                "messages": [],
            }),
        )
        .await
        .unwrap();

    let mut metadata = HashMap::new();
    metadata.insert(
        "provider_env".to_owned(),
        json!({ "ANTHROPIC_BASE_URL": "http://127.0.0.1:19999" }),
    );
    metadata.insert("client_note".to_owned(), json!("kept"));

    let prepared = prepare_chat_request(
        &state,
        ChatRequest {
            thread_id: Some("thread::reserved-env".to_owned()),
            message: "hello".to_owned(),
            attachments: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
            client_intent_id: None,
            from_id: "api-user".to_owned(),
            account_id: "main".to_owned(),
            bot: None,
            wait_for_response: true,
            workspace_path: None,
            provider_type: Some(ProviderType::ClaudeCode),
            metadata,
        },
    )
    .await
    .expect("prepare chat request");

    assert!(
        !prepared.metadata.contains_key("provider_env"),
        "client-supplied provider_env must be stripped at the chat boundary"
    );
    assert_eq!(
        prepared.metadata.get("client_note").and_then(Value::as_str),
        Some("kept"),
        "non-reserved client metadata must pass through"
    );
}

#[tokio::test]
async fn prepare_chat_request_binds_explicit_api_thread_to_from_id() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set(
            "thread::api-explicit",
            json!({
                "thread_id": "thread::api-explicit",
                "label": "Existing API thread",
                "channel": "api",
                "account_id": "main",
                "from_id": "old-api-user",
                "messages": [],
                "workspace_dir": "/repo",
                "channel_bindings": [],
            }),
        )
        .await
        .unwrap();

    let prepared = prepare_chat_request(
        &state,
        ChatRequest {
            thread_id: Some("thread::api-explicit".to_owned()),
            message: "Continue this thread".to_owned(),
            attachments: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
            client_intent_id: None,
            from_id: "mobile-client".to_owned(),
            account_id: "main".to_owned(),
            bot: None,
            wait_for_response: true,
            workspace_path: None,
            provider_type: None,
            metadata: HashMap::new(),
        },
    )
    .await
    .expect("prepare chat request");

    assert_eq!(prepared.thread_id, "thread::api-explicit");
    let updated = state
        .threads
        .thread_store
        .get("thread::api-explicit")
        .await
        .unwrap()
        .expect("thread exists");
    let binding = garyx_router::bindings_from_value(&updated)
        .into_iter()
        .find(|binding| binding.endpoint_key() == "api::main::mobile-client")
        .expect("api binding persisted");
    assert_eq!(binding.chat_id, "mobile-client");
    assert_eq!(binding.delivery_target_type, "chat_id");
    assert_eq!(binding.delivery_target_id, "mobile-client");
    assert_eq!(binding.display_label, "api/main/mobile-client");
    assert!(binding.last_inbound_at.is_some());

    let endpoint = state
        .ops
        .garyx_db
        .list_thread_channel_endpoints()
        .expect("endpoint projection")
        .into_iter()
        .find(|endpoint| endpoint.endpoint_key == "api::main::mobile-client")
        .expect("api endpoint projected");
    assert_eq!(endpoint.thread_id.as_deref(), Some("thread::api-explicit"));
    assert_eq!(endpoint.workspace_dir.as_deref(), Some("/repo"));
}

#[test]
fn merge_thread_model_cells_applies_legacy_override_keys() {
    let thread_data = json!({
        "metadata": {
            "model_override": "claude-opus-4-7",
            "model_reasoning_effort_override": "xhigh",
            "model": "stale-snapshot-model",
        }
    });
    let mut run_metadata = HashMap::new();
    merge_thread_model_cells(&thread_data, &mut run_metadata);
    assert_eq!(
        run_metadata.get("model"),
        Some(&Value::String("claude-opus-4-7".to_owned()))
    );
    assert_eq!(
        run_metadata.get("model_reasoning_effort"),
        Some(&Value::String("xhigh".to_owned()))
    );
    assert!(!run_metadata.contains_key("model_service_tier"));
}

#[test]
fn merge_thread_model_cells_keeps_request_metadata_priority() {
    let thread_data = json!({
        "metadata": {
            "model_override": "claude-opus-4-7",
            "model_reasoning_effort_override": "low",
        }
    });
    let mut run_metadata = HashMap::from([(
        "model".to_owned(),
        Value::String("request-model".to_owned()),
    )]);
    merge_thread_model_cells(&thread_data, &mut run_metadata);
    assert_eq!(
        run_metadata.get("model"),
        Some(&Value::String("request-model".to_owned()))
    );
    assert_eq!(
        run_metadata.get("model_reasoning_effort"),
        Some(&Value::String("low".to_owned()))
    );
}

#[test]
fn merge_thread_model_cells_ignores_blank_and_missing_values() {
    let thread_data = json!({
        "metadata": {
            "model_override": "   ",
        }
    });
    let mut run_metadata = HashMap::new();
    merge_thread_model_cells(&thread_data, &mut run_metadata);
    assert!(run_metadata.is_empty());

    let mut run_metadata = HashMap::new();
    merge_thread_model_cells(&json!({}), &mut run_metadata);
    assert!(run_metadata.is_empty());
}
