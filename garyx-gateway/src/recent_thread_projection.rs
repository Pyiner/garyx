use async_trait::async_trait;
use garyx_router::{
    ThreadStore, ThreadStoreError, active_run_snapshot_messages, active_run_snapshot_run_id,
    history_message_count, is_hidden_thread_value, is_thread_key, thread_kind_from_value,
    workspace_dir_from_value,
};
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

use crate::garyx_db::{GaryxDbService, RecentThreadDraft};

pub(crate) const RECENT_THREAD_MISSING_TIMESTAMP: &str = "1970-01-01T00:00:00.000Z";

pub(crate) struct RecentThreadProjectingStore {
    inner: Arc<dyn ThreadStore>,
    garyx_db: Arc<GaryxDbService>,
}

impl RecentThreadProjectingStore {
    pub(crate) fn new(inner: Arc<dyn ThreadStore>, garyx_db: Arc<GaryxDbService>) -> Self {
        Self { inner, garyx_db }
    }

    fn project_thread(&self, thread_id: &str, data: &Value) {
        if !is_thread_key(thread_id) {
            return;
        }
        if is_hidden_thread_value(data) {
            if let Err(error) = self.garyx_db.remove_recent_thread(thread_id) {
                warn!(thread_id, error = %error, "failed to remove hidden thread from recent thread projection");
            }
            return;
        }
        let Some(draft) = recent_thread_draft_from_thread_data(thread_id, data) else {
            return;
        };
        if let Err(error) = self.garyx_db.upsert_recent_thread(draft) {
            warn!(thread_id, error = %error, "failed to upsert recent thread projection");
        }
    }
}

pub(crate) async fn backfill_recent_thread_projection_if_empty(
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &GaryxDbService,
) -> usize {
    match garyx_db.count_recent_threads() {
        Ok(count) if count > 0 => return 0,
        Ok(_) => {}
        Err(error) => {
            warn!(error = %error, "failed to count recent thread projection before backfill");
            return 0;
        }
    }

    let mut drafts = Vec::new();
    for thread_id in thread_store.list_keys(Some("thread::")).await {
        let Some(data) = thread_store.get(&thread_id).await else {
            continue;
        };
        if let Some(draft) = recent_thread_draft_from_thread_data(&thread_id, &data) {
            drafts.push(draft);
        }
    }
    let count = drafts.len();
    if let Err(error) = garyx_db.sync_recent_threads_snapshot(drafts, usize::MAX) {
        warn!(error = %error, "failed to backfill recent thread projection");
        return 0;
    }
    count
}

#[async_trait]
impl ThreadStore for RecentThreadProjectingStore {
    async fn get(&self, thread_id: &str) -> Option<Value> {
        self.inner.get(thread_id).await
    }

    async fn set(&self, thread_id: &str, data: Value) {
        self.inner.set(thread_id, data.clone()).await;
        self.project_thread(thread_id, &data);
    }

    async fn delete(&self, thread_id: &str) -> bool {
        let deleted = self.inner.delete(thread_id).await;
        if deleted
            && is_thread_key(thread_id)
            && let Err(error) = self.garyx_db.remove_recent_thread(thread_id)
        {
            warn!(thread_id, error = %error, "failed to remove deleted thread from recent thread projection");
        }
        deleted
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Vec<String> {
        self.inner.list_keys(prefix).await
    }

    async fn exists(&self, thread_id: &str) -> bool {
        self.inner.exists(thread_id).await
    }

    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
        self.inner.update(thread_id, updates).await?;
        if let Some(data) = self.inner.get(thread_id).await {
            self.project_thread(thread_id, &data);
        }
        Ok(())
    }
}

pub(crate) fn recent_thread_draft_from_thread_data(
    thread_id: &str,
    data: &Value,
) -> Option<RecentThreadDraft> {
    let thread_id = thread_id.trim();
    if !is_thread_key(thread_id) || is_hidden_thread_value(data) {
        return None;
    }
    let title = data
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New Thread")
        .to_owned();
    let workspace_dir = workspace_dir_from_value(data);
    let thread_type = thread_kind_from_value(data).unwrap_or_else(|| "chat".to_owned());
    let provider_type = data
        .get("provider_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let agent_id = data
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let message_count = history_message_count(data).min(u32::MAX as usize) as u32;
    let recent_run_id = data
        .get("history")
        .and_then(|history| history.get("recent_committed_run_ids"))
        .and_then(Value::as_array)
        .and_then(|entries| entries.last())
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let active_run_id = active_run_snapshot_run_id(data);
    let updated_at = data
        .get("updated_at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let created_at = data
        .get("created_at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let last_active_at = updated_at
        .clone()
        .or(created_at)
        .unwrap_or_else(|| RECENT_THREAD_MISSING_TIMESTAMP.to_owned());
    let run_state = recent_thread_run_state(active_run_id.as_deref(), recent_run_id.as_deref());

    Some(RecentThreadDraft {
        thread_id: thread_id.to_owned(),
        title,
        workspace_dir,
        thread_type,
        provider_type,
        agent_id,
        message_count,
        last_message_preview: last_message_preview(data).unwrap_or_default(),
        recent_run_id,
        active_run_id,
        run_state,
        updated_at,
        last_active_at,
    })
}

fn recent_thread_run_state(active_run_id: Option<&str>, recent_run_id: Option<&str>) -> String {
    if active_run_id
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return "running".to_owned();
    }
    if recent_run_id
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return "completed".to_owned();
    }
    "idle".to_owned()
}

fn last_message_preview(data: &Value) -> Option<String> {
    last_message_preview_for_role(data, "user")
        .or_else(|| last_message_preview_for_role(data, "assistant"))
}

fn last_message_preview_for_role(data: &Value, role: &str) -> Option<String> {
    let active_messages = active_run_snapshot_messages(data);
    last_message_preview_in_messages(active_messages.iter(), role).or_else(|| {
        let messages = data.get("messages").and_then(Value::as_array)?;
        last_message_preview_in_messages(messages.iter(), role)
    })
}

fn last_message_preview_in_messages<'a>(
    messages: impl DoubleEndedIterator<Item = &'a Value>,
    role: &str,
) -> Option<String> {
    for message in messages.rev() {
        let Some(obj) = message.as_object() else {
            continue;
        };
        if obj.get("role").and_then(Value::as_str) != Some(role) {
            continue;
        }
        if let Some(summary) = summarize_message_content(obj.get("content")) {
            return Some(summary);
        }
        if let Some(summary) = summarize_message_content(obj.get("text")) {
            return Some(summary);
        }
    }
    None
}

fn summarize_message_content(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => summarize_text(text, 160),
        _ => None,
    }
}

fn summarize_text(value: &str, limit: usize) -> Option<String> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    let mut chars = text.chars();
    let mut summary = String::new();
    for _ in 0..limit {
        let Some(ch) = chars.next() else {
            return Some(summary);
        };
        summary.push(ch);
    }
    Some(summary + "…")
}
