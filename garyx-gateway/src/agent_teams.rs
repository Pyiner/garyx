use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use garyx_models::AgentTeamProfile;
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Deserialize)]
pub struct UpsertAgentTeamRequest {
    pub team_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub leader_agent_id: String,
    #[serde(default)]
    pub member_agent_ids: Vec<String>,
    pub workflow_text: String,
}

#[derive(Debug, Default)]
pub struct AgentTeamStore {
    inner: RwLock<HashMap<String, AgentTeamProfile>>,
    persistence_path: Option<PathBuf>,
}

impl AgentTeamStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            persistence_path: None,
        }
    }

    pub fn file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut teams = HashMap::new();
        if path.exists() {
            let content = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
            if !content.trim().is_empty() {
                teams = serde_json::from_str::<HashMap<String, AgentTeamProfile>>(&content)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(Self {
            inner: RwLock::new(teams),
            persistence_path: Some(path),
        })
    }

    pub async fn list_teams(&self) -> Vec<AgentTeamProfile> {
        let mut teams = self
            .inner
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        teams.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.team_id.cmp(&right.team_id))
        });
        teams
    }

    /// Snapshot the team list without entering async context.
    ///
    /// Intended for boot-time invariant checks (see
    /// [`garyx_models::validate_agent_team_registry_uniqueness`]). Returns an
    /// empty vec if the lock is contested, which during startup should never
    /// happen.
    pub fn list_teams_blocking(&self) -> Vec<AgentTeamProfile> {
        let Ok(guard) = self.inner.try_read() else {
            return Vec::new();
        };
        let mut teams = guard.values().cloned().collect::<Vec<_>>();
        teams.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.team_id.cmp(&right.team_id))
        });
        teams
    }

    pub async fn get_team(&self, team_id: &str) -> Option<AgentTeamProfile> {
        self.inner.read().await.get(team_id).cloned()
    }

    pub async fn find_team_for_agent(&self, agent_id: &str) -> Option<AgentTeamProfile> {
        let normalized = agent_id.trim();
        if normalized.is_empty() {
            return None;
        }
        self.inner
            .read()
            .await
            .values()
            .find(|team| {
                team.member_agent_ids
                    .iter()
                    .any(|member| member == normalized)
            })
            .cloned()
    }

    pub async fn upsert_team(
        &self,
        request: UpsertAgentTeamRequest,
    ) -> Result<AgentTeamProfile, String> {
        let team_id = request.team_id.trim();
        let display_name = request.display_name.trim();
        let leader_agent_id = request.leader_agent_id.trim();
        let workflow_text = request.workflow_text.trim();
        if team_id.is_empty() {
            return Err("team_id is required".to_owned());
        }
        if display_name.is_empty() {
            return Err("display_name is required".to_owned());
        }
        if leader_agent_id.is_empty() {
            return Err("leader_agent_id is required".to_owned());
        }
        if workflow_text.is_empty() {
            return Err("workflow_text is required".to_owned());
        }
        let mut seen = HashSet::new();
        let member_agent_ids = request
            .member_agent_ids
            .into_iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .filter(|value| seen.insert(value.clone()))
            .collect::<Vec<_>>();
        if !member_agent_ids
            .iter()
            .any(|member| member == leader_agent_id)
        {
            return Err("leader_agent_id must appear in member_agent_ids".to_owned());
        }
        let now = Utc::now().to_rfc3339();
        let mut inner = self.inner.write().await;
        let created_at = inner
            .get(team_id)
            .map(|existing| existing.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let team = AgentTeamProfile {
            team_id: team_id.to_owned(),
            display_name: display_name.to_owned(),
            leader_agent_id: leader_agent_id.to_owned(),
            member_agent_ids,
            workflow_text: workflow_text.to_owned(),
            created_at,
            updated_at: now,
        };
        inner.insert(team_id.to_owned(), team.clone());
        drop(inner);
        self.persist().await?;
        Ok(team)
    }

    pub async fn delete_team(&self, team_id: &str) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        if inner.remove(team_id).is_none() {
            return Err("agent team not found".to_owned());
        }
        drop(inner);
        self.persist().await
    }

    async fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let snapshot = self.inner.read().await.clone();
        let json = serde_json::to_string_pretty(&snapshot).map_err(|error| error.to_string())?;
        std::fs::write(path, json).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
