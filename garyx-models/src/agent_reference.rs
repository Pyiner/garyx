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
