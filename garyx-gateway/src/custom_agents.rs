use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
#[cfg(test)]
use garyx_models::AGENT_STORE_VERSION;
use garyx_models::{
    AgentAvailabilitySnapshot, CustomAgentProfile, ProviderType, builtin_provider_agent_profiles,
    parse_agent_store_document, resolve_effective_default, serialize_agent_store_document,
};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::optimistic_write::{StoreWriteError, WriteExpectation, check_write_expectation};

#[derive(Debug, Clone, Deserialize)]
pub struct UpsertCustomAgentRequest {
    pub agent_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub provider_type: ProviderType,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, alias = "modelReasoningEffort")]
    pub model_reasoning_effort: Option<String>,
    #[serde(default, alias = "modelServiceTier")]
    pub model_service_tier: Option<String>,
    #[serde(default, alias = "env", alias = "providerEnv")]
    pub provider_env: Option<HashMap<String, String>>,
    #[serde(
        default,
        alias = "defaultWorkspaceDir",
        alias = "workspace_dir",
        alias = "workspaceDir"
    )]
    pub default_workspace_dir: Option<String>,
    #[serde(default, alias = "avatarDataUrl")]
    pub avatar_data_url: Option<String>,
    #[serde(default, alias = "systemPrompt")]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone)]
struct AgentStoreState {
    agents: HashMap<String, CustomAgentProfile>,
    default_agent_id: Option<String>,
    revision: u64,
}

fn normalize_system_prompt(value: &str) -> String {
    value.trim().to_owned()
}

#[derive(Debug)]
pub struct CustomAgentStore {
    inner: RwLock<AgentStoreState>,
    persistence_path: Option<PathBuf>,
}

impl Default for CustomAgentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomAgentStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(AgentStoreState {
                agents: builtin_map(),
                default_agent_id: None,
                revision: 1,
            }),
            persistence_path: None,
        }
    }

    pub fn file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let mut state = AgentStoreState {
            agents: builtin_map(),
            default_agent_id: None,
            revision: 1,
        };
        let mut migrate_legacy = false;
        if path.exists() {
            let content = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
            if !content.trim().is_empty() {
                let parsed =
                    parse_agent_store_document(&content).map_err(|error| error.to_string())?;
                for agent_id in parsed.skipped_unsupported_agent_ids {
                    tracing::warn!(
                        agent_id,
                        "skipping persisted custom agent with unsupported provider type"
                    );
                }
                state.agents = parsed
                    .agents
                    .into_iter()
                    .map(|profile| (profile.agent_id.clone(), profile))
                    .collect();
                state.default_agent_id = parsed.default_agent_id;
                migrate_legacy = parsed.migrated_from_legacy;
            }
        }

        let store = Self {
            inner: RwLock::new(state),
            persistence_path: Some(path),
        };
        if migrate_legacy {
            let state = store
                .inner
                .try_read()
                .map_err(|_| "custom agent store lock unexpectedly contested".to_owned())?;
            store.persist_state(&state)?;
        }
        Ok(store)
    }

    /// Where this store persists to, if anywhere. `None` means the store is
    /// purely in-memory (the safe default for tests and ad-hoc states).
    pub fn persistence_path(&self) -> Option<&Path> {
        self.persistence_path.as_deref()
    }

    pub async fn snapshot(&self) -> AgentAvailabilitySnapshot {
        let state = self.inner.read().await;
        snapshot_from_state(&state)
    }

    /// Snapshot the whole decision unit without entering async context.
    pub fn snapshot_blocking(&self) -> AgentAvailabilitySnapshot {
        let state = self
            .inner
            .try_read()
            .expect("custom agent store lock contested during boot snapshot");
        snapshot_from_state(&state)
    }

    pub async fn list_agents(&self) -> Vec<CustomAgentProfile> {
        sorted_profiles(&self.inner.read().await.agents)
    }

    /// Snapshot the agent list without entering async context.
    pub fn list_agents_blocking(&self) -> Vec<CustomAgentProfile> {
        self.inner
            .try_read()
            .map(|state| sorted_profiles(&state.agents))
            .unwrap_or_default()
    }

    pub async fn default_agent_id(&self) -> Option<String> {
        self.inner.read().await.default_agent_id.clone()
    }

    pub async fn effective_default_agent_id(&self) -> Option<String> {
        resolve_effective_default(&self.snapshot().await).map(|binding| binding.agent_id)
    }

    pub async fn get_agent(&self, agent_id: &str) -> Option<CustomAgentProfile> {
        self.inner.read().await.agents.get(agent_id).cloned()
    }

    pub async fn upsert_agent(
        &self,
        request: UpsertCustomAgentRequest,
        expectation: WriteExpectation,
    ) -> Result<CustomAgentProfile, StoreWriteError> {
        let agent_id = request.agent_id.trim();
        let display_name = request.display_name.trim();
        let requested_model = request.model.map(|value| value.trim().to_owned());
        let requested_model_reasoning_effort = request
            .model_reasoning_effort
            .map(|value| value.trim().to_owned());
        let requested_model_service_tier = request
            .model_service_tier
            .map(|value| value.trim().to_owned());
        let provider_env = request.provider_env.map(|values| {
            values
                .into_iter()
                .filter_map(|(key, value)| {
                    let key = key.trim();
                    (!key.is_empty()).then(|| (key.to_owned(), value.trim().to_owned()))
                })
                .collect::<HashMap<_, _>>()
        });
        let requested_system_prompt = request
            .system_prompt
            .map(|value| normalize_system_prompt(&value));
        let requested_default_workspace_dir = request
            .default_workspace_dir
            .map(|value| value.trim().to_owned());
        let requested_avatar_data_url =
            request.avatar_data_url.map(|value| value.trim().to_owned());
        if agent_id.is_empty() {
            return Err(StoreWriteError::Invalid("agent_id is required".to_owned()));
        }
        if display_name.is_empty() {
            return Err(StoreWriteError::Invalid(
                "display_name is required".to_owned(),
            ));
        }

        let now = Utc::now().to_rfc3339();
        let mut state = self.inner.write().await;
        if state
            .agents
            .get(agent_id)
            .is_some_and(|existing| existing.built_in)
        {
            return Err(StoreWriteError::Invalid(
                "built-in agents cannot be modified".to_owned(),
            ));
        }
        check_write_expectation(
            &expectation,
            state
                .agents
                .get(agent_id)
                .map(|profile| profile.updated_at.as_str()),
            "custom agent",
        )?;

        let existing = state.agents.get(agent_id);
        let profile = CustomAgentProfile {
            agent_id: agent_id.to_owned(),
            display_name: display_name.to_owned(),
            provider_type: request.provider_type,
            enabled: request
                .enabled
                .or_else(|| existing.map(|profile| profile.enabled))
                .unwrap_or(true),
            model: requested_model
                .or_else(|| existing.map(|profile| profile.model.clone()))
                .unwrap_or_default(),
            model_reasoning_effort: requested_model_reasoning_effort
                .or_else(|| existing.map(|profile| profile.model_reasoning_effort.clone()))
                .unwrap_or_default(),
            model_service_tier: requested_model_service_tier
                .or_else(|| existing.map(|profile| profile.model_service_tier.clone()))
                .unwrap_or_default(),
            provider_env: provider_env
                .or_else(|| existing.map(|profile| profile.provider_env.clone()))
                .unwrap_or_default(),
            default_workspace_dir: match requested_default_workspace_dir {
                Some(value) if value.is_empty() => None,
                Some(value) => Some(value),
                None => existing.and_then(|profile| profile.default_workspace_dir.clone()),
            },
            avatar_data_url: match requested_avatar_data_url {
                Some(value) if value.is_empty() => None,
                Some(value) => Some(value),
                None => existing.and_then(|profile| profile.avatar_data_url.clone()),
            },
            system_prompt: requested_system_prompt
                .or_else(|| existing.map(|profile| profile.system_prompt.clone()))
                .unwrap_or_default(),
            built_in: false,
            standalone: true,
            created_at: existing
                .map(|profile| profile.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };

        let mut next = state.clone();
        next.agents.insert(agent_id.to_owned(), profile.clone());
        next.revision = next.revision.saturating_add(1);
        self.persist_state(&next)
            .map_err(StoreWriteError::Persist)?;
        *state = next;
        Ok(profile)
    }

    /// Test-only unconditional upsert preserving the pre-#TASK-1761 seeding
    /// semantics (create-or-replace, `String` errors). Production writes must
    /// pick an explicit [`WriteExpectation`].
    #[cfg(test)]
    pub async fn upsert_agent_for_test(
        &self,
        request: UpsertCustomAgentRequest,
    ) -> Result<CustomAgentProfile, String> {
        self.upsert_agent(request, WriteExpectation::Overwrite)
            .await
            .map_err(|error| error.message().to_owned())
    }

    pub async fn set_enabled(
        &self,
        agent_id: &str,
        enabled: bool,
    ) -> Result<CustomAgentProfile, StoreWriteError> {
        let agent_id = agent_id.trim();
        let mut state = self.inner.write().await;
        let Some(existing) = state.agents.get(agent_id) else {
            return Err(StoreWriteError::NotFound("agent not found".to_owned()));
        };
        if existing.enabled == enabled {
            return Ok(existing.clone());
        }
        let mut profile = existing.clone();
        profile.enabled = enabled;
        if !profile.built_in {
            profile.updated_at = Utc::now().to_rfc3339();
        }
        let mut next = state.clone();
        next.agents.insert(agent_id.to_owned(), profile.clone());
        next.revision = next.revision.saturating_add(1);
        self.persist_state(&next)
            .map_err(StoreWriteError::Persist)?;
        *state = next;
        Ok(profile)
    }

    pub async fn set_default_agent(
        &self,
        agent_id: &str,
    ) -> Result<CustomAgentProfile, StoreWriteError> {
        let agent_id = agent_id.trim();
        let mut state = self.inner.write().await;
        let Some(profile) = state.agents.get(agent_id).cloned() else {
            return Err(StoreWriteError::NotFound("agent not found".to_owned()));
        };
        if !profile.standalone {
            return Err(StoreWriteError::Invalid(
                "default agent must be standalone".to_owned(),
            ));
        }
        if !profile.enabled {
            return Err(StoreWriteError::Invalid(format!(
                "agent is disabled: {agent_id}"
            )));
        }
        if state.default_agent_id.as_deref() == Some(agent_id) {
            return Ok(profile);
        }
        let mut next = state.clone();
        next.default_agent_id = Some(agent_id.to_owned());
        next.revision = next.revision.saturating_add(1);
        self.persist_state(&next)
            .map_err(StoreWriteError::Persist)?;
        *state = next;
        Ok(profile)
    }

    pub async fn delete_agent(&self, agent_id: &str) -> Result<(), StoreWriteError> {
        let agent_id = agent_id.trim();
        let mut state = self.inner.write().await;
        let Some(existing) = state.agents.get(agent_id) else {
            return Err(StoreWriteError::NotFound(
                "custom agent not found".to_owned(),
            ));
        };
        if existing.built_in {
            return Err(StoreWriteError::Invalid(
                "built-in agents cannot be deleted".to_owned(),
            ));
        }
        let mut next = state.clone();
        next.agents.remove(agent_id);
        if next.default_agent_id.as_deref() == Some(agent_id) {
            next.default_agent_id = None;
        }
        next.revision = next.revision.saturating_add(1);
        self.persist_state(&next)
            .map_err(StoreWriteError::Persist)?;
        *state = next;
        Ok(())
    }

    /// Serialize exactly the proposed state. Callers still hold the mutation
    /// lock and only swap `inner` after this succeeds.
    fn persist_state(&self, state: &AgentStoreState) -> Result<(), String> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let agents = sorted_profiles(&state.agents);
        let json = serialize_agent_store_document(&agents, state.default_agent_id.as_deref())
            .map_err(|error| error.to_string())?;
        crate::atomic_write::write_json_atomic(path, &json)
    }
}

fn builtin_map() -> HashMap<String, CustomAgentProfile> {
    builtin_provider_agent_profiles()
        .into_iter()
        .map(|profile| (profile.agent_id.clone(), profile))
        .collect()
}

fn sorted_profiles(agents: &HashMap<String, CustomAgentProfile>) -> Vec<CustomAgentProfile> {
    const BUILTIN_ORDER: &[&str] = &["claude", "codex", "traex", "antigravity"];
    let builtin_rank = |agent_id: &str| {
        BUILTIN_ORDER
            .iter()
            .position(|candidate| *candidate == agent_id)
            .unwrap_or(usize::MAX)
    };
    let mut profiles = agents.values().cloned().collect::<Vec<_>>();
    profiles.sort_by(|left, right| match (left.built_in, right.built_in) {
        (true, true) => builtin_rank(&left.agent_id).cmp(&builtin_rank(&right.agent_id)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => left.agent_id.cmp(&right.agent_id),
    });
    profiles
}

fn snapshot_from_state(state: &AgentStoreState) -> AgentAvailabilitySnapshot {
    AgentAvailabilitySnapshot {
        agents: sorted_profiles(&state.agents),
        default_agent_id: state.default_agent_id.clone(),
        agent_state_revision: state.revision,
    }
}

#[cfg(test)]
mod tests;
