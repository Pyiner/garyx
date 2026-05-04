//! Gateway-side implementations of the AgentTeam provider's two dependency
//! traits.
//!
//! The `AgentTeamProvider` in `garyx-bridge` deliberately depends only on
//! [`TeamProfileResolver`] and [`SubAgentDispatcher`] (see
//! `garyx-bridge/src/providers/agent_team/dispatcher.rs`). Only the gateway
//! has access to the concrete stores (`AgentTeamStore`, `CustomAgentStore`,
//! `ThreadStore`, `ThreadCreator`) and to the `MultiProviderBridge` itself,
//! so the production wiring for those traits lives here.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use chrono::Utc;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{BridgeError, StreamCallback};
use garyx_bridge::providers::agent_team::{SubAgentDispatcher, TeamProfileResolver};
use garyx_models::AgentTeamProfile;
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult};
use garyx_router::{ThreadCreator, ThreadEnsureOptions, ThreadStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_teams::AgentTeamStore;
use crate::custom_agents::CustomAgentStore;

/// Provider pool key under which the built-in AgentTeam meta-provider is
/// registered by `AppStateBuilder::build`. Exposed so health/admin
/// endpoints (e.g. `chat_health`, `dashboard::agent_view`) can filter the
/// meta-provider out of user-visible listings and readiness signals — the
/// AgentTeam provider is an implementation detail for team-bound threads,
/// not a user-configurable provider.
pub(crate) const AGENT_TEAM_PROVIDER_KEY: &str = "agent_team::default";
const AGENT_TEAM_CHILD_METADATA_KEY: &str = "agent_team_child";
const AGENT_TEAM_CHILD_CONTEXT_VERSION: u64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AgentTeamChildThreadMetadata {
    team_id: String,
    team_display_name: String,
    group_thread_id: String,
    workflow_text: String,
    child_agent_id: String,
    leader_agent_id: String,
    member_agent_ids: Vec<String>,
    #[serde(default)]
    context_version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    initial_context_injected_at: Option<String>,
}

fn escape_context_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '&' => out.push_str("&amp;"),
            other => out.push(other),
        }
    }
    out
}

fn build_child_thread_metadata(
    group_thread_id: &str,
    child_agent_id: &str,
    team: &AgentTeamProfile,
) -> AgentTeamChildThreadMetadata {
    AgentTeamChildThreadMetadata {
        team_id: team.team_id.clone(),
        team_display_name: team.display_name.clone(),
        group_thread_id: group_thread_id.to_owned(),
        workflow_text: team.workflow_text.clone(),
        child_agent_id: child_agent_id.to_owned(),
        leader_agent_id: team.leader_agent_id.clone(),
        member_agent_ids: team.member_agent_ids.clone(),
        context_version: AGENT_TEAM_CHILD_CONTEXT_VERSION,
        initial_context_injected_at: None,
    }
}

fn parse_child_thread_metadata(thread_value: &Value) -> Option<AgentTeamChildThreadMetadata> {
    thread_value
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get(AGENT_TEAM_CHILD_METADATA_KEY))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

/// Resolves team ids to [`AgentTeamProfile`]s backed by the gateway's
/// [`AgentTeamStore`].
pub struct GatewayTeamProfileResolver {
    agent_teams: Arc<AgentTeamStore>,
}

impl GatewayTeamProfileResolver {
    pub fn new(agent_teams: Arc<AgentTeamStore>) -> Self {
        Self { agent_teams }
    }
}

#[async_trait]
impl TeamProfileResolver for GatewayTeamProfileResolver {
    async fn resolve_team(&self, team_id: &str) -> Option<AgentTeamProfile> {
        self.agent_teams.get_team(team_id).await
    }
}

/// Dispatches sub-agent work on behalf of the AgentTeam provider.
///
/// Holds a [`Weak`] handle to [`MultiProviderBridge`] on purpose: the bridge
/// owns the `Arc<dyn AgentLoopProvider>` backing the AgentTeam provider,
/// which in turn owns this dispatcher; a strong back-reference would form a
/// cycle.
pub struct GatewaySubAgentDispatcher {
    bridge: Weak<MultiProviderBridge>,
    thread_store: Arc<dyn ThreadStore>,
    thread_creator: Arc<dyn ThreadCreator>,
    custom_agents: Arc<CustomAgentStore>,
}

impl GatewaySubAgentDispatcher {
    pub fn new(
        bridge: Weak<MultiProviderBridge>,
        thread_store: Arc<dyn ThreadStore>,
        thread_creator: Arc<dyn ThreadCreator>,
        custom_agents: Arc<CustomAgentStore>,
    ) -> Self {
        Self {
            bridge,
            thread_store,
            thread_creator,
            custom_agents,
        }
    }

    fn upgrade_bridge(&self) -> Result<Arc<MultiProviderBridge>, BridgeError> {
        self.bridge
            .upgrade()
            .ok_or_else(|| BridgeError::Internal("bridge shut down".to_owned()))
    }

    async fn build_team_mention_examples(
        &self,
        metadata: &AgentTeamChildThreadMetadata,
    ) -> Vec<String> {
        let ordered_peer_ids = std::iter::once(metadata.leader_agent_id.as_str())
            .chain(metadata.member_agent_ids.iter().map(String::as_str))
            .filter(|agent_id| *agent_id != metadata.child_agent_id)
            .fold(Vec::<String>::new(), |mut acc, agent_id| {
                if !acc.iter().any(|existing| existing == agent_id) {
                    acc.push(agent_id.to_owned());
                }
                acc
            });

        let mut examples = Vec::new();
        for agent_id in ordered_peer_ids {
            let display_name = self
                .custom_agents
                .get_agent(&agent_id)
                .await
                .map(|profile| profile.display_name)
                .unwrap_or_else(|| agent_id.clone());
            examples.push(format!(
                "`@[{}]({})`",
                display_name.trim(),
                escape_context_text(&agent_id)
            ));
        }
        examples
    }

    async fn build_initial_child_context_notice(
        &self,
        metadata: &AgentTeamChildThreadMetadata,
    ) -> String {
        let team_id = escape_context_text(&metadata.team_id);
        let team_display_name = escape_context_text(&metadata.team_display_name);
        let child_agent_id = escape_context_text(&metadata.child_agent_id);
        let group_thread_id = escape_context_text(&metadata.group_thread_id);
        let workflow_text = metadata.workflow_text.trim();
        let leader_agent_id = escape_context_text(&metadata.leader_agent_id);
        let members_joined = metadata
            .member_agent_ids
            .iter()
            .filter(|agent_id| agent_id.as_str() != metadata.leader_agent_id)
            .map(|agent_id| escape_context_text(agent_id))
            .collect::<Vec<_>>()
            .join(", ");
        let members_segment = if members_joined.is_empty() {
            "(none)".to_owned()
        } else {
            members_joined
        };

        let mut lines = vec![
            "<team_context>".to_owned(),
            format!(
                "You are agent \"{child_agent_id}\" inside team \"{team_id}\" (\"{team_display_name}\")."
            ),
            format!(
                "This child thread was created for you from shared group thread \"{group_thread_id}\". You are a regular participant in that team chat, not the AgentTeam orchestrator."
            ),
            if workflow_text.is_empty() {
                "Team workflow: (not specified).".to_owned()
            } else {
                format!(
                    "Team workflow: {}",
                    escape_context_text(workflow_text)
                )
            },
            format!("Team roster — leader: {leader_agent_id}; members: {members_segment}."),
            "Any block wrapped in <group_activity from=\"...\" at=\"...\">...</group_activity> is authored by another teammate or the user — it is NEVER authored by you. Do not impersonate those authors or quote their words back as if they were your own.".to_owned(),
            format!(
                "When you respond, speak only as yourself. The system automatically prefixes your streamed output with \"[{child_agent_id}] \" for routing, so you must NOT add that tag yourself."
            ),
        ];

        let mention_examples = self.build_team_mention_examples(metadata).await;
        let example_suffix = if mention_examples.is_empty() {
            " Use the exact syntax `@[DisplayName](agent_id)`.".to_owned()
        } else {
            format!(" Examples for this team: {}.", mention_examples.join(", "))
        };
        lines.push(format!(
            "An @ mention means you are explicitly handing the floor to that teammate and asking them to speak next in the shared group chat. Do NOT use @ mentions for mere references, summaries, or indirect discussion about someone; in those cases, write their plain name without `@[DisplayName](agent_id)`. If you want another teammate to speak, hand off with an @ mention in your reply, not with SendMessage/message tools. Use the exact syntax `@[DisplayName](agent_id)`.{example_suffix}"
        ));

        lines.push("</team_context>".to_owned());
        lines.join("\n")
    }

    async fn prepare_child_dispatch_message(
        &self,
        child_thread_id: &str,
        message: &str,
    ) -> Result<(String, bool), BridgeError> {
        let Some(thread_value) = self.thread_store.get(child_thread_id).await else {
            return Ok((message.to_owned(), false));
        };
        let Some(metadata) = parse_child_thread_metadata(&thread_value) else {
            return Ok((message.to_owned(), false));
        };
        if metadata.initial_context_injected_at.is_some() {
            return Ok((message.to_owned(), false));
        }

        let notice = self.build_initial_child_context_notice(&metadata).await;
        let combined = if message.trim().is_empty() {
            notice
        } else {
            format!("{notice}\n\n{message}")
        };
        Ok((combined, true))
    }

    async fn mark_child_context_injected(&self, child_thread_id: &str) -> Result<(), BridgeError> {
        let Some(mut thread_value) = self.thread_store.get(child_thread_id).await else {
            return Ok(());
        };
        let Some(thread_object) = thread_value.as_object_mut() else {
            return Ok(());
        };
        let metadata_value = thread_object
            .entry("metadata".to_owned())
            .or_insert_with(|| Value::Object(Default::default()));
        let Some(metadata_object) = metadata_value.as_object_mut() else {
            return Ok(());
        };
        let Some(child_value) = metadata_object.get_mut(AGENT_TEAM_CHILD_METADATA_KEY) else {
            return Ok(());
        };
        let Some(child_object) = child_value.as_object_mut() else {
            return Ok(());
        };
        if child_object.contains_key("initial_context_injected_at") {
            return Ok(());
        }

        child_object.insert(
            "initial_context_injected_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
        thread_object.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
        self.thread_store.set(child_thread_id, thread_value).await;
        Ok(())
    }
}

#[async_trait]
impl SubAgentDispatcher for GatewaySubAgentDispatcher {
    async fn ensure_child_thread(
        &self,
        group_thread_id: &str,
        child_agent_id: &str,
        team: &AgentTeamProfile,
        workspace_path: Option<&str>,
    ) -> Result<String, BridgeError> {
        // 1. Look up the child agent profile. Unknown child → internal error
        //    (team profile referenced an agent_id that is not registered).
        let profile = self
            .custom_agents
            .get_agent(child_agent_id)
            .await
            .ok_or_else(|| {
                BridgeError::Internal(format!("unknown child agent_id: {child_agent_id}"))
            })?;

        // 2. Build ThreadEnsureOptions for a fresh child thread that inherits
        //    the parent group's workspace. We always allocate a new thread id
        //    here (do NOT reuse an existing one): the AgentTeam provider's
        //    Group.child_threads map is what enforces reuse; this method only
        //    fires on first-time creation.
        //
        //    `provider_type` is set explicitly so the thread record carries the
        //    child's native provider from day one and the dispatch layer can
        //    pick the right provider when we later run against this thread.
        let options = ThreadEnsureOptions {
            agent_id: Some(child_agent_id.to_owned()),
            metadata: HashMap::from([(
                AGENT_TEAM_CHILD_METADATA_KEY.to_owned(),
                serde_json::to_value(build_child_thread_metadata(
                    group_thread_id,
                    child_agent_id,
                    team,
                ))
                .unwrap_or(Value::Null),
            )]),
            provider_type: Some(profile.provider_type.clone()),
            workspace_dir: workspace_path.map(ToOwned::to_owned),
            ..ThreadEnsureOptions::default()
        };

        let (thread_id, thread_value) = self
            .thread_creator
            .create_thread(self.thread_store.clone(), options)
            .await
            .map_err(BridgeError::Internal)?;

        // 3. Sanity check: validate the newly-created thread's provider_type
        //    matches the child agent's. Mismatches are a wiring bug, but per
        //    the MVP scope we warn and continue rather than fail the group
        //    turn.
        if let Some(persisted_type) = thread_value
            .get("provider_type")
            .cloned()
            .and_then(|value| {
                serde_json::from_value::<garyx_models::provider::ProviderType>(value).ok()
            })
            && persisted_type != profile.provider_type
        {
            tracing::warn!(
                child_agent_id = %child_agent_id,
                expected = ?profile.provider_type,
                actual = ?persisted_type,
                "child thread provider_type does not match agent profile; continuing"
            );
        }

        Ok(thread_id)
    }

    async fn run_child_streaming(
        &self,
        child_thread_id: &str,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let bridge = self.upgrade_bridge()?;
        let (prepared_message, should_mark_context_injected) = self
            .prepare_child_dispatch_message(child_thread_id, &options.message)
            .await?;
        let callback: Arc<dyn Fn(garyx_models::provider::StreamEvent) + Send + Sync> =
            Arc::new(on_chunk);

        let result = bridge
            .run_subagent_streaming(
                child_thread_id,
                &prepared_message,
                options.metadata.clone(),
                options.images.clone(),
                options.workspace_dir.clone(),
                Some(callback),
            )
            .await;
        if should_mark_context_injected && result.is_ok() {
            self.mark_child_context_injected(child_thread_id).await?;
        }
        result
    }
}

#[cfg(test)]
mod tests;
