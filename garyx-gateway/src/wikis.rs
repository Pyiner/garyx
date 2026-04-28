use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use garyx_models::WikiEntry;
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Deserialize)]
pub struct UpsertWikiRequest {
    pub wiki_id: String,
    pub display_name: String,
    pub path: String,
    pub topic: String,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateWikiStatsRequest {
    pub wiki_id: String,
    pub source_count: u32,
    pub page_count: u32,
}

#[derive(Debug, Default)]
pub struct WikiStore {
    inner: RwLock<HashMap<String, WikiEntry>>,
    persistence_path: Option<PathBuf>,
}

impl WikiStore {
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
        let mut wikis = HashMap::new();
        if path.exists() {
            let content = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
            if !content.trim().is_empty() {
                let persisted = serde_json::from_str::<HashMap<String, WikiEntry>>(&content)
                    .map_err(|error| error.to_string())?;
                for (wiki_id, entry) in persisted {
                    wikis.insert(wiki_id, entry);
                }
            }
        }
        Ok(Self {
            inner: RwLock::new(wikis),
            persistence_path: Some(path),
        })
    }

    pub async fn list_wikis(&self) -> Vec<WikiEntry> {
        let mut wikis = self
            .inner
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        wikis.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        wikis
    }

    pub async fn get_wiki(&self, wiki_id: &str) -> Option<WikiEntry> {
        self.inner.read().await.get(wiki_id).cloned()
    }

    pub async fn upsert_wiki(&self, request: UpsertWikiRequest) -> Result<WikiEntry, String> {
        let wiki_id = request.wiki_id.trim();
        let display_name = request.display_name.trim();
        let path = request.path.trim();
        let topic = request.topic.trim();
        if wiki_id.is_empty() {
            return Err("wiki_id is required".to_owned());
        }
        if display_name.is_empty() {
            return Err("display_name is required".to_owned());
        }
        if path.is_empty() {
            return Err("path is required".to_owned());
        }
        if topic.is_empty() {
            return Err("topic is required".to_owned());
        }
        let now = Utc::now().to_rfc3339();
        let mut inner = self.inner.write().await;
        let created_at = inner
            .get(wiki_id)
            .map(|existing| existing.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let source_count = inner
            .get(wiki_id)
            .map(|existing| existing.source_count)
            .unwrap_or(0);
        let page_count = inner
            .get(wiki_id)
            .map(|existing| existing.page_count)
            .unwrap_or(0);
        let entry = WikiEntry {
            wiki_id: wiki_id.to_owned(),
            display_name: display_name.to_owned(),
            path: path.to_owned(),
            topic: topic.to_owned(),
            agent_id: request.agent_id,
            source_count,
            page_count,
            created_at,
            updated_at: now,
        };
        inner.insert(wiki_id.to_owned(), entry.clone());
        drop(inner);
        self.persist().await?;
        Ok(entry)
    }

    pub async fn update_stats(&self, request: UpdateWikiStatsRequest) -> Result<WikiEntry, String> {
        let mut inner = self.inner.write().await;
        let Some(entry) = inner.get_mut(&request.wiki_id) else {
            return Err("wiki not found".to_owned());
        };
        entry.source_count = request.source_count;
        entry.page_count = request.page_count;
        entry.updated_at = Utc::now().to_rfc3339();
        let result = entry.clone();
        drop(inner);
        self.persist().await?;
        Ok(result)
    }

    pub async fn delete_wiki(&self, wiki_id: &str) -> Result<(), String> {
        let mut inner = self.inner.write().await;
        if inner.remove(wiki_id).is_none() {
            return Err("wiki not found".to_owned());
        }
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
            .map(|(wiki_id, entry)| (wiki_id.clone(), entry.clone()))
            .collect::<HashMap<_, _>>();
        let json = serde_json::to_string_pretty(&snapshot).map_err(|error| error.to_string())?;
        std::fs::write(path, json).map_err(|error| error.to_string())
    }
}
