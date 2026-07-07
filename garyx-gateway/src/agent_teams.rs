use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use garyx_models::AgentTeamProfile;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::optimistic_write::{StoreWriteError, WriteExpectation, check_write_expectation};

#[derive(Debug, Clone, Deserialize)]
pub struct UpsertAgentTeamRequest {
    pub team_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub leader_agent_id: String,
    #[serde(default)]
    pub member_agent_ids: Vec<String>,
    pub workflow_text: String,
    #[serde(default, alias = "avatarDataUrl")]
    pub avatar_data_url: Option<String>,
}

#[derive(Debug, Default)]
pub struct AgentTeamStore {
    inner: RwLock<HashMap<String, AgentTeamProfile>>,
    persistence_path: Option<PathBuf>,
    /// Serializes whole team mutations *including their handler-level side
    /// effects* (deleted-marker clearing, group-state reconciliation, thread
    /// tombstoning). The store's own write lock only covers the profile map,
    /// so without this a PUT that succeeded could interleave with a DELETE
    /// and clear the deletion markers the delete just wrote. Team management
    /// is low-frequency, so one store-wide mutex is fine.
    mutation_serial: tokio::sync::Mutex<()>,
}

impl AgentTeamStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            persistence_path: None,
            mutation_serial: tokio::sync::Mutex::new(()),
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
            mutation_serial: tokio::sync::Mutex::new(()),
        })
    }

    /// Where this store persists to, if anywhere. `None` means the store is
    /// purely in-memory (the safe default for tests and ad-hoc states).
    pub fn persistence_path(&self) -> Option<&Path> {
        self.persistence_path.as_deref()
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

    /// Take the mutation-serialization guard. Handlers must hold this across
    /// the store write *and* every side effect that must not interleave with
    /// a concurrent create/update/delete of any team.
    pub async fn lock_mutations(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.mutation_serial.lock().await
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
        expectation: WriteExpectation,
    ) -> Result<AgentTeamProfile, StoreWriteError> {
        let team_id = request.team_id.trim();
        let display_name = request.display_name.trim();
        let leader_agent_id = request.leader_agent_id.trim();
        let workflow_text = request.workflow_text.trim();
        let requested_avatar_data_url =
            request.avatar_data_url.map(|value| value.trim().to_owned());
        if team_id.is_empty() {
            return Err(StoreWriteError::Invalid("team_id is required".to_owned()));
        }
        if display_name.is_empty() {
            return Err(StoreWriteError::Invalid(
                "display_name is required".to_owned(),
            ));
        }
        if leader_agent_id.is_empty() {
            return Err(StoreWriteError::Invalid(
                "leader_agent_id is required".to_owned(),
            ));
        }
        if workflow_text.is_empty() {
            return Err(StoreWriteError::Invalid(
                "workflow_text is required".to_owned(),
            ));
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
            return Err(StoreWriteError::Invalid(
                "leader_agent_id must appear in member_agent_ids".to_owned(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        let mut inner = self.inner.write().await;
        check_write_expectation(
            &expectation,
            inner.get(team_id).map(|team| team.updated_at.as_str()),
            "agent team",
        )?;
        let created_at = inner
            .get(team_id)
            .map(|existing| existing.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let avatar_data_url = match requested_avatar_data_url {
            Some(value) if value.is_empty() => None,
            Some(value) => Some(value),
            None => inner
                .get(team_id)
                .and_then(|existing| existing.avatar_data_url.clone()),
        };
        let team = AgentTeamProfile {
            team_id: team_id.to_owned(),
            display_name: display_name.to_owned(),
            leader_agent_id: leader_agent_id.to_owned(),
            member_agent_ids,
            workflow_text: workflow_text.to_owned(),
            avatar_data_url,
            created_at,
            updated_at: now,
        };
        inner.insert(team_id.to_owned(), team.clone());
        self.persist_locked(&inner)
            .map_err(StoreWriteError::Persist)?;
        Ok(team)
    }

    /// Test-only unconditional upsert preserving the pre-#TASK-1761 seeding
    /// semantics (create-or-replace, `String` errors). Production writes must
    /// pick an explicit [`WriteExpectation`].
    #[cfg(test)]
    pub async fn upsert_team_for_test(
        &self,
        request: UpsertAgentTeamRequest,
    ) -> Result<AgentTeamProfile, String> {
        self.upsert_team(request, WriteExpectation::Overwrite)
            .await
            .map_err(|error| error.message().to_owned())
    }

    pub async fn delete_team(&self, team_id: &str) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        if inner.remove(team_id).is_none() {
            return Err("agent team not found".to_owned());
        }
        self.persist_locked(&inner)
    }

    /// Persist while the caller still holds the write guard, so a mutation
    /// and its disk write form one critical section and a stale snapshot can
    /// never land after a newer one (lost update).
    fn persist_locked(&self, inner: &HashMap<String, AgentTeamProfile>) -> Result<(), String> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let json = serde_json::to_string_pretty(inner).map_err(|error| error.to_string())?;
        crate::atomic_write::write_json_atomic(path, &json)
    }
}

#[cfg(test)]
mod tests;
