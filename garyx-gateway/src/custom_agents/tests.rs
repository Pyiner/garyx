use std::sync::Arc;

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
        enabled: None,
        model: model.map(str::to_owned),
        model_reasoning_effort: model_reasoning_effort.map(str::to_owned),
        model_service_tier: model_service_tier.map(str::to_owned),
        provider_env: None,
        default_workspace_dir: None,
        avatar_data_url: None,
        system_prompt: Some("Review carefully.".to_owned()),
    }
}

#[tokio::test]
async fn lists_only_provider_builtin_agents() {
    let store = CustomAgentStore::new();
    let agents = store.list_agents().await;
    assert!(agents.iter().any(|agent| agent.agent_id == "claude"));
    assert!(agents.iter().any(|agent| agent.agent_id == "codex"));
    assert!(agents.iter().any(|agent| agent.agent_id == "traex"));
    assert!(agents.iter().any(|agent| agent.agent_id == "antigravity"));
    assert!(agents.iter().any(|agent| agent.agent_id == "grok"));
    assert!(agents.iter().filter(|agent| agent.built_in).all(|agent| {
        agent
            .avatar_data_url
            .as_deref()
            .is_some_and(|value| value.starts_with("data:image/png;base64,"))
    }));
    assert!(
        !agents
            .iter()
            .any(|agent| agent.agent_id == "removed-provider")
    );
    assert!(!agents.iter().any(|agent| agent.agent_id == "planner"));
    assert!(!agents.iter().any(|agent| agent.agent_id == "generator"));
    assert!(!agents.iter().any(|agent| agent.agent_id == "reviewer"));
}

#[tokio::test]
async fn file_store_skips_profiles_with_unsupported_provider_types() {
    let seed = CustomAgentStore::new();
    let supported = seed
        .upsert_agent_for_test(model_contract_request(
            "supported-reviewer",
            Some("gpt-5"),
            None,
            None,
        ))
        .await
        .expect("supported profile");
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("custom-agents.json");
    let mut legacy_supported = serde_json::to_value(&supported).expect("serialize profile");
    legacy_supported
        .as_object_mut()
        .expect("profile object")
        .remove("enabled");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "supported-reviewer": legacy_supported,
            "removed-reviewer": {
                "provider_type": "removed_provider"
            }
        }))
        .expect("serialize persisted agents"),
    )
    .expect("write persisted agents");

    let store = CustomAgentStore::file(&path).expect("load agent store");
    let agents = store.list_agents().await;

    assert!(
        agents
            .iter()
            .any(|agent| agent.agent_id == "supported-reviewer")
    );
    assert!(
        agents
            .iter()
            .find(|agent| agent.agent_id == "supported-reviewer")
            .unwrap()
            .enabled,
        "legacy profiles without enabled migrate as enabled"
    );
    assert!(
        !agents
            .iter()
            .any(|agent| agent.agent_id == "removed-reviewer")
    );
    let migrated: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read migrated envelope"))
            .expect("parse migrated envelope");
    assert_eq!(migrated["version"], AGENT_STORE_VERSION);
    assert!(migrated["agents"].is_object());
}

#[tokio::test]
async fn rejects_builtin_agent_modification() {
    let store = CustomAgentStore::new();
    let error = store
        .upsert_agent_for_test(UpsertCustomAgentRequest {
            agent_id: "claude".to_owned(),
            display_name: "Claude Override".to_owned(),
            provider_type: ProviderType::ClaudeCode,
            enabled: None,
            model: Some("claude-opus-4-1".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: Some("Override".to_owned()),
        })
        .await
        .expect_err("built-in upsert should fail");
    assert_eq!(error, "built-in agents cannot be modified");
}

#[tokio::test]
async fn upsert_without_model_fields_preserves_existing_model_settings() {
    let store = CustomAgentStore::new();
    store
        .upsert_agent_for_test(model_contract_request(
            "reviewer",
            Some("gpt-5"),
            Some("high"),
            Some("priority"),
        ))
        .await
        .expect("create agent");

    let updated = store
        .upsert_agent_for_test(model_contract_request("reviewer", None, None, None))
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
        .upsert_agent_for_test(model_contract_request(
            "reviewer",
            Some("gpt-5"),
            Some("high"),
            Some("priority"),
        ))
        .await
        .expect("create agent");

    let updated = store
        .upsert_agent_for_test(model_contract_request(
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
        .upsert_agent_for_test(model_contract_request(
            "reviewer",
            Some("gpt-5"),
            Some("high"),
            Some("priority"),
        ))
        .await
        .expect("create agent");

    let updated = store
        .upsert_agent_for_test(model_contract_request(
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
        .upsert_agent_for_test(model_contract_request("reviewer", None, None, None))
        .await
        .expect("create agent");

    assert_eq!(created.model, "");
    assert_eq!(created.model_reasoning_effort, "");
    assert_eq!(created.model_service_tier, "");
}

#[tokio::test]
async fn upsert_create_without_system_prompt_stores_unset_prompt() {
    let store = CustomAgentStore::new();
    let mut request = model_contract_request("reviewer", None, None, None);
    request.system_prompt = None;

    let created = store
        .upsert_agent_for_test(request)
        .await
        .expect("create agent");

    assert_eq!(created.system_prompt, "");
}

#[tokio::test]
async fn upsert_create_with_blank_system_prompt_stores_unset_prompt() {
    let store = CustomAgentStore::new();
    let mut request = model_contract_request("reviewer", None, None, None);
    request.system_prompt = Some("   \n\t ".to_owned());

    let created = store
        .upsert_agent_for_test(request)
        .await
        .expect("create agent");

    assert_eq!(created.system_prompt, "");
}

#[tokio::test]
async fn upsert_without_system_prompt_preserves_existing_prompt() {
    let store = CustomAgentStore::new();
    store
        .upsert_agent_for_test(model_contract_request("reviewer", None, None, None))
        .await
        .expect("create agent");
    let mut request = model_contract_request("reviewer", None, None, None);
    request.system_prompt = None;

    let updated = store
        .upsert_agent_for_test(request)
        .await
        .expect("update agent");

    assert_eq!(updated.system_prompt, "Review carefully.");
}

#[tokio::test]
async fn upsert_with_blank_system_prompt_clears_existing_prompt() {
    let store = CustomAgentStore::new();
    store
        .upsert_agent_for_test(model_contract_request("reviewer", None, None, None))
        .await
        .expect("create agent");
    let mut request = model_contract_request("reviewer", None, None, None);
    request.system_prompt = Some("   ".to_owned());

    let updated = store
        .upsert_agent_for_test(request)
        .await
        .expect("update agent");

    assert_eq!(updated.system_prompt, "");
}

#[tokio::test]
async fn upsert_with_system_prompt_replaces_existing_prompt() {
    let store = CustomAgentStore::new();
    store
        .upsert_agent_for_test(model_contract_request("reviewer", None, None, None))
        .await
        .expect("create agent");
    let mut request = model_contract_request("reviewer", None, None, None);
    request.system_prompt = Some("  Review tersely.  ".to_owned());

    let updated = store
        .upsert_agent_for_test(request)
        .await
        .expect("update agent");

    assert_eq!(updated.system_prompt, "Review tersely.");
}

#[tokio::test]
async fn upsert_preserves_and_clears_default_workspace_dir() {
    let store = CustomAgentStore::new();
    let created = store
        .upsert_agent_for_test(UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            enabled: None,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some("high".to_owned()),
            model_service_tier: Some("priority".to_owned()),
            provider_env: None,
            default_workspace_dir: Some("  /tmp/reviewer  ".to_owned()),
            avatar_data_url: None,
            system_prompt: Some("Review carefully.".to_owned()),
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
        .upsert_agent_for_test(UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            enabled: None,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: Some("Review carefully.".to_owned()),
        })
        .await
        .expect("update agent");
    assert_eq!(
        updated.default_workspace_dir.as_deref(),
        Some("/tmp/reviewer")
    );

    let cleared = store
        .upsert_agent_for_test(UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            enabled: None,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            default_workspace_dir: Some("  ".to_owned()),
            avatar_data_url: None,
            system_prompt: Some("Review carefully.".to_owned()),
        })
        .await
        .expect("clear agent workspace");
    assert!(cleared.default_workspace_dir.is_none());
}

#[tokio::test]
async fn upsert_preserves_and_clears_avatar_data_url() {
    let store = CustomAgentStore::new();
    let created = store
        .upsert_agent_for_test(UpsertCustomAgentRequest {
            agent_id: "designer".to_owned(),
            display_name: "Designer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            enabled: None,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            default_workspace_dir: None,
            avatar_data_url: Some("  data:image/png;base64,dGVzdA==  ".to_owned()),
            system_prompt: Some("Design carefully.".to_owned()),
        })
        .await
        .expect("create agent");
    assert_eq!(
        created.avatar_data_url.as_deref(),
        Some("data:image/png;base64,dGVzdA==")
    );

    let updated = store
        .upsert_agent_for_test(UpsertCustomAgentRequest {
            agent_id: "designer".to_owned(),
            display_name: "Designer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            enabled: None,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: Some("Design carefully.".to_owned()),
        })
        .await
        .expect("update agent");
    assert_eq!(
        updated.avatar_data_url.as_deref(),
        Some("data:image/png;base64,dGVzdA==")
    );

    let cleared = store
        .upsert_agent_for_test(UpsertCustomAgentRequest {
            agent_id: "designer".to_owned(),
            display_name: "Designer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            enabled: None,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            default_workspace_dir: None,
            avatar_data_url: Some("  ".to_owned()),
            system_prompt: Some("Design carefully.".to_owned()),
        })
        .await
        .expect("clear agent avatar");
    assert!(cleared.avatar_data_url.is_none());
}

/// Regression guard for the 2026-07-06 gary incident: mutations used to
/// release the write lock before `persist()` re-acquired a read lock for its
/// snapshot, so writer A could serialize a pre-B snapshot and flush it to
/// disk *after* B's newer snapshot landed — silently dropping B's agent from
/// `custom-agents.json`. Persisting inside the mutation's critical section
/// makes disk writes strictly ordered with mutations, so the reloaded file
/// must always contain every concurrently upserted agent.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_writers_never_lose_each_others_agents() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("custom-agents.json");
    let store = Arc::new(CustomAgentStore::file(&path).expect("file store"));

    let mut handles = Vec::new();
    for index in 0..32 {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            store
                .upsert_agent_for_test(model_contract_request(
                    &format!("agent-{index}"),
                    None,
                    None,
                    None,
                ))
                .await
                .expect("upsert agent");
        }));
    }
    for handle in handles {
        handle.await.expect("join upsert task");
    }

    let reloaded = CustomAgentStore::file(&path).expect("reload persisted store");
    let agents = reloaded.list_agents().await;
    for index in 0..32 {
        let agent_id = format!("agent-{index}");
        assert!(
            agents.iter().any(|agent| agent.agent_id == agent_id),
            "{agent_id} was lost from the persisted file by a concurrent writer"
        );
    }
}

#[tokio::test]
async fn envelope_round_trip_preserves_enabled_builtins_custom_and_raw_default() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("custom-agents.json");
    let store = CustomAgentStore::file(&path).expect("file store");
    store
        .set_enabled("claude", false)
        .await
        .expect("disable builtin");
    let mut request = model_contract_request("reviewer", Some("gpt-5"), None, None);
    request.enabled = Some(false);
    store
        .upsert_agent_for_test(request)
        .await
        .expect("disabled custom agent");
    store
        .set_default_agent("codex")
        .await
        .expect("set raw default");

    let reloaded = CustomAgentStore::file(&path).expect("reload envelope");
    assert!(!reloaded.get_agent("claude").await.unwrap().enabled);
    assert!(!reloaded.get_agent("reviewer").await.unwrap().enabled);
    assert_eq!(reloaded.default_agent_id().await.as_deref(), Some("codex"));
    assert_eq!(
        reloaded.effective_default_agent_id().await.as_deref(),
        Some("codex")
    );
}

#[tokio::test]
async fn delete_raw_default_clears_it_in_the_same_persisted_mutation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("custom-agents.json");
    let store = CustomAgentStore::file(&path).expect("file store");
    store
        .upsert_agent_for_test(model_contract_request("reviewer", None, None, None))
        .await
        .expect("custom agent");
    store
        .set_default_agent("reviewer")
        .await
        .expect("set default");
    store
        .delete_agent("reviewer")
        .await
        .expect("delete default");

    assert!(store.default_agent_id().await.is_none());
    let reloaded = CustomAgentStore::file(&path).expect("reload");
    assert!(reloaded.default_agent_id().await.is_none());
    assert!(reloaded.get_agent("reviewer").await.is_none());
}

#[tokio::test]
async fn persist_failure_leaves_memory_revision_and_disk_state_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("custom-agents.json");
    let store = CustomAgentStore::file(&path).expect("file store");
    store
        .set_default_agent("codex")
        .await
        .expect("seed envelope");
    let before = store.snapshot().await;
    let before_disk = std::fs::read(&path).expect("read before");

    // A directory at the sibling temp path makes the atomic write fail while
    // leaving the previously committed destination file untouched.
    let mut blocking_tmp = path.as_os_str().to_owned();
    blocking_tmp.push(".tmp");
    let blocking_tmp = std::path::PathBuf::from(blocking_tmp);
    std::fs::create_dir(&blocking_tmp).expect("create blocking temp directory");
    let error = store
        .set_enabled("claude", false)
        .await
        .expect_err("persist must fail");
    assert!(matches!(error, StoreWriteError::Persist(_)));
    assert_eq!(store.snapshot().await, before);
    assert_eq!(
        std::fs::read(&path).expect("read unchanged disk state"),
        before_disk
    );
    std::fs::remove_dir(&blocking_tmp).expect("remove blocking temp directory");
    let reloaded = CustomAgentStore::file(&path).expect("reload old disk state");
    assert_eq!(reloaded.default_agent_id().await.as_deref(), Some("codex"));
    assert!(reloaded.get_agent("claude").await.unwrap().enabled);
}

#[tokio::test]
async fn idempotent_toggle_does_not_advance_revision_but_real_mutations_do() {
    let store = CustomAgentStore::new();
    let initial = store.snapshot().await.agent_state_revision;
    store
        .set_enabled("claude", true)
        .await
        .expect("idempotent set");
    assert_eq!(store.snapshot().await.agent_state_revision, initial);
    store.set_enabled("claude", false).await.expect("disable");
    assert_eq!(store.snapshot().await.agent_state_revision, initial + 1);
}
