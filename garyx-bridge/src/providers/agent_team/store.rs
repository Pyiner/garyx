//! Group state store for the AgentTeam provider.
//!
//! `ThreadRecord` schema stays untouched for threads bound to a team. The
//! per-group runtime state — the mapping from sub-agent id to child thread id
//! and the catch-up offset per sub-agent — is owned by the provider and
//! persisted separately, keyed by the group thread's id.
//!
//! This module defines:
//! - [`Group`]: the serde-serializable state record for a single group.
//! - [`GroupStore`]: the async trait the provider consumes.
//! - [`FileGroupStore`]: a JSON-per-file implementation with a write-through
//!   in-memory cache.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

/// Provider-owned runtime state for one AgentTeam group.
///
/// Keyed externally by `group_thread_id` — the thread whose `agent_id`
/// resolves to the team.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Group {
    /// The thread whose `agent_id` is the team's id. This is the group's
    /// canonical identity; the group IS this thread from the user's POV.
    pub group_thread_id: String,
    /// The team profile id (AgentTeamProfile::id).
    pub team_id: String,
    /// agent_id -> child thread id, lazily populated as each sub-agent is
    /// first dispatched to.
    pub child_threads: HashMap<String, String>,
    /// agent_id -> next index into the group thread's transcript (`messages[]`)
    /// that still needs to be fed to that sub-agent as catch-up context.
    pub catch_up_offsets: HashMap<String, usize>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Group {
    /// Construct a fresh Group with empty maps and matching timestamps.
    pub fn new(group_thread_id: impl Into<String>, team_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            group_thread_id: group_thread_id.into(),
            team_id: team_id.into(),
            child_threads: HashMap::new(),
            catch_up_offsets: HashMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Record that `agent_id` has been spawned a child thread at `child_thread_id`.
    /// Bumps `updated_at`.
    pub fn record_child_thread(&mut self, agent_id: &str, child_thread_id: &str) {
        self.child_threads
            .insert(agent_id.to_owned(), child_thread_id.to_owned());
        self.updated_at = Utc::now();
    }

    /// Advance the catch-up offset for `agent_id` to `new_offset`. Bumps
    /// `updated_at`.
    pub fn advance_catch_up(&mut self, agent_id: &str, new_offset: usize) {
        self.catch_up_offsets
            .insert(agent_id.to_owned(), new_offset);
        self.updated_at = Utc::now();
    }

    /// Current catch-up offset for `agent_id`. Defaults to 0 if the agent has
    /// never been dispatched to.
    pub fn catch_up_offset(&self, agent_id: &str) -> usize {
        self.catch_up_offsets.get(agent_id).copied().unwrap_or(0)
    }

    /// Child thread id for `agent_id`, if one has been spawned.
    pub fn child_thread(&self, agent_id: &str) -> Option<&str> {
        self.child_threads.get(agent_id).map(String::as_str)
    }
}

/// Persistence interface for [`Group`] records.
///
/// Implementations must be safe to share across threads (`Send + Sync + 'static`)
/// so the provider can hand out a single handle to the whole dispatch loop.
#[async_trait]
pub trait GroupStore: Send + Sync + 'static {
    /// Load a group by its `group_thread_id`. Returns `None` if no group has
    /// been persisted for this id yet.
    async fn load(&self, group_thread_id: &str) -> Option<Group>;
    /// Persist `group`, overwriting any existing record with the same
    /// `group_thread_id`.
    async fn save(&self, group: &Group);
    /// Delete the group's record. No-op if no such group exists.
    async fn delete(&self, group_thread_id: &str);
}

/// File-backed [`GroupStore`]: one JSON file per group at
/// `<base_dir>/<sanitized_group_thread_id>.json`, plus a write-through
/// in-memory cache keyed by the *original* (unsanitized) id so hot-path
/// loads don't hit disk.
///
/// **Filename sanitization.** Thread ids in this workspace commonly contain
/// `:` (e.g. `th::abc-123`), which is a path separator on some filesystems
/// and invalid in filenames on others. To keep the mapping injective and
/// deterministic we replace every `:` and `/` byte with `_`. Because both
/// `a:b` and `a/b` would map to `a_b`, tests assert that each unique id
/// stably round-trips through the same file — callers in the real system
/// always pass UUID-like thread ids, so the collision space is effectively
/// nil. (The sister `FileThreadStore` solves the same problem with hex
/// encoding; for this store we deliberately keep the filename
/// human-readable for debugging, since groups are few and long-lived.)
pub struct FileGroupStore {
    base_dir: PathBuf,
    cache: RwLock<HashMap<String, Group>>,
}

impl FileGroupStore {
    /// Create a new store rooted at `base_dir`. The directory is NOT created
    /// eagerly — the first `save` call runs `create_dir_all`.
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Map a `group_thread_id` to its on-disk filename. See the struct-level
    /// doc comment for the sanitization rules.
    fn sanitize_filename(group_thread_id: &str) -> String {
        let mut out = String::with_capacity(group_thread_id.len());
        for ch in group_thread_id.chars() {
            match ch {
                ':' | '/' | '\\' => out.push('_'),
                _ => out.push(ch),
            }
        }
        out
    }

    fn file_path(&self, group_thread_id: &str) -> PathBuf {
        let sanitized = Self::sanitize_filename(group_thread_id);
        self.base_dir.join(format!("{sanitized}.json"))
    }

    /// Atomic write: serialize to a sibling temp file and rename into place
    /// so readers never observe a partial file. Mirrors the pattern used in
    /// `garyx-router/src/file_store.rs::atomic_write`.
    fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, data)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[async_trait]
impl GroupStore for FileGroupStore {
    async fn load(&self, group_thread_id: &str) -> Option<Group> {
        // Cache hit → clone out and return.
        {
            let cache = self.cache.read().await;
            if let Some(group) = cache.get(group_thread_id) {
                return Some(group.clone());
            }
        }

        let path = self.file_path(group_thread_id);
        if !path.exists() {
            return None;
        }

        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                error!(group_thread_id, error = %e, "failed to read group file");
                return None;
            }
        };

        let group: Group = match serde_json::from_slice(&bytes) {
            Ok(g) => g,
            Err(e) => {
                warn!(group_thread_id, error = %e, "failed to parse group json");
                return None;
            }
        };

        let mut cache = self.cache.write().await;
        cache.insert(group_thread_id.to_owned(), group.clone());
        debug!(group_thread_id, "loaded group from disk");
        Some(group)
    }

    async fn save(&self, group: &Group) {
        // Ensure base dir exists lazily.
        if let Err(e) = fs::create_dir_all(&self.base_dir) {
            error!(
                group_thread_id = %group.group_thread_id,
                error = %e,
                "failed to create agent-team-groups dir"
            );
            return;
        }

        let path = self.file_path(&group.group_thread_id);
        let bytes = match serde_json::to_vec_pretty(group) {
            Ok(b) => b,
            Err(e) => {
                error!(
                    group_thread_id = %group.group_thread_id,
                    error = %e,
                    "failed to serialize group"
                );
                return;
            }
        };

        if let Err(e) = Self::atomic_write(&path, &bytes) {
            error!(
                group_thread_id = %group.group_thread_id,
                error = %e,
                "failed to write group file"
            );
            return;
        }

        // Write-through cache update.
        let mut cache = self.cache.write().await;
        cache.insert(group.group_thread_id.clone(), group.clone());
        debug!(group_thread_id = %group.group_thread_id, "saved group");
    }

    async fn delete(&self, group_thread_id: &str) {
        {
            let mut cache = self.cache.write().await;
            cache.remove(group_thread_id);
        }
        let path = self.file_path(group_thread_id);
        if path.exists() {
            if let Err(e) = fs::remove_file(&path) {
                error!(group_thread_id, error = %e, "failed to remove group file");
            } else {
                debug!(group_thread_id, "deleted group");
            }
        }
    }
}

#[cfg(test)]
mod tests;
