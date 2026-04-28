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
            system_prompt: "Override".to_owned(),
        })
        .await
        .expect_err("built-in upsert should fail");
    assert_eq!(error, "built-in agents cannot be modified");
}
