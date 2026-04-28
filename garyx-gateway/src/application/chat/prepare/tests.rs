use super::*;
use crate::agent_teams::AgentTeamStore;
use crate::custom_agents::CustomAgentStore;
use crate::server::AppStateBuilder;
use garyx_models::config::{GaryxConfig, McpServerConfig, SlashCommand};
use serde_json::json;

fn test_state() -> Arc<AppState> {
    AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(Arc::new(CustomAgentStore::new()))
        .with_agent_team_store(Arc::new(AgentTeamStore::new()))
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
    let mut provider_metadata = HashMap::new();
    provider_metadata.insert(
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
        HashMap::new(),
        provider_metadata,
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
}

#[tokio::test]
async fn prepare_chat_request_resolves_provider_and_system_prompt_from_thread_agent() {
    let state = test_state();
    state
        .ops
        .custom_agents
        .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "spec-review".to_owned(),
            display_name: "Spec Review".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: "gpt-5-codex".to_owned(),
            system_prompt: "Review specs carefully.".to_owned(),
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
                "channel_bindings": []
            }),
        )
        .await;

    let prepared = prepare_chat_request(
        &state,
        ChatRequest {
            thread_id: Some("thread::agent-bound".to_owned()),
            message: "Check this design".to_owned(),
            attachments: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
            from_id: "api-user".to_owned(),
            account_id: "main".to_owned(),
            wait_for_response: true,
            workspace_path: None,
            provider_type: Some(ProviderType::ClaudeCode),
            metadata: HashMap::new(),
            provider_metadata: HashMap::new(),
        },
    )
    .await
    .expect("prepare chat request");

    assert_eq!(prepared.provider_type, Some(ProviderType::CodexAppServer));
    assert_eq!(
        prepared
            .provider_metadata
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
            .get("agent_display_name")
            .and_then(Value::as_str),
        Some("Spec Review")
    );
}
