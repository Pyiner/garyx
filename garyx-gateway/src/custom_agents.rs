use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use garyx_models::config::{
    default_garyx_native_max_tool_iterations, default_native_request_timeout,
};
use garyx_models::{CustomAgentProfile, ProviderType, builtin_provider_agent_profiles};
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
    pub model: Option<String>,
    #[serde(default, alias = "modelReasoningEffort")]
    pub model_reasoning_effort: Option<String>,
    #[serde(default, alias = "modelServiceTier")]
    pub model_service_tier: Option<String>,
    #[serde(default, alias = "env", alias = "providerEnv")]
    pub provider_env: Option<HashMap<String, String>>,
    #[serde(default, alias = "authSource")]
    pub auth_source: Option<String>,
    #[serde(default, alias = "baseUrl")]
    pub base_url: Option<String>,
    #[serde(default, alias = "codexHome")]
    pub codex_home: Option<String>,
    #[serde(default, alias = "maxToolIterations")]
    pub max_tool_iterations: Option<u32>,
    #[serde(default, alias = "requestTimeoutSeconds")]
    pub request_timeout_seconds: Option<u32>,
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

fn normalize_system_prompt(value: &str) -> String {
    value.trim().to_owned()
}

#[derive(Debug, Default)]
pub struct CustomAgentStore {
    inner: RwLock<HashMap<String, CustomAgentProfile>>,
    persistence_path: Option<PathBuf>,
}

impl CustomAgentStore {
    pub fn new() -> Self {
        let builtins = builtin_profiles()
            .into_iter()
            .map(|profile| (profile.agent_id.clone(), profile))
            .collect();
        Self {
            inner: RwLock::new(builtins),
            persistence_path: None,
        }
    }

    pub fn file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut agents = builtin_profiles()
            .into_iter()
            .map(|profile| (profile.agent_id.clone(), profile))
            .collect::<HashMap<_, _>>();
        if path.exists() {
            let content = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
            if !content.trim().is_empty() {
                let persisted =
                    serde_json::from_str::<HashMap<String, CustomAgentProfile>>(&content)
                        .map_err(|error| error.to_string())?;
                for (agent_id, profile) in persisted {
                    agents.insert(agent_id, profile);
                }
            }
        }
        Ok(Self {
            inner: RwLock::new(agents),
            persistence_path: Some(path),
        })
    }

    /// Where this store persists to, if anywhere. `None` means the store is
    /// purely in-memory (the safe default for tests and ad-hoc states).
    pub fn persistence_path(&self) -> Option<&Path> {
        self.persistence_path.as_deref()
    }

    pub async fn list_agents(&self) -> Vec<CustomAgentProfile> {
        let mut agents = self
            .inner
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        agents.sort_by(|left, right| {
            left.built_in
                .cmp(&right.built_in)
                .then_with(|| left.display_name.cmp(&right.display_name))
        });
        agents
    }

    /// Snapshot the agent list without entering async context.
    ///
    /// Intended for boot-time snapshots. Returns an empty vec if the lock is
    /// contested, which during startup should never happen.
    pub fn list_agents_blocking(&self) -> Vec<CustomAgentProfile> {
        let Ok(guard) = self.inner.try_read() else {
            return Vec::new();
        };
        let mut agents = guard.values().cloned().collect::<Vec<_>>();
        agents.sort_by(|left, right| {
            left.built_in
                .cmp(&right.built_in)
                .then_with(|| left.display_name.cmp(&right.display_name))
        });
        agents
    }

    pub async fn get_agent(&self, agent_id: &str) -> Option<CustomAgentProfile> {
        self.inner.read().await.get(agent_id).cloned()
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
                    if key.is_empty() {
                        None
                    } else {
                        Some((key.to_owned(), value.trim().to_owned()))
                    }
                })
                .collect::<HashMap<_, _>>()
        });
        let auth_source = request.auth_source.map(|value| value.trim().to_owned());
        let base_url = request.base_url.map(|value| value.trim().to_owned());
        let codex_home = request.codex_home.map(|value| value.trim().to_owned());
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
        let mut inner = self.inner.write().await;
        if inner
            .get(agent_id)
            .is_some_and(|existing| existing.built_in)
        {
            return Err(StoreWriteError::Invalid(
                "built-in agents cannot be modified".to_owned(),
            ));
        }
        check_write_expectation(
            &expectation,
            inner
                .get(agent_id)
                .map(|profile| profile.updated_at.as_str()),
            "custom agent",
        )?;
        let created_at = inner
            .get(agent_id)
            .map(|existing| existing.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let default_workspace_dir = match requested_default_workspace_dir {
            Some(value) if value.is_empty() => None,
            Some(value) => Some(value),
            None => inner
                .get(agent_id)
                .and_then(|existing| existing.default_workspace_dir.clone()),
        };
        let avatar_data_url = match requested_avatar_data_url {
            Some(value) if value.is_empty() => None,
            Some(value) => Some(value),
            None => inner
                .get(agent_id)
                .and_then(|existing| existing.avatar_data_url.clone()),
        };
        let existing = inner.get(agent_id);
        let model = requested_model
            .or_else(|| existing.map(|profile| profile.model.clone()))
            .unwrap_or_default();
        let model_reasoning_effort = requested_model_reasoning_effort
            .or_else(|| existing.map(|profile| profile.model_reasoning_effort.clone()))
            .unwrap_or_default();
        let model_service_tier = requested_model_service_tier
            .or_else(|| existing.map(|profile| profile.model_service_tier.clone()))
            .unwrap_or_default();
        let provider_env = provider_env
            .or_else(|| existing.map(|profile| profile.provider_env.clone()))
            .unwrap_or_default();
        let auth_source = auth_source
            .or_else(|| existing.map(|profile| profile.auth_source.clone()))
            .unwrap_or_default();
        let base_url = base_url
            .or_else(|| existing.map(|profile| profile.base_url.clone()))
            .unwrap_or_default();
        let codex_home = codex_home
            .or_else(|| existing.map(|profile| profile.codex_home.clone()))
            .unwrap_or_default();
        let system_prompt = requested_system_prompt
            .or_else(|| existing.map(|profile| profile.system_prompt.clone()))
            .unwrap_or_default();
        let max_tool_iterations = request
            .max_tool_iterations
            .or_else(|| existing.map(|profile| profile.max_tool_iterations))
            .unwrap_or_else(default_garyx_native_max_tool_iterations);
        let request_timeout_seconds = request
            .request_timeout_seconds
            .or_else(|| existing.map(|profile| profile.request_timeout_seconds))
            .unwrap_or(default_native_request_timeout() as u32);
        let profile = CustomAgentProfile {
            agent_id: agent_id.to_owned(),
            display_name: display_name.to_owned(),
            provider_type: request.provider_type,
            model,
            model_reasoning_effort,
            model_service_tier,
            provider_env,
            auth_source,
            base_url,
            codex_home,
            max_tool_iterations,
            request_timeout_seconds,
            default_workspace_dir,
            avatar_data_url,
            system_prompt,
            built_in: false,
            standalone: true,
            created_at,
            updated_at: now,
        };
        inner.insert(agent_id.to_owned(), profile.clone());
        self.persist_locked(&inner)
            .map_err(StoreWriteError::Persist)?;
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

    pub async fn delete_agent(&self, agent_id: &str) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        let Some(existing) = inner.get(agent_id) else {
            return Err("custom agent not found".to_owned());
        };
        if existing.built_in {
            return Err("built-in agents cannot be deleted".to_owned());
        }
        inner.remove(agent_id);
        self.persist_locked(&inner)
    }

    /// Persist while the caller still holds the write guard, so a mutation
    /// and its disk write form one critical section. Writers are strictly
    /// ordered and a stale snapshot can never land after a newer one — the
    /// lost-update shape that erased a real agent definition on 2026-07-06.
    fn persist_locked(&self, inner: &HashMap<String, CustomAgentProfile>) -> Result<(), String> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let snapshot = inner
            .iter()
            .filter(|(_, profile)| !profile.built_in)
            .map(|(agent_id, profile)| (agent_id.clone(), profile.clone()))
            .collect::<HashMap<_, _>>();
        let json = serde_json::to_string_pretty(&snapshot).map_err(|error| error.to_string())?;
        crate::atomic_write::write_json_atomic(path, &json)
    }
}

fn builtin_profiles() -> Vec<CustomAgentProfile> {
    builtin_provider_agent_profiles()
}

#[cfg(test)]
mod tests;
