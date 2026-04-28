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
