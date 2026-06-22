use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_router::{
    ThreadStore, ThreadStoreError, ThreadTranscriptStore, history_message_count,
    is_hidden_thread_value, is_thread_key, thread_kind_from_value, workspace_dir_from_value,
};
use serde_json::Value;
use std::sync::{Arc, Weak};
use tracing::warn;

use crate::garyx_db::{GaryxDbService, RecentThreadDraft};
use crate::task_projection::task_projection_draft_from_thread_data;
use crate::thread_meta_projection::thread_meta_projection_from_thread_data_with_active_run;
use crate::transcript_run_projection::active_run_id_from_transcript_store;

pub(crate) const RECENT_THREAD_MISSING_TIMESTAMP: &str = "1970-01-01T00:00:00.000Z";
const RECENT_THREAD_PROJECTION_NAME: &str = "recent_threads";
const RECENT_THREAD_PROJECTION_VERSION: i64 = 1;

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

pub(crate) struct RecentThreadProjectingStore {
    inner: Arc<dyn ThreadStore>,
    garyx_db: Arc<GaryxDbService>,
    transcript_store: Arc<ThreadTranscriptStore>,
    active_run_probe: Arc<dyn ActiveRunProbe>,
}

impl RecentThreadProjectingStore {
    pub(crate) fn new(
        inner: Arc<dyn ThreadStore>,
        garyx_db: Arc<GaryxDbService>,
        transcript_store: Arc<ThreadTranscriptStore>,
        active_run_probe: Arc<dyn ActiveRunProbe>,
    ) -> Self {
        Self {
            inner,
            garyx_db,
            transcript_store,
            active_run_probe,
        }
    }

    async fn project_thread(&self, thread_id: &str, data: &Value) {
        if !is_thread_key(thread_id) {
            return;
        }
        let active_run_id = resolve_active_run_id(
            &self.transcript_store,
            self.active_run_probe.as_ref(),
            thread_id,
        )
        .await;
        match thread_meta_projection_from_thread_data_with_active_run(
            thread_id,
            data,
            active_run_id.clone(),
        ) {
            Some(draft) => {
                if let Err(error) = self.garyx_db.replace_thread_meta_projection(draft) {
                    warn!(thread_id, error = %error, "failed to upsert thread meta projection");
                }
            }
            None => {
                if let Err(error) = self.garyx_db.remove_thread_meta_projection(thread_id) {
                    warn!(thread_id, error = %error, "failed to remove thread meta projection");
                }
            }
        }
        match task_projection_draft_from_thread_data(thread_id, data) {
            Some(draft) => {
                if let Err(error) = self.garyx_db.replace_task_projection(draft) {
                    warn!(thread_id, error = %error, "failed to upsert task projection");
                }
            }
            None => {
                if let Err(error) = self.garyx_db.remove_task_projection(thread_id) {
                    warn!(thread_id, error = %error, "failed to remove task projection");
                }
            }
        }
        if is_hidden_thread_value(data) {
            if let Err(error) = self.garyx_db.remove_recent_thread(thread_id) {
                warn!(thread_id, error = %error, "failed to remove hidden thread from recent thread projection");
            }
            return;
        }
        if is_recent_thread_excluded(data) {
            if let Err(error) = self.garyx_db.remove_recent_thread(thread_id) {
                warn!(thread_id, error = %error, "failed to remove excluded thread from recent thread projection");
            }
            return;
        }
        let Some(draft) =
            recent_thread_draft_from_thread_data_with_active_run(thread_id, data, active_run_id)
        else {
            if let Err(error) = self.garyx_db.remove_recent_thread(thread_id) {
                warn!(thread_id, error = %error, "failed to remove non-projectable thread from recent thread projection");
            }
            return;
        };
        if let Err(error) = self.garyx_db.upsert_recent_thread(draft) {
            warn!(thread_id, error = %error, "failed to upsert recent thread projection");
        }
    }

    fn clear_archived_thread_projections(&self, thread_id: &str) {
        if let Err(error) = self.garyx_db.unpin_thread(thread_id) {
            warn!(thread_id, error = %error, "failed to unpin archived thread");
        }
        if let Err(error) = self.garyx_db.remove_recent_thread(thread_id) {
            warn!(thread_id, error = %error, "failed to remove archived thread from recent projection");
        }
        if let Err(error) = self.garyx_db.remove_thread_meta_projection(thread_id) {
            warn!(thread_id, error = %error, "failed to remove archived thread meta projection");
        }
        if let Err(error) = self.garyx_db.remove_task_projection(thread_id) {
            warn!(thread_id, error = %error, "failed to remove archived task projection");
        }
    }

    async fn reject_archived_thread_write(&self, thread_id: &str) -> bool {
        if !is_thread_key(thread_id) {
            return false;
        }
        match self.garyx_db.is_thread_archived(thread_id) {
            Ok(true) => {
                self.inner.delete(thread_id).await;
                self.clear_archived_thread_projections(thread_id);
                true
            }
            Ok(false) => false,
            Err(error) => {
                warn!(thread_id, error = %error, "failed to check archived thread tombstone before projection");
                false
            }
        }
    }
}

pub(crate) async fn backfill_recent_thread_projection_if_incomplete(
    thread_store: &Arc<dyn ThreadStore>,
    transcript_store: &Arc<ThreadTranscriptStore>,
    garyx_db: &GaryxDbService,
    probe: &dyn ActiveRunProbe,
) -> usize {
    match garyx_db.count_recent_threads() {
        Ok(count) if count > 0 => return 0,
        Ok(_) => {}
        Err(error) => {
            warn!(error = %error, "failed to count recent thread projection before backfill");
            return 0;
        }
    }

    let thread_ids = thread_store.list_keys(Some("thread::")).await;
    match garyx_db.projection_state_matches(
        RECENT_THREAD_PROJECTION_NAME,
        RECENT_THREAD_PROJECTION_VERSION,
        thread_ids.len(),
    ) {
        Ok(true) => return 0,
        Ok(false) => {}
        Err(error) => {
            warn!(error = %error, "failed to check recent thread projection before backfill");
            return 0;
        }
    }

    let mut drafts = Vec::new();
    let source_row_count = thread_ids.len();
    for thread_id in thread_ids {
        let Some(data) = thread_store.get(&thread_id).await else {
            continue;
        };
        let active_run_id = resolve_active_run_id(transcript_store, probe, &thread_id).await;
        if let Some(draft) =
            recent_thread_draft_from_thread_data_with_active_run(&thread_id, &data, active_run_id)
        {
            drafts.push(draft);
        }
    }
    let count = drafts.len();
    if let Err(error) = garyx_db.sync_recent_threads_snapshot(drafts, usize::MAX) {
        warn!(error = %error, "failed to backfill recent thread projection");
        return 0;
    }
    if let Err(error) = garyx_db.record_projection_state(
        RECENT_THREAD_PROJECTION_NAME,
        RECENT_THREAD_PROJECTION_VERSION,
        source_row_count,
    ) {
        warn!(error = %error, "failed to record recent thread projection state");
    }
    count
}

pub(crate) async fn reconcile_active_recent_thread_projection(
    thread_store: &Arc<dyn ThreadStore>,
    transcript_store: &Arc<ThreadTranscriptStore>,
    garyx_db: &GaryxDbService,
    probe: &dyn ActiveRunProbe,
) -> usize {
    let records = match garyx_db.list_recent_threads(usize::MAX, 0) {
        Ok(records) => records,
        Err(error) => {
            warn!(error = %error, "failed to list recent thread projection before active-run reconcile");
            return 0;
        }
    };

    let mut reconciled = 0;
    for record in records {
        let projection_is_active = record
            .active_run_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || record.run_state == "running";
        if !projection_is_active {
            continue;
        }

        let Some(data) = thread_store.get(&record.thread_id).await else {
            continue;
        };
        if is_hidden_thread_value(&data) || is_recent_thread_excluded(&data) {
            if let Err(error) = garyx_db.remove_recent_thread(&record.thread_id) {
                warn!(thread_id = %record.thread_id, error = %error, "failed to remove hidden or excluded active recent thread projection during reconcile");
            } else {
                reconciled += 1;
            }
            continue;
        }

        let active_run_id =
            resolve_active_run_id(transcript_store, probe, &record.thread_id).await;
        let Some(draft) = recent_thread_draft_from_thread_data_with_active_run(
            &record.thread_id,
            &data,
            active_run_id,
        ) else {
            continue;
        };
        if draft.active_run_id == record.active_run_id && draft.run_state == record.run_state {
            continue;
        }
        if let Err(error) = garyx_db.upsert_recent_thread(draft) {
            warn!(thread_id = %record.thread_id, error = %error, "failed to reconcile active recent thread projection");
            continue;
        }
        reconciled += 1;
    }
    reconciled
}

pub(crate) async fn prune_excluded_recent_thread_projection(
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &GaryxDbService,
) -> usize {
    let records = match garyx_db.list_recent_threads(usize::MAX, 0) {
        Ok(records) => records,
        Err(error) => {
            warn!(error = %error, "failed to list recent thread projection before exclusion prune");
            return 0;
        }
    };

    let mut pruned = 0;
    for record in records {
        let should_remove = match thread_store.get(&record.thread_id).await {
            Some(data) => is_hidden_thread_value(&data) || is_recent_thread_excluded(&data),
            // An unresolved read is not proof the thread should leave the recent
            // list. A concurrent gateway, a not-yet-warm store, or a path mismatch
            // can transiently fail to load an existing thread; pruning on that
            // signal once wiped the whole projection. Genuine deletions are removed
            // explicitly via `remove_recent_thread`, so only drop rows we can
            // positively confirm are hidden or excluded.
            None => false,
        };
        if !should_remove {
            continue;
        }
        match garyx_db.remove_recent_thread(&record.thread_id) {
            Ok(true) => pruned += 1,
            Ok(false) => {}
            Err(error) => {
                warn!(thread_id = %record.thread_id, error = %error, "failed to prune excluded recent thread projection");
            }
        }
    }
    pruned
}

#[async_trait]
impl ThreadStore for RecentThreadProjectingStore {
    async fn get(&self, thread_id: &str) -> Option<Value> {
        self.inner.get(thread_id).await
    }

    async fn set(&self, thread_id: &str, data: Value) {
        if self.reject_archived_thread_write(thread_id).await {
            return;
        }
        self.inner.set(thread_id, data.clone()).await;
        self.project_thread(thread_id, &data).await;
    }

    async fn delete(&self, thread_id: &str) -> bool {
        let deleted = self.inner.delete(thread_id).await;
        if deleted
            && is_thread_key(thread_id)
            && let Err(error) = self.garyx_db.remove_recent_thread(thread_id)
        {
            warn!(thread_id, error = %error, "failed to remove deleted thread from recent thread projection");
        }
        if deleted
            && is_thread_key(thread_id)
            && let Err(error) = self.garyx_db.remove_thread_meta_projection(thread_id)
        {
            warn!(thread_id, error = %error, "failed to remove deleted thread meta projection");
        }
        if deleted
            && is_thread_key(thread_id)
            && let Err(error) = self.garyx_db.remove_task_projection(thread_id)
        {
            warn!(thread_id, error = %error, "failed to remove deleted task projection");
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
        if self.reject_archived_thread_write(thread_id).await {
            return Err(ThreadStoreError::NotFound(thread_id.to_owned()));
        }
        self.inner.update(thread_id, updates).await?;
        if let Some(data) = self.inner.get(thread_id).await {
            self.project_thread(thread_id, &data).await;
        }
        Ok(())
    }
}

fn recent_thread_draft_from_thread_data_with_active_run(
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
    use super::*;
    use garyx_router::{InMemoryThreadStore, RunTranscriptRecordDraft};
    use serde_json::json;

    struct FakeActiveRunProbe {
        active: bool,
    }

    #[async_trait]
    impl ActiveRunProbe for FakeActiveRunProbe {
        async fn is_run_active(&self, _run_id: &str) -> bool {
            self.active
        }
    }

    /// Seed a `run_start` control with no paired close, so the transcript
    /// reduces to busy (active_run_id = run_id) — the shape a crash leaves.
    async fn seed_busy_run(
        transcript_store: &Arc<ThreadTranscriptStore>,
        thread_id: &str,
        run_id: &str,
    ) {
        transcript_store
            .append_run_records(
                thread_id,
                Some(run_id),
                &[RunTranscriptRecordDraft {
                    timestamp: Some("2026-01-01T00:00:00Z".to_owned()),
                    message: json!({
                        "role": "system",
                        "kind": "control",
                        "internal": true,
                        "internal_kind": "control",
                        "control": {
                            "kind": "run_start",
                            "thread_id": thread_id,
                            "run_id": run_id,
                        },
                    }),
                }],
            )
            .await
            .expect("seed run_start");
    }

    fn stale_active_draft(thread_id: &str) -> RecentThreadDraft {
        RecentThreadDraft {
            thread_id: thread_id.to_owned(),
            title: "Stale Thread".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 1,
            last_message_preview: "hello".to_owned(),
            recent_run_id: Some("run::done".to_owned()),
            active_run_id: Some("run::stale".to_owned()),
            run_state: "running".to_owned(),
            updated_at: Some("2026-01-01T00:00:00Z".to_owned()),
            last_active_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    #[tokio::test]
    async fn reconcile_active_recent_thread_projection_clears_stale_active_run() {
        let thread_id = "thread::stale-active-projection";
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        thread_store
            .set(
                thread_id,
                json!({
                    "label": "Finished Thread",
                    "updated_at": "2026-01-01T00:00:01Z",
                    "history": {
                        "recent_committed_run_ids": ["run::done"]
                    },
                    "messages": [
                        {"role": "user", "content": "hello"},
                        {"role": "assistant", "content": "done"}
                    ]
                }),
            )
            .await;
        let garyx_db = GaryxDbService::memory().expect("memory db");
        garyx_db
            .upsert_recent_thread(stale_active_draft(thread_id))
            .expect("seed stale recent thread");
        let transcript_store = Arc::new(ThreadTranscriptStore::memory());

        let probe = FakeActiveRunProbe { active: false };
        let count = reconcile_active_recent_thread_projection(
            &thread_store,
            &transcript_store,
            &garyx_db,
            &probe,
        )
        .await;

        assert_eq!(count, 1);
        let records = garyx_db
            .list_recent_threads(10, 0)
            .expect("list recent threads");
        assert_eq!(records[0].thread_id, thread_id);
        assert_eq!(records[0].active_run_id, None);
        assert_eq!(records[0].run_state, "completed");
    }

    #[tokio::test]
    async fn project_clears_running_when_run_not_active_in_memory() {
        // Transcript still shows an open run (crash left no close), but the
        // bridge no longer holds it → the badge must resolve to idle.
        let thread_id = "thread::orphan-running";
        let run_id = "run::orphan";
        let inner: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let transcript_store = Arc::new(ThreadTranscriptStore::memory());
        seed_busy_run(&transcript_store, thread_id, run_id).await;

        let probe: Arc<dyn ActiveRunProbe> = Arc::new(FakeActiveRunProbe { active: false });
        let store =
            RecentThreadProjectingStore::new(inner, garyx_db.clone(), transcript_store, probe);
        store
            .set(
                thread_id,
                json!({
                    "label": "Orphan",
                    "updated_at": "2026-01-01T00:00:00Z",
                    "messages": [{"role": "user", "content": "hi"}]
                }),
            )
            .await;

        let records = garyx_db.list_recent_threads(10, 0).expect("list");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].active_run_id, None);
        assert_ne!(records[0].run_state, "running");
    }

    #[tokio::test]
    async fn project_keeps_running_when_run_active_in_memory() {
        // Transcript shows an open run and the bridge confirms it is live → running.
        let thread_id = "thread::live-running";
        let run_id = "run::live";
        let inner: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let transcript_store = Arc::new(ThreadTranscriptStore::memory());
        seed_busy_run(&transcript_store, thread_id, run_id).await;

        let probe: Arc<dyn ActiveRunProbe> = Arc::new(FakeActiveRunProbe { active: true });
        let store =
            RecentThreadProjectingStore::new(inner, garyx_db.clone(), transcript_store, probe);
        store
            .set(
                thread_id,
                json!({
                    "label": "Live",
                    "updated_at": "2026-01-01T00:00:00Z",
                    "messages": [{"role": "user", "content": "hi"}]
                }),
            )
            .await;

        let records = garyx_db.list_recent_threads(10, 0).expect("list");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].active_run_id.as_deref(), Some(run_id));
        assert_eq!(records[0].run_state, "running");
    }

    #[tokio::test]
    async fn backfill_recent_thread_projection_keeps_existing_rows() {
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        thread_store
            .set(
                "thread::legacy-kept",
                json!({
                    "label": "Legacy Kept",
                    "updated_at": "2026-01-01T00:00:02Z",
                    "message_count": 2,
                    "messages": [{"role": "user", "content": "kept"}]
                }),
            )
            .await;
        thread_store
            .set(
                "thread::legacy-missing",
                json!({
                    "label": "Legacy Missing",
                    "updated_at": "2026-01-01T00:00:01Z",
                    "message_count": 1,
                    "messages": [{"role": "user", "content": "missing"}]
                }),
            )
            .await;
        let garyx_db = GaryxDbService::memory().expect("memory db");
        garyx_db
            .upsert_recent_thread(stale_active_draft("thread::legacy-kept"))
            .expect("seed stale partial recent row");
        garyx_db
            .record_projection_state(
                RECENT_THREAD_PROJECTION_NAME,
                RECENT_THREAD_PROJECTION_VERSION,
                2,
            )
            .expect("seed projection state");
        let transcript_store = Arc::new(ThreadTranscriptStore::memory());

        let backfilled = backfill_recent_thread_projection_if_incomplete(
            &thread_store,
            &transcript_store,
            &garyx_db,
            &FakeActiveRunProbe { active: false },
        )
        .await;

        assert_eq!(backfilled, 0);
        let records = garyx_db
            .list_recent_threads(10, 0)
            .expect("list retained recent threads");
        assert_eq!(
            records
                .into_iter()
                .map(|record| record.thread_id)
                .collect::<Vec<_>>(),
            vec!["thread::legacy-kept"]
        );
    }

    #[test]
    fn generated_automation_threads_are_not_projectable_recent_threads() {
        let data = json!({
            "label": "Daily automation",
            "automation_thread_mode": "generated_thread",
            "exclude_from_recent": true,
            "updated_at": "2026-01-01T00:00:01Z",
            "messages": [
                {"role": "user", "content": "run"}
            ]
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
            "messages": [
                {"role": "user", "content": "run"}
            ]
        });

        assert!(is_recent_thread_excluded(&data));
        assert!(
            recent_thread_draft_from_thread_data_with_active_run("thread::workflow", &data, None)
                .is_none()
        );
    }

    #[tokio::test]
    async fn projection_removes_existing_recent_row_when_thread_becomes_excluded() {
        let thread_id = "thread::automation-projection";
        let inner: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let transcript_store = Arc::new(ThreadTranscriptStore::memory());
        let probe: Arc<dyn ActiveRunProbe> = Arc::new(FakeActiveRunProbe { active: false });
        let store =
            RecentThreadProjectingStore::new(inner, garyx_db.clone(), transcript_store, probe);

        store
            .set(
                thread_id,
                json!({
                    "label": "Visible",
                    "updated_at": "2026-01-01T00:00:00Z",
                    "messages": [{"role": "user", "content": "hello"}]
                }),
            )
            .await;
        assert_eq!(
            garyx_db
                .list_recent_threads(10, 0)
                .expect("list visible")
                .len(),
            1
        );

        store
            .update(
                thread_id,
                json!({
                    "automation_thread_mode": "generated_thread",
                    "exclude_from_recent": true,
                    "updated_at": "2026-01-01T00:00:01Z"
                }),
            )
            .await
            .expect("update thread");

        assert!(
            garyx_db
                .list_recent_threads(10, 0)
                .expect("list after exclusion")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn prune_excluded_recent_thread_projection_removes_seeded_rows() {
        let thread_id = "thread::automation-prune";
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        thread_store
            .set(
                thread_id,
                json!({
                    "label": "Generated",
                    "automation_thread_mode": "generated_thread",
                    "exclude_from_recent": true,
                    "updated_at": "2026-01-01T00:00:01Z"
                }),
            )
            .await;
        let garyx_db = GaryxDbService::memory().expect("memory db");
        garyx_db
            .upsert_recent_thread(stale_active_draft(thread_id))
            .expect("seed recent row");

        let count = prune_excluded_recent_thread_projection(&thread_store, &garyx_db).await;

        assert_eq!(count, 1);
        assert!(
            garyx_db
                .list_recent_threads(10, 0)
                .expect("list after prune")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn prune_excluded_recent_thread_projection_keeps_unresolved_rows() {
        // A thread whose data cannot be loaded right now (empty store here; in
        // production a concurrent gateway, a cold store, or a path mismatch) must
        // not be pruned: an unresolved read is not proof of exclusion. Regression
        // guard for the projection wipe caused by pruning on `None`.
        let thread_id = "thread::unresolved-keep";
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let garyx_db = GaryxDbService::memory().expect("memory db");
        garyx_db
            .upsert_recent_thread(stale_active_draft(thread_id))
            .expect("seed recent row");

        let count = prune_excluded_recent_thread_projection(&thread_store, &garyx_db).await;

        assert_eq!(count, 0);
        assert_eq!(
            garyx_db
                .list_recent_threads(10, 0)
                .expect("list after prune")
                .len(),
            1
        );
    }
}
