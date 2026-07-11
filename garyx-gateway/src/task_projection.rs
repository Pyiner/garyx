use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::SecondsFormat;
use garyx_models::{Principal, TaskNotificationTarget, TaskSource, ThreadTask};
use garyx_router::tasks::{
    TaskId, TaskListFilter, TaskProjectionReader, TaskSummary, task_from_record,
};
use garyx_router::{TaskCounterError, TaskCounterStore, is_thread_key};
use serde::Serialize;
use serde_json::Value;
use tracing::warn;

use crate::garyx_db::{GaryxDbService, TaskProjectionDraft};

pub(crate) struct SqlTaskProjectionReader {
    garyx_db: Arc<GaryxDbService>,
}

impl SqlTaskProjectionReader {
    pub(crate) fn new(garyx_db: Arc<GaryxDbService>) -> Self {
        Self { garyx_db }
    }
}

/// Projections derive in the same transaction as every record write
/// (#TASK-1864): the table is structurally current by construction, so
/// this reader only translates queries — backend failures surface as
/// errors instead of degrading into empty results.
#[async_trait]
impl TaskProjectionReader for SqlTaskProjectionReader {
    async fn thread_id_for_number(&self, number: u64) -> Result<Option<String>, String> {
        self.garyx_db
            .run_blocking(move |db| db.thread_id_for_number(number))
            .await
            .map_err(|error| error.to_string())
    }

    async fn has_running_subtask_targeting(&self, thread_id: &str) -> Result<bool, String> {
        let owned_thread_id = thread_id.to_owned();
        self.garyx_db
            .run_blocking(move |db| db.has_running_subtask_targeting(&owned_thread_id))
            .await
            .map_err(|error| error.to_string())
    }

    async fn list_task_summaries(
        &self,
        filter: &TaskListFilter,
    ) -> Result<(Vec<TaskSummary>, usize, bool), String> {
        let filter = filter.clone();
        self.garyx_db
            .run_blocking(move |db| db.list_task_summaries(&filter))
            .await
            .map_err(|error| error.to_string())
    }
}

/// SQLite-owned task-number allocation (#TASK-2099): one transactional
/// counter bump, floored against the task projection's `MAX(number)`.
pub(crate) struct SqliteTaskCounterStore {
    garyx_db: Arc<GaryxDbService>,
}

impl SqliteTaskCounterStore {
    pub(crate) fn new(garyx_db: Arc<GaryxDbService>) -> Self {
        Self { garyx_db }
    }
}

#[async_trait]
impl TaskCounterStore for SqliteTaskCounterStore {
    async fn allocate(&self) -> Result<u64, TaskCounterError> {
        self.garyx_db
            .run_blocking(|db| db.allocate_task_number())
            .await
            .map_err(|error| TaskCounterError::Backend(error.to_string()))
    }
}

/// One-shot migration of the retired file-based task counter
/// (`<data_dir>/task-counters/global.txt` held the next number to hand
/// out). Seeds the SQLite counter row when it does not exist yet; the
/// seed also floors against every task number embedded in thread record
/// bodies, covering archived threads whose projections were removed.
pub fn seed_task_counter_from_legacy(garyx_db: &Arc<GaryxDbService>, data_dir: &Path) {
    let file_floor = std::fs::read_to_string(data_dir.join("task-counters/global.txt"))
        .ok()
        .and_then(|contents| contents.trim().parse::<u64>().ok())
        .map(|next| next.saturating_sub(1))
        .unwrap_or(0);
    match garyx_db.seed_task_counter_if_missing(file_floor) {
        Ok(true) => {
            tracing::info!(file_floor, "seeded sqlite task counter from legacy state");
        }
        Ok(false) => {}
        Err(error) => {
            warn!(error = %error, "failed to seed sqlite task counter");
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
