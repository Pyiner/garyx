use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_router::{
    ThreadTranscriptStore, history_message_count,
    is_hidden_thread_value, is_thread_key, workspace_dir_from_value,
};
use serde_json::Value;
use std::sync::{Arc, Weak};

use crate::garyx_db::RecentThreadDraft;
use crate::thread_type::thread_summary_type_from_record;
use crate::transcript_run_projection::active_run_id_from_transcript_store;

pub(crate) const RECENT_THREAD_MISSING_TIMESTAMP: &str = "1970-01-01T00:00:00.000Z";

/// In-memory confirmation that a run is still actually executing, backed by the
/// bridge run index. Used to veto a transcript `running` state left dangling by
/// a crash: the transcript keeps a `run_start` with no paired close, but the
/// run index is rebuilt empty on restart so the orphan resolves to idle.
#[async_trait]
pub(crate) trait ActiveRunProbe: Send + Sync {
    async fn is_run_active(&self, run_id: &str) -> bool;
}

/// Bridge-backed active-run probe. Holds a `Weak` because the bridge owns an
/// `Arc` to the projecting thread store (`set_thread_store_blocking`); an `Arc`
/// back would form a reference cycle.
pub(crate) struct BridgeActiveRunProbe {
    bridge: Weak<MultiProviderBridge>,
}

impl BridgeActiveRunProbe {
    pub(crate) fn new(bridge: Weak<MultiProviderBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl ActiveRunProbe for BridgeActiveRunProbe {
    async fn is_run_active(&self, run_id: &str) -> bool {
        match self.bridge.upgrade() {
            Some(bridge) => bridge.is_run_active(run_id).await,
            None => false,
        }
    }
}

/// Resolve the authoritative active run id for a thread: the transcript's
/// reduced active run, gated by in-memory confirmation that the run is still
/// executing. Returns `None` (idle) when the transcript shows no open run or
/// when the bridge no longer holds the run (crash orphan).
pub(crate) async fn resolve_active_run_id(
    transcript_store: &Arc<ThreadTranscriptStore>,
    probe: &dyn ActiveRunProbe,
    thread_id: &str,
) -> Option<String> {
    let active_run_id = active_run_id_from_transcript_store(transcript_store, thread_id).await?;
    if probe.is_run_active(&active_run_id).await {
        Some(active_run_id)
    } else {
        None
    }
}

/// Probe that reports no run as active. Used where no bridge exists yet
/// (the AppStateBuilder default store): cold-start semantics — every
/// transcript-open run resolves as a crash orphan, i.e. idle.
pub(crate) struct IdleActiveRunProbe;

#[async_trait]
impl ActiveRunProbe for IdleActiveRunProbe {
    async fn is_run_active(&self, _run_id: &str) -> bool {
        false
    }
}

/// Test probe that reports every run as active, so route/projection tests can
/// seed a busy transcript and have it project as `running` without standing up
/// a real bridge run. Crash-orphan behavior is covered by tests that use a
/// probe reporting inactive.
#[cfg(test)]
pub(crate) struct AlwaysActiveRunProbe;

#[cfg(test)]
#[async_trait]
impl ActiveRunProbe for AlwaysActiveRunProbe {
    async fn is_run_active(&self, _run_id: &str) -> bool {
        true
    }
}

pub(crate) fn recent_thread_draft_from_thread_data_with_active_run(
    thread_id: &str,
    data: &Value,
    active_run_id: Option<String>,
) -> Option<RecentThreadDraft> {
    let thread_id = thread_id.trim();
    if !is_thread_key(thread_id) || is_hidden_thread_value(data) || is_recent_thread_excluded(data)
    {
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
    let thread_type = thread_summary_type_from_record(data);
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

pub(crate) fn is_recent_thread_excluded(data: &Value) -> bool {
    if bool_field(data, "exclude_from_recent") {
        return true;
    }
    if string_field(data, "source").is_some_and(|value| value == "workflow") {
        return true;
    }
    if string_field(data, "workflow_child_run_id").is_some_and(|value| !value.is_empty()) {
        return true;
    }
    if string_field(data, "automation_thread_mode").is_some_and(|value| value == "generated_thread")
    {
        return true;
    }
    let Some(metadata) = data.get("metadata") else {
        return false;
    };
    bool_field(metadata, "exclude_from_recent")
        || string_field(metadata, "source").is_some_and(|value| value == "workflow")
        || string_field(metadata, "workflow_child_run_id").is_some_and(|value| !value.is_empty())
        || string_field(metadata, "automation_thread_mode")
            .is_some_and(|value| value == "generated_thread")
}

fn bool_field(data: &Value, key: &str) -> bool {
    match data.get(key) {
        Some(Value::Bool(true)) => true,
        Some(Value::String(value)) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "1"
        ),
        _ => false,
    }
}

fn string_field(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
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
    // Write-time preview fields are the source (#TASK-1864 batch 1).
    if let Some(preview) = garyx_models::message_preview::preview_field_for_role(role)
        .and_then(|field| data.get(field))
        .and_then(Value::as_str)
    {
        return Some(preview.to_owned());
    }
    // Legacy fallback for records not yet touched by a post-batch-1 run;
    // deleted after Batch 2's import backfills the fields.
    let messages = data.get("messages").and_then(Value::as_array)?;
    last_message_preview_in_messages(messages.iter(), role)
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn generated_automation_threads_are_not_projectable_recent_threads() {
        let data = json!({
            "label": "Daily automation",
            "automation_thread_mode": "generated_thread",
            "exclude_from_recent": true,
            "updated_at": "2026-01-01T00:00:01Z",
        });

        assert!(is_recent_thread_excluded(&data));
        assert!(
            recent_thread_draft_from_thread_data_with_active_run("thread::automation", &data, None)
                .is_none()
        );
    }

    #[test]
    fn workflow_child_threads_are_not_projectable_recent_threads() {
        let data = json!({
            "label": "Workflow child",
            "source": "workflow",
            "workflow_child_run_id": "workflow-child::one",
            "updated_at": "2026-01-01T00:00:01Z",
        });

        assert!(is_recent_thread_excluded(&data));
        assert!(
            recent_thread_draft_from_thread_data_with_active_run("thread::workflow", &data, None)
                .is_none()
        );
    }
}