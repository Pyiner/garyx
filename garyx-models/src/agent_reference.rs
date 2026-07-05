use std::collections::HashMap;

use serde_json::Value;

use crate::provider::ProviderType;
use crate::{AgentTeamProfile, CustomAgentProfile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentReference {
    Standalone {
        requested_id: String,
        profile: CustomAgentProfile,
    },
    /// A team reference. Leader is NOT privileged: the team as a whole is the
    /// addressable agent, and it is served by the meta-provider
    /// `ProviderType::AgentTeam`.
    Team {
        requested_id: String,
        team: AgentTeamProfile,
    },
}

impl AgentReference {
    pub fn requested_id(&self) -> &str {
        match self {
            Self::Standalone { requested_id, .. } | Self::Team { requested_id, .. } => requested_id,
        }
    }

    /// The canonical agent_id that this reference binds to on a thread.
    ///
    /// For a standalone agent it's the agent's own `agent_id`; for a team it's
    /// the `team_id` (teams occupy the unified agent_id namespace — the team
    /// itself is the agent, not its leader).
    pub fn bound_agent_id(&self) -> &str {
        match self {
            Self::Standalone { profile, .. } => &profile.agent_id,
            Self::Team { team, .. } => &team.team_id,
        }
    }

    pub fn provider_type(&self) -> ProviderType {
        match self {
            Self::Standalone { profile, .. } => profile.provider_type.clone(),
            Self::Team { .. } => ProviderType::AgentTeam,
        }
    }

    pub fn team(&self) -> Option<&AgentTeamProfile> {
        match self {
            Self::Standalone { .. } => None,
            Self::Team { team, .. } => Some(team),
        }
    }
}

/// Resolve a requested agent_id against the unified agent/team namespace.
///
/// Teams and standalone agents share one id space — a team_id is the agent_id
/// you talk to when you want the whole team. Leader aliasing is NOT supported:
/// to reach a team you must use its `team_id`; to reach the leader as a solo
/// agent you use the leader's `agent_id`.
pub fn resolve_agent_reference(
    requested_id: &str,
    agents: &[CustomAgentProfile],
    teams: &[AgentTeamProfile],
) -> Result<AgentReference, String> {
    let normalized = requested_id.trim();
    if normalized.is_empty() {
        return Err("agent_id is required".to_owned());
    }

    if let Some(team) = teams.iter().find(|team| team.team_id == normalized) {
        // Leader must exist — reported first so a misconfigured leader isn't
        // masked by a follow-on "unknown member" error (the leader typically
        // also appears in `member_agent_ids`).
        if !agents
            .iter()
            .any(|agent| agent.agent_id == team.leader_agent_id)
        {
            return Err(format!(
                "team '{}' references unknown leader_agent_id '{}'",
                team.team_id, team.leader_agent_id
            ));
        }
        // All remaining members must exist so callers can't construct a
        // reference pointing at a half-built team.
        for member_id in &team.member_agent_ids {
            if *member_id == team.leader_agent_id {
                continue;
            }
            if !agents.iter().any(|agent| agent.agent_id == *member_id) {
                return Err(format!(
                    "team '{}' references unknown member agent_id '{}'",
                    team.team_id, member_id
                ));
            }
        }
        return Ok(AgentReference::Team {
            requested_id: normalized.to_owned(),
            team: team.clone(),
        });
    }

    let profile = agents
        .iter()
        .find(|agent| agent.agent_id == normalized)
        .cloned()
        .ok_or_else(|| format!("unknown agent_id: {normalized}"))?;
    if !profile.standalone {
        return Err(format!("agent_id is not standalone: {normalized}"));
    }

    Ok(AgentReference::Standalone {
        requested_id: normalized.to_owned(),
        profile,
    })
}

/// Run metadata that carries an agent reference's execution configuration to
/// the shared providers: identity, provider routing, model override,
/// reasoning effort, service tier, and system prompt.
///
/// Every dispatch path must apply this for the thread's bound agent (the
/// bridge backfills it at run resolution); entry points that resolve an
/// explicit agent override apply it at the edge and their values win.
pub fn agent_runtime_metadata(reference: &AgentReference) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    metadata.insert(
        "agent_id".to_owned(),
        Value::String(reference.bound_agent_id().to_owned()),
    );
    metadata.insert(
        "requested_provider_type".to_owned(),
        Value::String(reference.provider_type().as_slug().to_owned()),
    );
    match reference {
        AgentReference::Standalone { profile, .. } => {
            metadata.insert(
                "agent_display_name".to_owned(),
                Value::String(profile.display_name.clone()),
            );
            if !profile.model.trim().is_empty() {
                metadata.insert("model".to_owned(), Value::String(profile.model.clone()));
            }
            if !profile.model_reasoning_effort.trim().is_empty() {
                metadata.insert(
                    "model_reasoning_effort".to_owned(),
                    Value::String(profile.model_reasoning_effort.clone()),
                );
            }
            if !profile.model_service_tier.trim().is_empty() {
                metadata.insert(
                    "model_service_tier".to_owned(),
                    Value::String(profile.model_service_tier.clone()),
                );
            }
            if !profile.system_prompt.trim().is_empty() {
                metadata.insert(
                    "system_prompt".to_owned(),
                    Value::String(profile.system_prompt.clone()),
                );
            }
        }
        AgentReference::Team { team, .. } => {
            metadata.insert(
                "agent_team_id".to_owned(),
                Value::String(team.team_id.clone()),
            );
            metadata.insert(
                "agent_display_name".to_owned(),
                Value::String(team.display_name.clone()),
            );
        }
    }
    metadata
}

pub fn agent_provider_env_metadata(reference: &AgentReference) -> HashMap<String, Value> {
    let AgentReference::Standalone { profile, .. } = reference else {
        return HashMap::new();
    };
    if profile.provider_env.is_empty() {
        return HashMap::new();
    }
    let env = profile
        .provider_env
        .iter()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect();
    HashMap::from([("provider_env".to_owned(), Value::Object(env))])
}

pub fn agent_runtime_snapshot_metadata(reference: &AgentReference) -> HashMap<String, Value> {
    let mut metadata = agent_runtime_metadata(reference);
    metadata.extend(agent_provider_env_metadata(reference));
    metadata
}

const THREAD_AGENT_RUNTIME_SNAPSHOT_KEYS: &[&str] = &[
    "agent_id",
    "agent_display_name",
    "agent_team_id",
    "requested_provider_type",
    "model",
    "model_reasoning_effort",
    "model_service_tier",
    "system_prompt",
    "provider_env",
];

pub fn merge_thread_agent_runtime_snapshot(
    thread_data: &Value,
    run_metadata: &mut HashMap<String, Value>,
) {
    let Some(thread_metadata) = thread_data.get("metadata").and_then(Value::as_object) else {
        return;
    };
    for key in THREAD_AGENT_RUNTIME_SNAPSHOT_KEYS {
        if let Some(value) = thread_metadata.get(*key) {
            run_metadata
                .entry((*key).to_owned())
                .or_insert_with(|| value.clone());
        }
    }
}

/// Verify that team_ids do not collide with agent_ids in the unified namespace.
///
/// Must be invoked at boot (see `garyx-gateway::composition::app_bootstrap`).
/// A collision is a configuration error: the same id cannot mean both "a team"
/// and "a standalone agent".
pub fn validate_agent_team_registry_uniqueness(
    agents: &[CustomAgentProfile],
    teams: &[AgentTeamProfile],
) -> Result<(), String> {
    // Collisions between a team_id and any agent_id.
    for team in teams {
        if agents.iter().any(|agent| agent.agent_id == team.team_id) {
            return Err(format!(
                "agent_team registry conflict: team_id '{}' collides with an existing agent_id",
                team.team_id
            ));
        }
    }

    // Duplicates within teams themselves.
    for (i, team) in teams.iter().enumerate() {
        if teams[..i].iter().any(|other| other.team_id == team.team_id) {
            return Err(format!(
                "agent_team registry conflict: duplicate team_id '{}'",
                team.team_id
            ));
        }
    }

    // Duplicates within agents themselves (belt-and-suspenders; usually enforced
    // upstream, but we want boot to shout clearly if it ever slips through).
    for (i, agent) in agents.iter().enumerate() {
        if agents[..i]
            .iter()
            .any(|other| other.agent_id == agent.agent_id)
        {
            return Err(format!(
                "agent_team registry conflict: duplicate agent_id '{}'",
                agent.agent_id
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
