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
            default_workspace_dir: None,
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
            default_workspace_dir: Some("  /tmp/reviewer  ".to_owned()),
            system_prompt: "Review carefully.".to_owned(),
        })
        .await
        .expect("create agent");
    assert_eq!(
        created.default_workspace_dir.as_deref(),
        Some("/tmp/reviewer")
    );

    let updated = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: "gpt-5".to_owned(),
            default_workspace_dir: None,
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
            default_workspace_dir: Some("  ".to_owned()),
            system_prompt: "Review carefully.".to_owned(),
        })
        .await
        .expect("clear agent workspace");
    assert!(cleared.default_workspace_dir.is_none());
}
