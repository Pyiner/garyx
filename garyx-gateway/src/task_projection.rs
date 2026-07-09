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

pub(crate) fn register_gateway_task_projection_reader(
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &Arc<GaryxDbService>,
) -> Arc<dyn TaskProjectionReader> {
    let reader: Arc<dyn TaskProjectionReader> =
        Arc::new(SqlTaskProjectionReader::new(garyx_db.clone()));
    register_task_projection_reader(thread_store, reader.clone());
    reader
}

#[async_trait]
impl TaskProjectionReader for SqlTaskProjectionReader {
    async fn is_current(&self) -> bool {
        // Projections derive in the same transaction as every record write
        // (#TASK-1864): the table is structurally current by construction.
        true
    }

    async fn ensure_current(&self) -> bool {
        true
    }

    async fn task_index_rows(&self) -> Vec<(u64, String)> {
        match self
            .garyx_db
            .run_blocking(|db| db.task_index_rows())
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                warn!(error = %error, "failed to read task projection index rows");
                Vec::new()
            }
        }
    }

    async fn thread_id_for_number(&self, number: u64) -> Option<String> {
        match self
            .garyx_db
            .run_blocking(move |db| db.thread_id_for_number(number))
            .await
        {
            Ok(thread_id) => thread_id,
            Err(error) => {
                warn!(number, error = %error, "failed to read task projection number lookup");
                None
            }
        }
    }

    async fn has_running_subtask_targeting(&self, thread_id: &str) -> bool {
        let owned_thread_id = thread_id.to_owned();
        match self
            .garyx_db
            .run_blocking(move |db| db.has_running_subtask_targeting(&owned_thread_id))
            .await
        {
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
        let filter = filter.clone();
        match self
            .garyx_db
            .run_blocking(move |db| db.list_task_summaries(&filter))
            .await
        {
            Ok(page) => Some(page),
            Err(error) => {
                warn!(error = %error, "failed to list task projection summaries");
                None
            }
        }
    }

    async fn max_number(&self) -> Option<u64> {
        match self
            .garyx_db
            .run_blocking(|db| db.max_task_projection_number())
            .await
        {
            Ok(number) => number,
            Err(error) => {
                warn!(error = %error, "failed to read task projection max number");
                None
            }
        }
    }

    async fn remove_thread(&self, thread_id: &str) {
        let owned_thread_id = thread_id.to_owned();
        if let Err(error) = self
            .garyx_db
            .run_blocking(move |db| db.remove_task_projection(&owned_thread_id).map(|_| ()))
            .await
        {
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

