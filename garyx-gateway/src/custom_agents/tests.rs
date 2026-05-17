use super::*;

#[tokio::test]
async fn lists_only_provider_builtin_agents() {
    let store = CustomAgentStore::new();
    let agents = store.list_agents().await;
    assert!(agents.iter().any(|agent| agent.agent_id == "claude"));
    assert!(agents.iter().any(|agent| agent.agent_id == "codex"));
    assert!(agents.iter().any(|agent| agent.agent_id == "gemini"));
    assert!(!agents.iter().any(|agent| agent.agent_id == "planner"));
    assert!(!agents.iter().any(|agent| agent.agent_id == "generator"));
    assert!(!agents.iter().any(|agent| agent.agent_id == "reviewer"));
}

#[tokio::test]
async fn rejects_builtin_agent_modification() {
    let store = CustomAgentStore::new();
    let error = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "claude".to_owned(),
            display_name: "Claude Override".to_owned(),
            provider_type: ProviderType::ClaudeCode,
            model: "claude-opus-4-1".to_owned(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: "Override".to_owned(),
        })
        .await
        .expect_err("built-in upsert should fail");
    assert_eq!(error, "built-in agents cannot be modified");
}

#[tokio::test]
async fn upsert_preserves_and_clears_default_workspace_dir() {
    let store = CustomAgentStore::new();
    let created = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: "gpt-5".to_owned(),
            model_reasoning_effort: "high".to_owned(),
            model_service_tier: "priority".to_owned(),
            default_workspace_dir: Some("  /tmp/reviewer  ".to_owned()),
            avatar_data_url: None,
            system_prompt: "Review carefully.".to_owned(),
        })
        .await
        .expect("create agent");
    assert_eq!(
        created.default_workspace_dir.as_deref(),
        Some("/tmp/reviewer")
    );
    assert_eq!(created.model_reasoning_effort, "high");
    assert_eq!(created.model_service_tier, "priority");

    let updated = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: "gpt-5".to_owned(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: "Review carefully.".to_owned(),
        })
        .await
        .expect("update agent");
    assert_eq!(
        updated.default_workspace_dir.as_deref(),
        Some("/tmp/reviewer")
    );

    let cleared = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: "gpt-5".to_owned(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            default_workspace_dir: Some("  ".to_owned()),
            avatar_data_url: None,
            system_prompt: "Review carefully.".to_owned(),
        })
        .await
        .expect("clear agent workspace");
    assert!(cleared.default_workspace_dir.is_none());
}

#[tokio::test]
async fn upsert_preserves_and_clears_avatar_data_url() {
    let store = CustomAgentStore::new();
    let created = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "designer".to_owned(),
            display_name: "Designer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: "gpt-5".to_owned(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            default_workspace_dir: None,
            avatar_data_url: Some("  data:image/png;base64,dGVzdA==  ".to_owned()),
            system_prompt: "Design carefully.".to_owned(),
        })
        .await
        .expect("create agent");
    assert_eq!(
        created.avatar_data_url.as_deref(),
        Some("data:image/png;base64,dGVzdA==")
    );

    let updated = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "designer".to_owned(),
            display_name: "Designer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: "gpt-5".to_owned(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: "Design carefully.".to_owned(),
        })
        .await
        .expect("update agent");
    assert_eq!(
        updated.avatar_data_url.as_deref(),
        Some("data:image/png;base64,dGVzdA==")
    );

    let cleared = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "designer".to_owned(),
            display_name: "Designer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: "gpt-5".to_owned(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            default_workspace_dir: None,
            avatar_data_url: Some("  ".to_owned()),
            system_prompt: "Design carefully.".to_owned(),
        })
        .await
        .expect("clear agent avatar");
    assert!(cleared.avatar_data_url.is_none());
}
