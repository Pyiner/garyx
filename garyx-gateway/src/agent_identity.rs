use std::sync::Arc;

use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_models::{AgentReference, resolve_agent_reference};
use garyx_router::{
    ThreadCreator, ThreadEnsureOptions, ThreadStore, create_thread_record, workspace_dir_from_value,
};
use serde_json::Value;

use crate::agent_teams::AgentTeamStore;
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
    agent_teams: &AgentTeamStore,
    requested_id: &str,
) -> Result<AgentReference, String> {
    let agents = custom_agents.list_agents().await;
    let teams = agent_teams.list_teams().await;
    resolve_agent_reference(requested_id, &agents, &teams)
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
        AgentReference::Team { .. } => None,
    }
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
    agent_teams: Arc<AgentTeamStore>,
    options: ThreadEnsureOptions,
) -> Result<(String, Value, AgentReference), String> {
    let requested_agent_id = selected_agent_reference_id(options.agent_id.as_deref(), None);
    let resolved = resolve_agent_reference_from_stores(
        custom_agents.as_ref(),
        agent_teams.as_ref(),
        &requested_agent_id,
    )
    .await?;

    let mut canonical_options = options;
    canonical_options.agent_id = Some(resolved.bound_agent_id().to_owned());
    canonical_options.provider_type = Some(resolved.provider_type());
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

    thread_store.set(&thread_id, data.clone()).await;
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
    agent_teams: Arc<AgentTeamStore>,
}

impl GatewayThreadCreator {
    pub(crate) fn new(
        bridge: Arc<MultiProviderBridge>,
        custom_agents: Arc<CustomAgentStore>,
        agent_teams: Arc<AgentTeamStore>,
    ) -> Self {
        Self {
            bridge,
            custom_agents,
            agent_teams,
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
            self.agent_teams.clone(),
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
    use garyx_router::{InMemoryThreadStore, ThreadEnsureOptions, ThreadStore};

    async fn custom_agent_store_with_default_workspace() -> Arc<CustomAgentStore> {
        let store = Arc::new(CustomAgentStore::new());
        store
            .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
                agent_id: "reviewer".to_owned(),
                display_name: "Reviewer".to_owned(),
                provider_type: ProviderType::CodexAppServer,
                model: "gpt-5".to_owned(),
                default_workspace_dir: Some("/tmp/agent-default".to_owned()),
                system_prompt: "Review carefully.".to_owned(),
            })
            .await
            .expect("custom agent");
        store
    }

    #[tokio::test]
    async fn create_thread_uses_agent_default_workspace_when_unset() {
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let custom_agents = custom_agent_store_with_default_workspace().await;
        let agent_teams = Arc::new(AgentTeamStore::new());
        let (thread_id, data, _) = create_thread_for_agent_reference(
            thread_store,
            Arc::new(MultiProviderBridge::new()),
            custom_agents,
            agent_teams,
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
        let agent_teams = Arc::new(AgentTeamStore::new());
        let (_, data, _) = create_thread_for_agent_reference(
            thread_store,
            Arc::new(MultiProviderBridge::new()),
            custom_agents,
            agent_teams,
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
}
