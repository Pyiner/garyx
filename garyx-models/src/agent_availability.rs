use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    AgentReference, CustomAgentProfile, ProviderType, agent_runtime_snapshot_metadata,
    resolve_agent_reference,
};

/// Server-owned metadata keys that callers may not use to choose or alter an
/// agent binding. Model, reasoning, tier, and system-prompt keys intentionally
/// remain outside this set so their existing request > thread > agent priority
/// is preserved.
pub const SERVER_OWNED_AGENT_METADATA_KEYS: &[&str] =
    &["agent_id", "requested_provider_type", "provider_env"];

pub fn strip_server_owned_agent_metadata(metadata: &mut HashMap<String, Value>) {
    for key in SERVER_OWNED_AGENT_METADATA_KEYS {
        metadata.remove(*key);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAvailabilitySnapshot {
    pub agents: Vec<CustomAgentProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
    pub agent_state_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgentBinding {
    pub agent_id: String,
    pub provider_type: ProviderType,
    pub runtime_metadata: HashMap<String, Value>,
    pub default_workspace_dir: Option<String>,
}

impl ResolvedAgentBinding {
    pub fn from_reference(reference: &AgentReference) -> Self {
        let AgentReference::Standalone { profile, .. } = reference;
        Self {
            agent_id: profile.agent_id.clone(),
            provider_type: profile.provider_type.clone(),
            runtime_metadata: agent_runtime_snapshot_metadata(reference),
            default_workspace_dir: profile
                .default_workspace_dir
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentBindingError {
    #[error("agent is disabled: {0}")]
    AgentDisabled(String),
    #[error("no enabled standalone agent is available")]
    NoEnabledAgent,
    #[error("unknown agent_id: {0}")]
    UnknownAgent(String),
    #[error("agent_id is not standalone: {0}")]
    NotStandalone(String),
    #[error("agent_id is required")]
    AgentIdRequired,
}

fn normalized(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn profile<'a>(
    snapshot: &'a AgentAvailabilitySnapshot,
    agent_id: &str,
) -> Result<&'a CustomAgentProfile, AgentBindingError> {
    let Some(agent_id) = normalized(agent_id) else {
        return Err(AgentBindingError::AgentIdRequired);
    };
    snapshot
        .agents
        .iter()
        .find(|agent| agent.agent_id == agent_id)
        .ok_or_else(|| AgentBindingError::UnknownAgent(agent_id.to_owned()))
}

/// Validate an explicit new binding. Disabled explicit choices are rejected
/// and never fall through to another agent.
pub fn ensure_enabled_for_new_binding(
    snapshot: &AgentAvailabilitySnapshot,
    agent_id: &str,
) -> Result<ResolvedAgentBinding, AgentBindingError> {
    let profile = profile(snapshot, agent_id)?;
    if !profile.standalone {
        return Err(AgentBindingError::NotStandalone(profile.agent_id.clone()));
    }
    if !profile.enabled {
        return Err(AgentBindingError::AgentDisabled(profile.agent_id.clone()));
    }
    let reference = resolve_agent_reference(&profile.agent_id, &snapshot.agents)
        .map_err(|_| AgentBindingError::UnknownAgent(profile.agent_id.clone()))?;
    Ok(ResolvedAgentBinding::from_reference(&reference))
}

/// Resolve the global effective default with deterministic ordering: raw
/// enabled default, enabled Claude, built-ins in seed order, then custom ids.
pub fn resolve_effective_default(
    snapshot: &AgentAvailabilitySnapshot,
) -> Option<ResolvedAgentBinding> {
    if let Some(raw) = snapshot.default_agent_id.as_deref()
        && let Ok(binding) = ensure_enabled_for_new_binding(snapshot, raw)
    {
        return Some(binding);
    }
    if let Ok(binding) = ensure_enabled_for_new_binding(snapshot, "claude") {
        return Some(binding);
    }

    const BUILTIN_ORDER: &[&str] = &["claude", "codex", "traex", "antigravity", "grok"];
    for agent_id in BUILTIN_ORDER {
        if let Ok(binding) = ensure_enabled_for_new_binding(snapshot, agent_id) {
            return Some(binding);
        }
    }

    let mut custom_ids = snapshot
        .agents
        .iter()
        .filter(|agent| !agent.built_in && agent.enabled && agent.standalone)
        .map(|agent| agent.agent_id.as_str())
        .collect::<Vec<_>>();
    custom_ids.sort_unstable();
    custom_ids
        .into_iter()
        .find_map(|agent_id| ensure_enabled_for_new_binding(snapshot, agent_id).ok())
}

/// Shared requested -> current -> global-default chain. `current` denotes an
/// already-existing binding, so enabled is deliberately not consulted there.
pub fn resolve_agent_binding(
    snapshot: &AgentAvailabilitySnapshot,
    requested: Option<&str>,
    current: Option<&str>,
) -> Result<ResolvedAgentBinding, AgentBindingError> {
    if let Some(requested) = requested.and_then(normalized) {
        return ensure_enabled_for_new_binding(snapshot, requested);
    }
    if let Some(current) = current.and_then(normalized) {
        let profile = profile(snapshot, current)?;
        if !profile.standalone {
            return Err(AgentBindingError::NotStandalone(profile.agent_id.clone()));
        }
        let reference = resolve_agent_reference(current, &snapshot.agents)
            .map_err(|_| AgentBindingError::UnknownAgent(current.to_owned()))?;
        return Ok(ResolvedAgentBinding::from_reference(&reference));
    }
    resolve_effective_default(snapshot).ok_or(AgentBindingError::NoEnabledAgent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_provider_agent_profiles;

    fn snapshot() -> AgentAvailabilitySnapshot {
        AgentAvailabilitySnapshot {
            agents: builtin_provider_agent_profiles(),
            default_agent_id: None,
            agent_state_revision: 1,
        }
    }

    #[test]
    fn explicit_disabled_never_falls_back() {
        let mut snapshot = snapshot();
        snapshot
            .agents
            .iter_mut()
            .find(|agent| agent.agent_id == "codex")
            .unwrap()
            .enabled = false;
        assert_eq!(
            resolve_agent_binding(&snapshot, Some("codex"), None),
            Err(AgentBindingError::AgentDisabled("codex".to_owned()))
        );
    }

    #[test]
    fn current_disabled_binding_is_preserved() {
        let mut snapshot = snapshot();
        snapshot
            .agents
            .iter_mut()
            .find(|agent| agent.agent_id == "codex")
            .unwrap()
            .enabled = false;
        assert_eq!(
            resolve_agent_binding(&snapshot, None, Some("codex"))
                .unwrap()
                .agent_id,
            "codex"
        );
    }

    #[test]
    fn raw_default_then_claude_then_seeded_builtin_then_sorted_custom() {
        let mut snapshot = snapshot();
        snapshot.default_agent_id = Some("codex".to_owned());
        assert_eq!(
            resolve_effective_default(&snapshot).unwrap().agent_id,
            "codex"
        );

        snapshot
            .agents
            .iter_mut()
            .find(|agent| agent.agent_id == "codex")
            .unwrap()
            .enabled = false;
        assert_eq!(
            resolve_effective_default(&snapshot).unwrap().agent_id,
            "claude"
        );

        snapshot.default_agent_id = None;
        snapshot
            .agents
            .iter_mut()
            .find(|agent| agent.agent_id == "claude")
            .unwrap()
            .enabled = false;
        assert_eq!(
            resolve_effective_default(&snapshot).unwrap().agent_id,
            "traex",
            "built-in fallback follows seed order after disabled claude/codex"
        );

        for agent in &mut snapshot.agents {
            agent.enabled = false;
        }
        let mut zed = builtin_provider_agent_profiles().remove(1);
        zed.agent_id = "zed".to_owned();
        zed.built_in = false;
        zed.enabled = true;
        let mut alpha = zed.clone();
        alpha.agent_id = "alpha".to_owned();
        snapshot.agents.extend([zed, alpha]);
        assert_eq!(
            resolve_effective_default(&snapshot).unwrap().agent_id,
            "alpha"
        );
    }

    #[test]
    fn unknown_and_non_standalone_choices_are_typed_and_never_selected_as_default() {
        let mut snapshot = snapshot();
        assert_eq!(
            resolve_agent_binding(&snapshot, Some("missing"), None),
            Err(AgentBindingError::UnknownAgent("missing".to_owned()))
        );

        let mut embedded = snapshot.agents[0].clone();
        embedded.agent_id = "embedded".to_owned();
        embedded.built_in = false;
        embedded.standalone = false;
        snapshot.agents.push(embedded);
        assert_eq!(
            resolve_agent_binding(&snapshot, Some("embedded"), None),
            Err(AgentBindingError::NotStandalone("embedded".to_owned()))
        );
        snapshot.default_agent_id = Some("embedded".to_owned());
        assert_eq!(
            resolve_effective_default(&snapshot).unwrap().agent_id,
            "claude",
            "an invalid raw default falls through without becoming a binding"
        );
    }

    #[test]
    fn no_enabled_agent_is_typed() {
        let mut snapshot = snapshot();
        for agent in &mut snapshot.agents {
            agent.enabled = false;
        }
        assert_eq!(
            resolve_agent_binding(&snapshot, None, None),
            Err(AgentBindingError::NoEnabledAgent)
        );
    }

    #[test]
    fn reserved_metadata_keys_are_exact() {
        let mut metadata = HashMap::from([
            ("agent_id".to_owned(), Value::String("other".to_owned())),
            (
                "requested_provider_type".to_owned(),
                Value::String("codex_app_server".to_owned()),
            ),
            ("provider_env".to_owned(), Value::Object(Default::default())),
            (
                "model".to_owned(),
                Value::String("request-model".to_owned()),
            ),
            (
                "system_prompt".to_owned(),
                Value::String("request".to_owned()),
            ),
        ]);
        strip_server_owned_agent_metadata(&mut metadata);
        assert_eq!(metadata.len(), 2);
        assert_eq!(metadata["model"], "request-model");
        assert_eq!(metadata["system_prompt"], "request");
    }
}
