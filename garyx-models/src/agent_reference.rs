use std::collections::HashMap;

use serde_json::Value;

use crate::provider::ProviderType;
use crate::CustomAgentProfile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentReference {
    Standalone {
        requested_id: String,
        profile: CustomAgentProfile,
    },
}

impl AgentReference {
    pub fn requested_id(&self) -> &str {
        match self {
            Self::Standalone { requested_id, .. } => requested_id,
        }
    }

    /// The canonical agent_id that this reference binds to on a thread.
    ///
    pub fn bound_agent_id(&self) -> &str {
        match self {
            Self::Standalone { profile, .. } => &profile.agent_id,
        }
    }

    pub fn provider_type(&self) -> ProviderType {
        match self {
            Self::Standalone { profile, .. } => profile.provider_type.clone(),
        }
    }
}

/// Resolve a requested agent id against the standalone agent catalog.
pub fn resolve_agent_reference(
    requested_id: &str,
    agents: &[CustomAgentProfile],
) -> Result<AgentReference, String> {
    let normalized = requested_id.trim();
    if normalized.is_empty() {
        return Err("agent_id is required".to_owned());
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
    }
    metadata
}

pub fn agent_provider_env_metadata(reference: &AgentReference) -> HashMap<String, Value> {
    let AgentReference::Standalone { profile, .. } = reference;
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

#[cfg(test)]
mod tests;
