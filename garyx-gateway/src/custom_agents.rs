use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use garyx_models::{CustomAgentProfile, ProviderType, builtin_provider_agent_profiles};
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Deserialize)]
pub struct UpsertCustomAgentRequest {
    pub agent_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub provider_type: ProviderType,
    #[serde(default)]
    pub model: String,
    pub system_prompt: String,
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
    /// Intended for boot-time invariant checks (see
    /// [`garyx_models::validate_agent_team_registry_uniqueness`]). Returns an
    /// empty vec if the lock is contested, which during startup should never
    /// happen.
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
    ) -> Result<CustomAgentProfile, String> {
        let agent_id = request.agent_id.trim();
        let display_name = request.display_name.trim();
        let model = request.model.trim();
        let system_prompt = request.system_prompt.trim();
        if agent_id.is_empty() {
            return Err("agent_id is required".to_owned());
        }
        if display_name.is_empty() {
            return Err("display_name is required".to_owned());
        }
        if system_prompt.is_empty() {
            return Err("system_prompt is required".to_owned());
        }
        let now = Utc::now().to_rfc3339();
        let mut inner = self.inner.write().await;
        if inner
            .get(agent_id)
            .is_some_and(|existing| existing.built_in)
        {
            return Err("built-in agents cannot be modified".to_owned());
        }
        let created_at = inner
            .get(agent_id)
            .map(|existing| existing.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let profile = CustomAgentProfile {
            agent_id: agent_id.to_owned(),
            display_name: display_name.to_owned(),
            provider_type: request.provider_type,
            model: model.to_owned(),
            system_prompt: system_prompt.to_owned(),
            built_in: false,
            standalone: true,
            created_at,
            updated_at: now,
        };
        inner.insert(agent_id.to_owned(), profile.clone());
        drop(inner);
        self.persist().await?;
        Ok(profile)
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
        drop(inner);
        self.persist().await
    }

    async fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let snapshot = self
            .inner
            .read()
            .await
            .iter()
            .filter(|(_, profile)| !profile.built_in)
            .map(|(agent_id, profile)| (agent_id.clone(), profile.clone()))
            .collect::<HashMap<_, _>>();
        let json = serde_json::to_string_pretty(&snapshot).map_err(|error| error.to_string())?;
        std::fs::write(path, json).map_err(|error| error.to_string())
    }
}

fn builtin_profiles() -> Vec<CustomAgentProfile> {
    builtin_provider_agent_profiles()
}

#[cfg(test)]
mod tests;
