use std::sync::Arc;

use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_models::{AgentReference, agent_runtime_snapshot_metadata, resolve_agent_reference};
use garyx_router::{
    ThreadCreator, ThreadEnsureOptions, ThreadStore, create_thread_record, workspace_dir_from_value,
};
use serde_json::Value;

use crate::custom_agents::CustomAgentStore;

pub(crate) const DEFAULT_AGENT_REFERENCE_ID: &str = "claude";

pub(crate) fn selected_agent_reference_id(
    requested: Option<&str>,
    current: Option<&str>,
) -> String {
    requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| current.map(str::trim).filter(|value| !value.is_empty()))
        .unwrap_or(DEFAULT_AGENT_REFERENCE_ID)
        .to_owned()
}

pub(crate) async fn resolve_agent_reference_from_stores(
    custom_agents: &CustomAgentStore,
    requested_id: &str,
) -> Result<AgentReference, String> {
    let agents = custom_agents.list_agents().await;
    resolve_agent_reference(requested_id, &agents)
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
        metadata.entry(key).or_insert(value);
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
        metadata.entry(key).or_insert(value);
    }
    Ok(())
}

fn set_thread_metadata_fields(value: &mut Value, entries: &[(&str, Value)]) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for (key, entry_value) in entries {
        object.insert((*key).to_owned(), entry_value.clone());
    }
}

pub(crate) async fn create_thread_for_agent_reference(
    thread_store: Arc<dyn ThreadStore>,
    bridge: Arc<MultiProviderBridge>,
    custom_agents: Arc<CustomAgentStore>,
    options: ThreadEnsureOptions,
) -> Result<(String, Value, AgentReference), String> {
    let requested_agent_id = selected_agent_reference_id(options.agent_id.as_deref(), None);
    let resolved =
        resolve_agent_reference_from_stores(custom_agents.as_ref(), &requested_agent_id).await?;

    let mut canonical_options = options;
    canonical_options.agent_id = Some(resolved.bound_agent_id().to_owned());
    canonical_options.provider_type = Some(resolved.provider_type());
    merge_agent_runtime_snapshot_metadata(&mut canonical_options.metadata, &resolved);
    if canonical_options
        .workspace_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        canonical_options.workspace_dir = default_workspace_dir_from_agent_reference(&resolved);
    }

    let (thread_id, mut data) = create_thread_record(&thread_store, canonical_options).await?;
    set_thread_metadata_fields(
        &mut data,
        &[(
            "agent_id",
            Value::String(resolved.bound_agent_id().to_owned()),
        )],
    );

    if let Err(error) = thread_store.set(&thread_id, data.clone()).await {
        tracing::warn!(thread_id = %thread_id, error = %error, "failed to persist agent runtime snapshot");
    }
    if let Some(workspace_dir) = workspace_dir_from_value(&data) {
        bridge
            .set_thread_workspace_binding(&thread_id, Some(workspace_dir))
            .await;
    }
    Ok((thread_id, data, resolved))
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
    ) -> Result<(String, Value), String> {
        let (thread_id, data, _) = create_thread_for_agent_reference(
            thread_store,
            self.bridge.clone(),
            self.custom_agents.clone(),
            options,
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
                model: Some("gpt-5".to_owned()),
                model_reasoning_effort: Some("high".to_owned()),
                model_service_tier: Some("priority".to_owned()),
                provider_env: Some(std::collections::HashMap::from([(
                    "OPENAI_BASE_URL".to_owned(),
                    "http://127.0.0.1:15721/v1".to_owned(),
                )])),
                auth_source: None,
                base_url: None,
                codex_home: None,
                max_tool_iterations: None,
                request_timeout_seconds: None,
                default_workspace_dir: Some("/tmp/agent-default".to_owned()),
                avatar_data_url: None,
                system_prompt: Some("Review carefully.".to_owned()),
            })
            .await
            .expect("custom agent");
        store
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
}
