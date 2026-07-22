use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{CustomAgentProfile, ProviderType, builtin_provider_agent_profiles};

pub const AGENT_STORE_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAgentStoreDocument {
    pub agents: Vec<CustomAgentProfile>,
    pub default_agent_id: Option<String>,
    pub migrated_from_legacy: bool,
    pub skipped_unsupported_agent_ids: Vec<String>,
}

#[derive(Debug, Error)]
pub enum AgentStoreDocumentError {
    #[error("invalid custom agent store: {0}")]
    Invalid(#[from] serde_json::Error),
    #[error("unsupported custom-agents.json version: {0}")]
    UnsupportedVersion(u32),
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedAgentEnvelope {
    version: u32,
    agents: BTreeMap<String, Value>,
    #[serde(default)]
    disabled_builtin_ids: Vec<String>,
    #[serde(default)]
    default_agent_id: Option<String>,
}

fn normalize_optional_id(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn sorted_profiles(agents: HashMap<String, CustomAgentProfile>) -> Vec<CustomAgentProfile> {
    const BUILTIN_ORDER: &[&str] = &["claude", "codex", "traex", "antigravity", "grok"];
    let builtin_rank = |agent_id: &str| {
        BUILTIN_ORDER
            .iter()
            .position(|candidate| *candidate == agent_id)
            .unwrap_or(usize::MAX)
    };
    let mut profiles = agents.into_values().collect::<Vec<_>>();
    profiles.sort_by(|left, right| match (left.built_in, right.built_in) {
        (true, true) => builtin_rank(&left.agent_id).cmp(&builtin_rank(&right.agent_id)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => left.agent_id.cmp(&right.agent_id),
    });
    profiles
}

/// Parse either the legacy bare custom-agent map or the version-2 envelope.
/// The returned list is the complete catalog: built-ins plus persisted custom
/// profiles, with disabled built-ins applied.
pub fn parse_agent_store_document(
    content: &str,
) -> Result<ParsedAgentStoreDocument, AgentStoreDocumentError> {
    let root: Value = serde_json::from_str(content)?;
    let is_envelope = root.get("version").and_then(Value::as_u64).is_some()
        && root.get("agents").is_some_and(Value::is_object);
    let (persisted, disabled_builtin_ids, default_agent_id, migrated_from_legacy) = if is_envelope {
        let envelope: PersistedAgentEnvelope = serde_json::from_value(root)?;
        if envelope.version != AGENT_STORE_VERSION {
            return Err(AgentStoreDocumentError::UnsupportedVersion(
                envelope.version,
            ));
        }
        (
            envelope.agents,
            envelope.disabled_builtin_ids,
            normalize_optional_id(envelope.default_agent_id),
            false,
        )
    } else {
        (
            serde_json::from_value::<BTreeMap<String, Value>>(root)?,
            Vec::new(),
            None,
            true,
        )
    };

    let disabled_builtin_ids = disabled_builtin_ids.into_iter().collect::<HashSet<_>>();
    let mut agents = builtin_provider_agent_profiles()
        .into_iter()
        .map(|mut profile| {
            profile.enabled = !disabled_builtin_ids.contains(&profile.agent_id);
            (profile.agent_id.clone(), profile)
        })
        .collect::<HashMap<_, _>>();
    let mut skipped_unsupported_agent_ids = Vec::new();
    for (agent_id, value) in persisted {
        if value
            .get("provider_type")
            .and_then(Value::as_str)
            .is_some_and(|provider_type| ProviderType::from_slug(provider_type).is_none())
        {
            skipped_unsupported_agent_ids.push(agent_id);
            continue;
        }
        let mut profile = serde_json::from_value::<CustomAgentProfile>(value)?;
        profile.agent_id = agent_id.clone();
        profile.built_in = false;
        agents.insert(agent_id, profile);
    }

    Ok(ParsedAgentStoreDocument {
        agents: sorted_profiles(agents),
        default_agent_id,
        migrated_from_legacy,
        skipped_unsupported_agent_ids,
    })
}

/// Serialize the complete catalog as the canonical version-2 envelope.
pub fn serialize_agent_store_document(
    agents: &[CustomAgentProfile],
    default_agent_id: Option<&str>,
) -> Result<String, AgentStoreDocumentError> {
    let persisted = agents
        .iter()
        .filter(|profile| !profile.built_in)
        .map(|profile| {
            serde_json::to_value(profile)
                .map(|value| (profile.agent_id.clone(), value))
                .map_err(AgentStoreDocumentError::Invalid)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut disabled_builtin_ids = agents
        .iter()
        .filter(|profile| profile.built_in && !profile.enabled)
        .map(|profile| profile.agent_id.clone())
        .collect::<Vec<_>>();
    disabled_builtin_ids.sort_unstable();
    let envelope = PersistedAgentEnvelope {
        version: AGENT_STORE_VERSION,
        agents: persisted,
        disabled_builtin_ids,
        default_agent_id: default_agent_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    };
    serde_json::to_string_pretty(&envelope).map_err(AgentStoreDocumentError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_round_trip_preserves_disabled_builtins_default_and_custom_profiles() {
        let mut agents = builtin_provider_agent_profiles();
        agents
            .iter_mut()
            .find(|agent| agent.agent_id == "claude")
            .unwrap()
            .enabled = false;
        let mut custom = agents[1].clone();
        custom.agent_id = "reviewer".to_owned();
        custom.display_name = "Reviewer".to_owned();
        custom.built_in = false;
        agents.push(custom);

        let encoded = serialize_agent_store_document(&agents, Some("reviewer")).unwrap();
        let wire: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(wire["version"], AGENT_STORE_VERSION);
        assert_eq!(wire["default_agent_id"], "reviewer");
        assert_eq!(wire["disabled_builtin_ids"], serde_json::json!(["claude"]));
        assert!(wire["agents"].get("reviewer").is_some());
        assert!(wire["agents"].get("codex").is_none());

        let parsed = parse_agent_store_document(&encoded).unwrap();
        assert!(!parsed.migrated_from_legacy);
        assert_eq!(parsed.default_agent_id.as_deref(), Some("reviewer"));
        assert!(
            !parsed
                .agents
                .iter()
                .find(|agent| agent.agent_id == "claude")
                .unwrap()
                .enabled
        );
        assert!(
            parsed
                .agents
                .iter()
                .find(|agent| agent.agent_id == "reviewer")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn legacy_map_is_detected_and_missing_enabled_defaults_true() {
        let mut custom = builtin_provider_agent_profiles().remove(1);
        custom.agent_id = "legacy".to_owned();
        custom.display_name = "Legacy".to_owned();
        custom.built_in = false;
        let mut value = serde_json::to_value(custom).unwrap();
        value.as_object_mut().unwrap().remove("enabled");
        let encoded = serde_json::json!({ "legacy": value }).to_string();

        let parsed = parse_agent_store_document(&encoded).unwrap();
        assert!(parsed.migrated_from_legacy);
        assert_eq!(parsed.default_agent_id, None);
        assert!(
            parsed
                .agents
                .iter()
                .find(|agent| agent.agent_id == "legacy")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn null_default_is_emitted_in_canonical_envelope() {
        let encoded =
            serialize_agent_store_document(&builtin_provider_agent_profiles(), None).unwrap();
        let wire: Value = serde_json::from_str(&encoded).unwrap();
        assert!(wire.as_object().unwrap().contains_key("default_agent_id"));
        assert!(wire["default_agent_id"].is_null());
    }
}
