use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::SecondsFormat;
use garyx_models::{Principal, TaskNotificationTarget, TaskSource, ThreadTask};
use garyx_router::tasks::{
    TaskId, TaskListFilter, TaskProjectionReader, TaskSummary, register_task_projection_reader,
    task_from_record,
};
use garyx_router::{ThreadStore, is_thread_key};
use serde::Serialize;
use serde_json::Value;
use tracing::{debug, warn};

use crate::garyx_db::{GaryxDbService, TASK_PROJECTION_NAME, TaskProjectionDraft};

pub(crate) struct SqlTaskProjectionReader {
    thread_store: Arc<dyn ThreadStore>,
    garyx_db: Arc<GaryxDbService>,
}

impl SqlTaskProjectionReader {
    pub(crate) fn new(thread_store: Arc<dyn ThreadStore>, garyx_db: Arc<GaryxDbService>) -> Self {
        Self {
            thread_store,
            garyx_db,
        }
    }
}

pub(crate) fn register_gateway_task_projection_reader(
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &Arc<GaryxDbService>,
) -> Arc<dyn TaskProjectionReader> {
    let reader: Arc<dyn TaskProjectionReader> = Arc::new(SqlTaskProjectionReader::new(
        thread_store.clone(),
        garyx_db.clone(),
    ));
    register_task_projection_reader(thread_store, reader.clone());
    reader
}

#[async_trait]
impl TaskProjectionReader for SqlTaskProjectionReader {
    async fn is_current(&self) -> bool {
        match self.garyx_db.task_projection_is_current() {
            Ok(current) => current,
            Err(error) => {
                warn!(error = %error, "failed to check task projection current state");
                false
            }
        }
    }

    async fn ensure_current(&self) -> bool {
        if self.is_current().await {
            return true;
        }
        backfill_task_projection_if_incomplete(&self.thread_store, &self.garyx_db).await;
        self.is_current().await
    }

    async fn task_index_rows(&self) -> Vec<(u64, String)> {
        match self.garyx_db.task_index_rows() {
            Ok(rows) => rows,
            Err(error) => {
                warn!(error = %error, "failed to read task projection index rows");
                Vec::new()
            }
        }
    }

    async fn thread_id_for_number(&self, number: u64) -> Option<String> {
        match self.garyx_db.thread_id_for_number(number) {
            Ok(thread_id) => thread_id,
            Err(error) => {
                warn!(number, error = %error, "failed to read task projection number lookup");
                None
            }
        }
    }

    async fn has_running_subtask_targeting(&self, thread_id: &str) -> bool {
        match self.garyx_db.has_running_subtask_targeting(thread_id) {
            Ok(found) => found,
            Err(error) => {
                warn!(thread_id, error = %error, "failed to read task projection running-subtask gate");
                false
            }
        }
    }

    async fn list_task_summaries(
        &self,
        filter: &TaskListFilter,
    ) -> Option<(Vec<TaskSummary>, usize, bool)> {
        match self.garyx_db.list_task_summaries(filter) {
            Ok(page) => Some(page),
            Err(error) => {
                warn!(error = %error, "failed to list task projection summaries");
                None
            }
        }
    }

    async fn max_number(&self) -> Option<u64> {
        match self.garyx_db.max_task_projection_number() {
            Ok(number) => number,
            Err(error) => {
                warn!(error = %error, "failed to read task projection max number");
                None
            }
        }
    }

    async fn remove_thread(&self, thread_id: &str) {
        if let Err(error) = self.garyx_db.remove_task_projection(thread_id) {
            warn!(thread_id, error = %error, "failed to remove stale task projection row");
        }
    }
}

pub(crate) fn task_projection_draft_from_thread_data(
    thread_id: &str,
    data: &Value,
) -> Option<TaskProjectionDraft> {
    let thread_id = thread_id.trim();
    if !is_thread_key(thread_id) {
        return None;
    }
    let task = task_from_record(data).ok().flatten()?;
    task_projection_draft_from_task(thread_id, &task)
}

fn task_projection_draft_from_task(
    thread_id: &str,
    task: &ThreadTask,
) -> Option<TaskProjectionDraft> {
    let source = task.source.as_ref();
    let source_task_id = source.and_then(|source| normalized(source.task_id.as_deref()));
    let parent_task_number =
        source_task_id
            .as_deref()
            .and_then(|task_id| match TaskId::parse(task_id).ok()? {
                TaskId::Number(number) => Some(number),
                TaskId::ThreadId(_) => None,
            });
    let source_bot_id = source
        .and_then(|source| normalized(source.bot_id.as_deref()))
        .or_else(|| source.and_then(source_channel_account_id));
    let notification_thread_id = match task.notification_target.as_ref() {
        Some(TaskNotificationTarget::Thread { thread_id }) => normalized(Some(thread_id)),
        _ => None,
    };
    Some(TaskProjectionDraft {
        thread_id: thread_id.to_owned(),
        number: task.number,
        status: task.status.as_str().to_owned(),
        title: task.title.clone(),
        creator_json: canonical_json(&task.creator)?,
        creator_id: task.creator.id().to_owned(),
        assignee_json: optional_canonical_json(task.assignee.as_ref())?,
        assignee_id: task
            .assignee
            .as_ref()
            .map(Principal::id)
            .map(ToOwned::to_owned),
        updated_by_json: canonical_json(&task.updated_by)?,
        executor_json: optional_canonical_json(task.executor.as_ref())?,
        source_json: optional_canonical_json(task.source.as_ref())?,
        source_thread_id: source.and_then(|source| normalized(source.thread_id.as_deref())),
        source_task_thread_id: source
            .and_then(|source| normalized(source.task_thread_id.as_deref())),
        source_task_id,
        parent_task_number,
        source_bot_id,
        notification_thread_id,
        created_at: task.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        updated_at: task.updated_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        source_updated_at: task.updated_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        source_events_len: task.events.len(),
    })
}

pub(crate) async fn backfill_task_projection_if_incomplete(
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &GaryxDbService,
) -> usize {
    match garyx_db.task_projection_needs_backfill() {
        Ok(false) => return 0,
        Ok(true) => {}
        Err(error) => {
            warn!(error = %error, "failed to check task projection before backfill");
            return 0;
        }
    }

    let guard = garyx_db.lock_task_projection_backfill().await;
    match garyx_db.task_projection_needs_backfill() {
        Ok(false) => return 0,
        Ok(true) => {}
        Err(error) => {
            warn!(error = %error, "failed to recheck task projection before backfill");
            return 0;
        }
    }
    let active_backfill = match garyx_db.mark_task_projection_backfill_active() {
        Ok(active) => active,
        Err(error) => {
            warn!(error = %error, "failed to mark task projection backfill active");
            return 0;
        }
    };

    let thread_ids = thread_store.list_keys(Some("thread::")).await;
    let mut drafts = Vec::new();
    for thread_id in thread_ids {
        let Some(data) = thread_store.get(&thread_id).await else {
            continue;
        };
        if let Some(draft) = task_projection_draft_from_thread_data(&thread_id, &data) {
            drafts.push(draft);
        }
    }
    let count = drafts.len();
    if let Err(error) = garyx_db.sync_task_projection_snapshot(drafts) {
        warn!(error = %error, "failed to sync task projection snapshot");
        return 0;
    }
    if let Err(error) = garyx_db.record_projection_state(
        TASK_PROJECTION_NAME,
        crate::garyx_db::CURRENT_TASK_PROJECTION_VERSION,
        count,
    ) {
        warn!(error = %error, "failed to record task projection state");
        return 0;
    }
    drop(active_backfill);
    drop(guard);
    let reconciled = reconcile_task_projection(thread_store, garyx_db).await;
    debug!(
        task_projection_backfill_count = count,
        task_projection_reconcile_count = reconciled,
        "task projection backfill completed"
    );
    count
}

pub(crate) async fn reconcile_task_projection(
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &GaryxDbService,
) -> usize {
    let thread_ids = thread_store.list_keys(Some("thread::")).await;
    let mut projected_thread_ids = BTreeSet::new();
    let mut reconciled = 0usize;
    for thread_id in thread_ids {
        let Some(data) = thread_store.get(&thread_id).await else {
            continue;
        };
        if let Some(draft) = task_projection_draft_from_thread_data(&thread_id, &data) {
            projected_thread_ids.insert(thread_id.clone());
            if let Err(error) = garyx_db.replace_task_projection(draft) {
                warn!(thread_id, error = %error, "failed to reconcile task projection row");
            } else {
                reconciled += 1;
            }
        }
    }

    let existing_thread_ids = match garyx_db.list_task_projection_thread_ids() {
        Ok(thread_ids) => thread_ids,
        Err(error) => {
            warn!(error = %error, "failed to list task projection rows before reconcile prune");
            return reconciled;
        }
    };
    for thread_id in existing_thread_ids {
        if projected_thread_ids.contains(&thread_id) {
            continue;
        }
        let still_has_task = thread_store
            .get(&thread_id)
            .await
            .and_then(|data| task_projection_draft_from_thread_data(&thread_id, &data))
            .is_some();
        if still_has_task {
            continue;
        }
        match garyx_db.remove_task_projection(&thread_id) {
            Ok(true) => reconciled += 1,
            Ok(false) => {}
            Err(error) => {
                warn!(thread_id, error = %error, "failed to prune stale task projection row during reconcile");
            }
        }
    }
    reconciled
}

fn optional_canonical_json<T: Serialize>(value: Option<&T>) -> Option<Option<String>> {
    match value {
        Some(value) => Some(Some(canonical_json(value)?)),
        None => Some(None),
    }
}

fn canonical_json<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_string(value).ok()
}

fn normalized(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn source_channel_account_id(source: &TaskSource) -> Option<String> {
    let channel = normalized(source.channel.as_deref())?;
    let account_id = normalized(source.account_id.as_deref())?;
    Some(format!("{channel}:{account_id}"))
}
