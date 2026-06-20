use super::*;

fn model_contract_request(
    agent_id: &str,
    model: Option<&str>,
    model_reasoning_effort: Option<&str>,
    model_service_tier: Option<&str>,
) -> UpsertCustomAgentRequest {
    UpsertCustomAgentRequest {
        agent_id: agent_id.to_owned(),
        display_name: "Reviewer".to_owned(),
        provider_type: ProviderType::CodexAppServer,
        model: model.map(str::to_owned),
        model_reasoning_effort: model_reasoning_effort.map(str::to_owned),
        model_service_tier: model_service_tier.map(str::to_owned),
        provider_env: None,
        auth_source: None,
        base_url: None,
        codex_home: None,
        max_tool_iterations: None,
        request_timeout_seconds: None,
        default_workspace_dir: None,
        avatar_data_url: None,
        system_prompt: "Review carefully.".to_owned(),
    }
}

#[tokio::test]
async fn lists_only_provider_builtin_agents() {
    let store = CustomAgentStore::new();
    let agents = store.list_agents().await;
    assert!(agents.iter().any(|agent| agent.agent_id == "claude"));
    assert!(agents.iter().any(|agent| agent.agent_id == "codex"));
    assert!(agents.iter().any(|agent| agent.agent_id == "gemini"));
    assert!(agents.iter().filter(|agent| agent.built_in).all(|agent| {
        agent
            .avatar_data_url
            .as_deref()
            .is_some_and(|value| value.starts_with("data:image/png;base64,"))
    }));
    assert!(!agents.iter().any(|agent| agent.agent_id == "gpt"));
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
            model: Some("claude-opus-4-1".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: "Override".to_owned(),
        })
        .await
        .expect_err("built-in upsert should fail");
    assert_eq!(error, "built-in agents cannot be modified");
}

#[tokio::test]
async fn upsert_without_model_fields_preserves_existing_model_settings() {
    let store = CustomAgentStore::new();
    store
        .upsert_agent(model_contract_request(
            "reviewer",
            Some("gpt-5"),
            Some("high"),
            Some("priority"),
        ))
        .await
        .expect("create agent");

    let updated = store
        .upsert_agent(model_contract_request("reviewer", None, None, None))
        .await
        .expect("update agent");

    assert_eq!(updated.model, "gpt-5");
    assert_eq!(updated.model_reasoning_effort, "high");
    assert_eq!(updated.model_service_tier, "priority");
}

#[tokio::test]
async fn upsert_with_empty_model_fields_clears_existing_model_settings() {
    let store = CustomAgentStore::new();
    store
        .upsert_agent(model_contract_request(
            "reviewer",
            Some("gpt-5"),
            Some("high"),
            Some("priority"),
        ))
        .await
        .expect("create agent");

    let updated = store
        .upsert_agent(model_contract_request(
            "reviewer",
            Some(""),
            Some(""),
            Some(""),
        ))
        .await
        .expect("update agent");

    assert_eq!(updated.model, "");
    assert_eq!(updated.model_reasoning_effort, "");
    assert_eq!(updated.model_service_tier, "");
}

#[tokio::test]
async fn upsert_with_model_fields_replaces_existing_model_settings() {
    let store = CustomAgentStore::new();
    store
        .upsert_agent(model_contract_request(
            "reviewer",
            Some("gpt-5"),
            Some("high"),
            Some("priority"),
        ))
        .await
        .expect("create agent");

    let updated = store
        .upsert_agent(model_contract_request(
            "reviewer",
            Some(" claude-opus-4-8 "),
            Some(" max "),
            Some(" flex "),
        ))
        .await
        .expect("update agent");

    assert_eq!(updated.model, "claude-opus-4-8");
    assert_eq!(updated.model_reasoning_effort, "max");
    assert_eq!(updated.model_service_tier, "flex");
}

#[tokio::test]
async fn upsert_create_without_model_fields_stores_provider_default_settings() {
    let store = CustomAgentStore::new();
    let created = store
        .upsert_agent(model_contract_request("reviewer", None, None, None))
        .await
        .expect("create agent");

    assert_eq!(created.model, "");
    assert_eq!(created.model_reasoning_effort, "");
    assert_eq!(created.model_service_tier, "");
}

#[tokio::test]
async fn upsert_preserves_and_clears_default_workspace_dir() {
    let store = CustomAgentStore::new();
    let created = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some("high".to_owned()),
            model_service_tier: Some("priority".to_owned()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
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
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
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
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
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
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
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
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
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
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
            default_workspace_dir: None,
            avatar_data_url: Some("  ".to_owned()),
            system_prompt: "Design carefully.".to_owned(),
        })
        .await
        .expect("clear agent avatar");
    assert!(cleared.avatar_data_url.is_none());
}

#[tokio::test]
async fn upsert_persists_and_preserves_provider_auth_config() {
    let store = CustomAgentStore::new();
    let created = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "budget-gpt".to_owned(),
            display_name: "Budget GPT".to_owned(),
            provider_type: ProviderType::Gpt,
            model: Some("gpt-5.5".to_owned()),
            model_reasoning_effort: Some("medium".to_owned()),
            model_service_tier: Some(String::new()),
            provider_env: Some(HashMap::from([(
                " OPENAI_API_KEY ".to_owned(),
                " test-api-key ".to_owned(),
            )])),
            auth_source: Some(" api_key ".to_owned()),
            base_url: Some(" https://example.invalid/v1 ".to_owned()),
            codex_home: None,
            max_tool_iterations: Some(24),
            request_timeout_seconds: Some(120),
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: "Use GPT.".to_owned(),
        })
        .await
        .expect("create native agent");

    assert_eq!(created.auth_source, "api_key");
    assert_eq!(created.base_url, "https://example.invalid/v1");
    assert_eq!(created.max_tool_iterations, 24);
    assert_eq!(created.request_timeout_seconds, 120);
    assert_eq!(
        created
            .provider_env
            .get("OPENAI_API_KEY")
            .map(String::as_str),
        Some("test-api-key")
    );

    let updated = store
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: "budget-gpt".to_owned(),
            display_name: "Budget GPT".to_owned(),
            provider_type: ProviderType::Gpt,
            model: Some("gpt-5.5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: "Use GPT.".to_owned(),
        })
        .await
        .expect("update native agent");

    assert_eq!(updated.auth_source, "api_key");
    assert_eq!(
        updated
            .provider_env
            .get("OPENAI_API_KEY")
            .map(String::as_str),
        Some("test-api-key")
    );
}
