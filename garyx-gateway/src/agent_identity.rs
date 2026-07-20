use std::sync::Arc;

use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_models::{
    AgentBindingError, AgentReference, ResolvedAgentBinding, SERVER_OWNED_AGENT_METADATA_KEYS,
    agent_runtime_snapshot_metadata, resolve_agent_binding, resolve_agent_reference,
};
use garyx_router::{
    ThreadCreationError, ThreadCreator, ThreadEnsureOptions, ThreadStore, create_thread_record,
    prepare_thread_record, workspace_dir_from_value,
};
use serde_json::Value;

use crate::custom_agents::CustomAgentStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentBindingIntent {
    Fresh,
    Fork,
    RecoverExistingSession,
}

pub(crate) async fn resolve_agent_reference_from_stores(
    custom_agents: &CustomAgentStore,
    requested_id: &str,
) -> Result<AgentReference, String> {
    let agents = custom_agents.list_agents().await;
    resolve_agent_reference(requested_id, &agents)
}

pub(crate) async fn resolve_new_agent_binding_from_store(
    custom_agents: &CustomAgentStore,
    requested: Option<&str>,
) -> Result<ResolvedAgentBinding, AgentBindingError> {
    resolve_agent_binding(&custom_agents.snapshot().await, requested, None)
}

pub(crate) fn default_workspace_dir_from_agent_reference(
    reference: &AgentReference,
) -> Option<String> {
    match reference {
        AgentReference::Standalone { profile, .. } => profile
            .default_workspace_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    }
}

pub(crate) use garyx_models::agent_runtime_metadata;

pub(crate) fn merge_agent_runtime_snapshot_metadata(
    metadata: &mut std::collections::HashMap<String, Value>,
    reference: &AgentReference,
) {
    for (key, value) in agent_runtime_snapshot_metadata(reference) {
        if SERVER_OWNED_AGENT_METADATA_KEYS.contains(&key.as_str()) {
            metadata.insert(key, value);
        } else {
            metadata.entry(key).or_insert(value);
        }
    }
}

pub(crate) fn snapshot_agent_runtime_metadata_to_thread_record(
    record: &mut Value,
    reference: &AgentReference,
) -> Result<(), String> {
    let obj = record
        .as_object_mut()
        .ok_or_else(|| "thread record is not an object".to_owned())?;
    let metadata_value = obj
        .entry("metadata".to_owned())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !metadata_value.is_object() {
        *metadata_value = Value::Object(serde_json::Map::new());
    }
    let metadata = metadata_value
        .as_object_mut()
        .ok_or_else(|| "thread metadata is not an object".to_owned())?;
    for (key, value) in agent_runtime_snapshot_metadata(reference) {
        if SERVER_OWNED_AGENT_METADATA_KEYS.contains(&key.as_str()) {
            metadata.insert(key, value);
        } else {
            metadata.entry(key).or_insert(value);
        }
    }
    Ok(())
}

pub(crate) async fn create_thread_for_agent_reference(
    thread_store: Arc<dyn ThreadStore>,
    bridge: Arc<MultiProviderBridge>,
    custom_agents: Arc<CustomAgentStore>,
    options: ThreadEnsureOptions,
    intent: AgentBindingIntent,
) -> Result<(String, Value, AgentReference), ThreadCreationError> {
    let (canonical_options, resolved) =
        canonical_thread_options(custom_agents.as_ref(), options, intent).await?;
    let (thread_id, data) = create_thread_record(&thread_store, canonical_options)
        .await
        .map_err(|error| {
            if error.starts_with("workspace_mode=worktree") {
                ThreadCreationError::Other(error)
            } else {
                ThreadCreationError::Storage(error)
            }
        })?;
    if let Some(workspace_dir) = workspace_dir_from_value(&data) {
        bridge
            .set_thread_workspace_binding(&thread_id, Some(workspace_dir))
            .await;
    }
    Ok((thread_id, data, resolved))
}

async fn canonical_thread_options(
    custom_agents: &CustomAgentStore,
    options: ThreadEnsureOptions,
    intent: AgentBindingIntent,
) -> Result<(ThreadEnsureOptions, AgentReference), ThreadCreationError> {
    let snapshot = custom_agents.snapshot().await;
    let binding = match intent {
        AgentBindingIntent::Fresh | AgentBindingIntent::Fork => {
            resolve_agent_binding(&snapshot, options.agent_id.as_deref(), None)
        }
        AgentBindingIntent::RecoverExistingSession => {
            resolve_agent_binding(&snapshot, None, options.agent_id.as_deref())
        }
    }?;
    let resolved = resolve_agent_reference(&binding.agent_id, &snapshot.agents)
        .map_err(ThreadCreationError::Other)?;

    let mut canonical_options = options;
    canonical_options.agent_id = Some(resolved.bound_agent_id().to_owned());
    canonical_options.provider_type = Some(resolved.provider_type());
    merge_agent_runtime_snapshot_metadata(&mut canonical_options.metadata, &resolved);
    if !canonical_options.no_workspace
        && canonical_options
            .workspace_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        canonical_options.workspace_dir = default_workspace_dir_from_agent_reference(&resolved);
    }

    Ok((canonical_options, resolved))
}

pub(crate) async fn prepare_thread_for_agent_reference(
    thread_id: &str,
    custom_agents: &CustomAgentStore,
    options: ThreadEnsureOptions,
    intent: AgentBindingIntent,
) -> Result<(Value, AgentReference), ThreadCreationError> {
    let (canonical_options, resolved) =
        canonical_thread_options(custom_agents, options, intent).await?;
    let data = prepare_thread_record(thread_id, canonical_options)
        .await
        .map_err(|error| {
            if error.starts_with("workspace_mode=worktree") {
                ThreadCreationError::Other(error)
            } else {
                ThreadCreationError::Storage(error)
            }
        })?;
    Ok((data, resolved))
}

pub(crate) struct GatewayThreadCreator {
    bridge: Arc<MultiProviderBridge>,
    custom_agents: Arc<CustomAgentStore>,
}

impl GatewayThreadCreator {
    pub(crate) fn new(
        bridge: Arc<MultiProviderBridge>,
        custom_agents: Arc<CustomAgentStore>,
    ) -> Self {
        Self {
            bridge,
            custom_agents,
        }
    }
}

#[async_trait]
impl ThreadCreator for GatewayThreadCreator {
    async fn create_thread(
        &self,
        thread_store: Arc<dyn ThreadStore>,
        options: ThreadEnsureOptions,
    ) -> Result<(String, Value), ThreadCreationError> {
        let (thread_id, data, _) = create_thread_for_agent_reference(
            thread_store,
            self.bridge.clone(),
            self.custom_agents.clone(),
            options,
            AgentBindingIntent::Fresh,
        )
        .await?;
        Ok((thread_id, data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use garyx_bridge::MultiProviderBridge;
    use garyx_models::ProviderType;
    use garyx_router::{
        InMemoryThreadStore, ThreadEnsureOptions, ThreadStore, thread_metadata_from_value,
    };

    async fn custom_agent_store_with_default_workspace() -> Arc<CustomAgentStore> {
        let store = Arc::new(CustomAgentStore::new());
        store
            .upsert_agent_for_test(crate::custom_agents::UpsertCustomAgentRequest {
                agent_id: "reviewer".to_owned(),
                display_name: "Reviewer".to_owned(),
                provider_type: ProviderType::CodexAppServer,
                enabled: None,
                model: Some("gpt-5".to_owned()),
                model_reasoning_effort: Some("high".to_owned()),
                model_service_tier: Some("priority".to_owned()),
                provider_env: Some(std::collections::HashMap::from([(
                    "OPENAI_BASE_URL".to_owned(),
                    "http://127.0.0.1:15721/v1".to_owned(),
                )])),
                default_workspace_dir: Some("/tmp/agent-default".to_owned()),
                avatar_data_url: None,
                system_prompt: Some("Review carefully.".to_owned()),
            })
            .await
            .expect("custom agent");
        store
    }

    #[tokio::test]
    async fn explicit_no_workspace_is_never_replaced_by_the_agent_default() {
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let custom_agents = custom_agent_store_with_default_workspace().await;
        let (_, data, _) = create_thread_for_agent_reference(
            thread_store,
            Arc::new(MultiProviderBridge::new()),
            custom_agents,
            ThreadEnsureOptions {
                agent_id: Some("reviewer".to_owned()),
                no_workspace: true,
                ..ThreadEnsureOptions::default()
            },
            AgentBindingIntent::Fresh,
        )
        .await
        .expect("thread created");
        assert_eq!(
            workspace_dir_from_value(&data),
            None,
            "an explicit No-workspace create must stay workspace-less so the \
             runtime provisions the private managed workspace",
        );
    }

    #[tokio::test]
    async fn create_thread_uses_agent_default_workspace_when_unset() {
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let custom_agents = custom_agent_store_with_default_workspace().await;
        let (thread_id, data, _) = create_thread_for_agent_reference(
            thread_store,
            Arc::new(MultiProviderBridge::new()),
            custom_agents,
            ThreadEnsureOptions {
                agent_id: Some("reviewer".to_owned()),
                ..ThreadEnsureOptions::default()
            },
            AgentBindingIntent::Fresh,
        )
        .await
        .expect("thread created");

        assert!(thread_id.starts_with("thread::"));
        assert_eq!(
            workspace_dir_from_value(&data).as_deref(),
            Some("/tmp/agent-default")
        );
    }

    #[tokio::test]
    async fn create_thread_explicit_workspace_overrides_agent_default() {
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let custom_agents = custom_agent_store_with_default_workspace().await;
        let (_, data, _) = create_thread_for_agent_reference(
            thread_store,
            Arc::new(MultiProviderBridge::new()),
            custom_agents,
            ThreadEnsureOptions {
                agent_id: Some("reviewer".to_owned()),
                workspace_dir: Some("/tmp/bot-workspace".to_owned()),
                ..ThreadEnsureOptions::default()
            },
            AgentBindingIntent::Fresh,
        )
        .await
        .expect("thread created");

        assert_eq!(
            workspace_dir_from_value(&data).as_deref(),
            Some("/tmp/bot-workspace")
        );
    }

    #[tokio::test]
    async fn create_thread_persists_expanded_agent_runtime_metadata() {
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let custom_agents = custom_agent_store_with_default_workspace().await;
        let (_, data, _) = create_thread_for_agent_reference(
            thread_store,
            Arc::new(MultiProviderBridge::new()),
            custom_agents,
            ThreadEnsureOptions {
                agent_id: Some("reviewer".to_owned()),
                ..ThreadEnsureOptions::default()
            },
            AgentBindingIntent::Fresh,
        )
        .await
        .expect("thread created");

        let metadata = thread_metadata_from_value(&data);
        assert_eq!(metadata["agent_id"], "reviewer");
        assert_eq!(metadata["agent_display_name"], "Reviewer");
        assert_eq!(metadata["requested_provider_type"], "codex_app_server");
        assert_eq!(metadata["model"], "gpt-5");
        assert_eq!(metadata["model_reasoning_effort"], "high");
        assert_eq!(metadata["model_service_tier"], "priority");
        assert_eq!(metadata["system_prompt"], "Review carefully.");
        assert_eq!(
            metadata["provider_env"]["OPENAI_BASE_URL"],
            "http://127.0.0.1:15721/v1"
        );
    }

    #[tokio::test]
    async fn fresh_and_fork_reject_disabled_but_recovery_preserves_existing_binding() {
        let custom_agents = Arc::new(CustomAgentStore::new());
        custom_agents
            .set_enabled("codex", false)
            .await
            .expect("disable codex");

        for intent in [AgentBindingIntent::Fresh, AgentBindingIntent::Fork] {
            let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
            let error = create_thread_for_agent_reference(
                store.clone(),
                Arc::new(MultiProviderBridge::new()),
                custom_agents.clone(),
                ThreadEnsureOptions {
                    agent_id: Some("codex".to_owned()),
                    ..ThreadEnsureOptions::default()
                },
                intent,
            )
            .await
            .expect_err("new binding to disabled agent must be rejected");
            assert_eq!(error.to_string(), "agent is disabled: codex");
            assert!(store.list_keys(None).await.unwrap().is_empty());
        }

        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let (_, recovered, resolved) = create_thread_for_agent_reference(
            store,
            Arc::new(MultiProviderBridge::new()),
            custom_agents,
            ThreadEnsureOptions {
                agent_id: Some("codex".to_owned()),
                ..ThreadEnsureOptions::default()
            },
            AgentBindingIntent::RecoverExistingSession,
        )
        .await
        .expect("recovery keeps an existing disabled binding");
        assert_eq!(resolved.bound_agent_id(), "codex");
        assert_eq!(recovered["agent_id"], "codex");
    }

    #[tokio::test]
    async fn global_default_hot_switch_only_changes_later_implicit_bindings() {
        let custom_agents = Arc::new(CustomAgentStore::new());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = Arc::new(MultiProviderBridge::new());

        let (first_id, first, _) = create_thread_for_agent_reference(
            store.clone(),
            bridge.clone(),
            custom_agents.clone(),
            ThreadEnsureOptions::default(),
            AgentBindingIntent::Fresh,
        )
        .await
        .expect("first implicit thread");
        assert_eq!(first["agent_id"], "claude");

        custom_agents
            .set_default_agent("codex")
            .await
            .expect("switch default");
        let (_, second, _) = create_thread_for_agent_reference(
            store.clone(),
            bridge,
            custom_agents,
            ThreadEnsureOptions::default(),
            AgentBindingIntent::Fresh,
        )
        .await
        .expect("second implicit thread");
        assert_eq!(second["agent_id"], "codex");
        assert_eq!(
            store.get(&first_id).await.unwrap().unwrap()["agent_id"],
            "claude"
        );
    }

    #[tokio::test]
    async fn canonical_binding_overwrites_reserved_identity_but_preserves_model_priority() {
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let custom_agents = custom_agent_store_with_default_workspace().await;
        custom_agents
            .set_enabled("codex", false)
            .await
            .expect("disable smuggled agent");
        let metadata = std::collections::HashMap::from([
            ("agent_id".to_owned(), Value::String("codex".to_owned())),
            (
                "requested_provider_type".to_owned(),
                Value::String("claude_code".to_owned()),
            ),
            (
                "provider_env".to_owned(),
                serde_json::json!({"SHOULD_NOT_SURVIVE": "1"}),
            ),
            (
                "model".to_owned(),
                Value::String("typed-thread-model".to_owned()),
            ),
        ]);
        let (_, data, _) = create_thread_for_agent_reference(
            store,
            Arc::new(MultiProviderBridge::new()),
            custom_agents,
            ThreadEnsureOptions {
                agent_id: Some("reviewer".to_owned()),
                metadata,
                ..ThreadEnsureOptions::default()
            },
            AgentBindingIntent::Fresh,
        )
        .await
        .expect("typed agent selection wins");

        let metadata = thread_metadata_from_value(&data);
        assert_eq!(metadata["agent_id"], "reviewer");
        assert_eq!(metadata["requested_provider_type"], "codex_app_server");
        assert_eq!(metadata["model"], "typed-thread-model");
        assert_eq!(
            metadata["provider_env"]["OPENAI_BASE_URL"],
            "http://127.0.0.1:15721/v1"
        );
        assert!(metadata["provider_env"].get("SHOULD_NOT_SURVIVE").is_none());
    }

    #[tokio::test]
    async fn all_disabled_rejects_only_fresh_implicit_binding() {
        let custom_agents = Arc::new(CustomAgentStore::new());
        for agent in custom_agents.list_agents().await {
            custom_agents
                .set_enabled(&agent.agent_id, false)
                .await
                .expect("disable agent");
        }
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let error = create_thread_for_agent_reference(
            store.clone(),
            Arc::new(MultiProviderBridge::new()),
            custom_agents,
            ThreadEnsureOptions::default(),
            AgentBindingIntent::Fresh,
        )
        .await
        .expect_err("implicit binding must fail with no enabled agents");
        assert_eq!(
            error.to_string(),
            "no enabled standalone agent is available"
        );
        assert!(store.list_keys(None).await.unwrap().is_empty());
    }
}
