use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use chrono::Utc;
use garyx_models::{
    Principal, TASK_SCHEMA_VERSION_V1, TaskEvent, TaskEventKind, TaskNotificationTarget,
    TaskSource, TaskStatus, ThreadTask,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{TaskCounterError, TaskCounterStore};
use crate::{
    ThreadEnsureOptions, ThreadStore, agent_id_from_value, create_thread_record,
    history_message_count, is_thread_key,
};

const DEFAULT_TASK_LIST_LIMIT: usize = 50;
const MAX_TASK_LIST_LIMIT: usize = 200;
const DEFAULT_TASK_AGENT_ID: &str = "claude";
type TaskThreadLock = Arc<tokio::sync::Mutex<()>>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TaskIndexKey {
    store_id: usize,
    number: u64,
}

#[derive(Default)]
struct TaskIndexState {
    bootstrapped_stores: HashSet<usize>,
    by_number: HashMap<TaskIndexKey, String>,
}

static TASK_THREAD_LOCKS: OnceLock<StdMutex<HashMap<String, TaskThreadLock>>> = OnceLock::new();
static TASK_INDEX: OnceLock<StdMutex<TaskIndexState>> = OnceLock::new();

#[derive(Debug, thiserror::Error)]
pub enum TaskServiceError {
    #[error("NotFound: {0}")]
    NotFound(String),
    #[error("NotATask: {0}")]
    NotATask(String),
    #[error("AlreadyATask: {0}")]
    AlreadyATask(String),
    #[error("InvalidTransition: {from:?} -> {to:?}")]
    InvalidTransition { from: TaskStatus, to: TaskStatus },
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("UnknownPrincipal: {0}")]
    UnknownPrincipal(String),
    #[error("UnknownAgent: {0}")]
    UnknownAgent(String),
    #[error("store error: {0}")]
    Store(String),
    #[error(transparent)]
    Counter(#[from] TaskCounterError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Principal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_target: Option<TaskNotificationTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TaskSource>,
    #[serde(default)]
    pub start: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Principal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<TaskRuntimeInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRuntimeInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteTaskInput {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Principal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_target: Option<TaskNotificationTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TaskSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Principal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTaskStatusInput {
    pub task_id: String,
    pub to: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Principal>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskListFilter {
    pub status: Option<TaskStatus>,
    pub assignee: Option<Principal>,
    pub creator: Option<Principal>,
    pub source_thread_id: Option<String>,
    pub source_task_id: Option<String>,
    pub source_bot_id: Option<String>,
    pub include_done: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub thread_id: String,
    pub task_id: String,
    pub number: u64,
    pub title: String,
    pub status: TaskStatus,
    pub creator: Principal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Principal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TaskSource>,
    pub updated_at: chrono::DateTime<Utc>,
    pub updated_by: Principal,
    pub runtime_agent_id: String,
    pub reply_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskHistoryPage {
    pub events: Vec<TaskEvent>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskId {
    ThreadId(String),
    Number(u64),
}

impl TaskId {
    pub fn parse(input: &str) -> Result<Self, TaskServiceError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(TaskServiceError::BadRequest("task_id is empty".to_owned()));
        }
        if is_thread_key(trimmed) {
            return Ok(Self::ThreadId(trimmed.to_owned()));
        }
        let rest = trimmed.strip_prefix('#').unwrap_or(trimmed);
        let rest = rest
            .strip_prefix("TASK-")
            .or_else(|| rest.strip_prefix("task-"))
            .unwrap_or(rest);
        if let Ok(number) = rest.parse::<u64>() {
            if number == 0 {
                return Err(TaskServiceError::BadRequest(
                    "task_id number must be greater than zero".to_owned(),
                ));
            }
            return Ok(Self::Number(number));
        }
        Err(TaskServiceError::BadRequest(format!(
            "task_id must be #TASK-<number> or a canonical thread id: {trimmed}"
        )))
    }
}

pub struct TaskService {
    thread_store: Arc<dyn ThreadStore>,
    counter_store: Arc<dyn TaskCounterStore>,
}

impl TaskService {
    pub fn new(
        thread_store: Arc<dyn ThreadStore>,
        counter_store: Arc<dyn TaskCounterStore>,
    ) -> Self {
        Self {
            thread_store,
            counter_store,
        }
    }

    fn index_store_id(&self) -> usize {
        thread_store_id(&self.thread_store)
    }

    pub async fn create_task(
        &self,
        input: CreateTaskInput,
    ) -> Result<(String, ThreadTask), TaskServiceError> {
        let actor = input.actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        if let Some(assignee) = &input.assignee {
            validate_principal(assignee)?;
        }
        validate_notification_target(input.notification_target.as_ref())?;
        let source = normalize_task_source(input.source);
        let runtime = input.runtime.clone();
        let auto_start = input.start || input.assignee.is_some();
        let thread_agent_id = runtime
            .as_ref()
            .and_then(|runtime| normalized_nonempty_string(runtime.agent_id.as_deref()))
            .or_else(|| normalized_nonempty_string(input.agent_id.as_deref()))
            .or_else(|| match &input.assignee {
                Some(Principal::Agent { agent_id }) => Some(agent_id.clone()),
                _ => None,
            })
            .or_else(|| match (&actor, auto_start) {
                (Principal::Agent { agent_id }, true) => Some(agent_id.clone()),
                (Principal::Human { .. }, true) => Some(DEFAULT_TASK_AGENT_ID.to_owned()),
                (_, false) => None,
            });
        let workspace_dir = runtime
            .as_ref()
            .and_then(|runtime| normalized_nonempty_string(runtime.workspace_dir.as_deref()))
            .or_else(|| normalized_nonempty_string(input.workspace_dir.as_deref()));

        let (thread_id, mut record) = create_thread_record(
            &self.thread_store,
            ThreadEnsureOptions {
                label: input.title.clone(),
                workspace_dir,
                agent_id: thread_agent_id,
                ..Default::default()
            },
        )
        .await
        .map_err(TaskServiceError::Store)?;

        let body = normalized_limited(input.body, 8_000)?;
        if let Some(body) = body.as_deref() {
            let message = json!({
                "role": "user",
                "content": body,
                "timestamp": Utc::now().to_rfc3339(),
            });
            let obj = record.as_object_mut().ok_or_else(|| {
                TaskServiceError::Store("thread record is not an object".to_owned())
            })?;
            obj.entry("messages".to_owned())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| TaskServiceError::Store("messages is not an array".to_owned()))?
                .push(message);
            obj.insert("message_count".to_owned(), Value::Number(1.into()));
        }

        let title = derive_title(input.title.as_deref(), &record);
        let task = self
            .build_task(
                title,
                if auto_start {
                    TaskStatus::InProgress
                } else {
                    TaskStatus::Todo
                },
                actor,
                input.assignee,
                input.notification_target,
                source,
                TaskEventKind::Created {
                    initial_status: if auto_start {
                        TaskStatus::InProgress
                    } else {
                        TaskStatus::Todo
                    },
                    assignee: None,
                },
            )
            .await?;

        let mut task = task;
        task.body = body;
        if let TaskEventKind::Created { assignee, .. } = &mut task.events[0].kind {
            *assignee = task.assignee.clone();
        }
        set_task_on_record(&mut record, &task)?;
        self.thread_store.set(&thread_id, record).await;
        task_index_upsert(self.index_store_id(), &thread_id, &task);
        Ok((thread_id, task))
    }

    pub async fn promote_task(
        &self,
        input: PromoteTaskInput,
    ) -> Result<ThreadTask, TaskServiceError> {
        let actor = input.actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        if let Some(assignee) = &input.assignee {
            validate_principal(assignee)?;
        }
        validate_notification_target(input.notification_target.as_ref())?;
        let source = normalize_task_source(input.source);
        let lock = task_thread_lock(&input.thread_id);
        let _guard = lock.lock().await;
        let mut record = self.load_record(&input.thread_id).await?;
        if task_from_record(&record)?.is_some() {
            return Err(TaskServiceError::AlreadyATask(input.thread_id));
        }
        let title = derive_title(input.title.as_deref(), &record);
        let auto_start = input.assignee.is_some();
        let task = self
            .build_task(
                title,
                if auto_start {
                    TaskStatus::InProgress
                } else {
                    TaskStatus::Todo
                },
                actor,
                input.assignee,
                input.notification_target,
                source,
                TaskEventKind::Promoted {
                    initial_status: if auto_start {
                        TaskStatus::InProgress
                    } else {
                        TaskStatus::Todo
                    },
                    assignee: None,
                },
            )
            .await?;
        let mut task = task;
        if let TaskEventKind::Promoted { assignee, .. } = &mut task.events[0].kind {
            *assignee = task.assignee.clone();
        }
        set_task_on_record(&mut record, &task)?;
        self.thread_store.set(&input.thread_id, record).await;
        task_index_upsert(self.index_store_id(), &input.thread_id, &task);
        Ok(task)
    }

    pub async fn get_task(
        &self,
        task_id: &str,
    ) -> Result<(String, Value, ThreadTask), TaskServiceError> {
        let (thread_id, record) = self.resolve_task_record(task_id).await?;
        let task = task_from_record(&record)?
            .ok_or_else(|| TaskServiceError::NotATask(thread_id.clone()))?;
        Ok((thread_id, record, task))
    }

    pub async fn list_tasks(
        &self,
        filter: TaskListFilter,
    ) -> Result<(Vec<TaskSummary>, usize, bool), TaskServiceError> {
        let limit = filter
            .limit
            .unwrap_or(DEFAULT_TASK_LIST_LIMIT)
            .clamp(1, MAX_TASK_LIST_LIMIT);
        let offset = filter.offset.unwrap_or(0);
        self.ensure_task_index().await?;
        let mut tasks = Vec::new();
        let mut stale_index_keys = Vec::new();
        let store_id = self.index_store_id();
        for (index_key, key) in task_index_entries(store_id) {
            let Some(record) = self.thread_store.get(&key).await else {
                stale_index_keys.push(index_key);
                continue;
            };
            let Some(task) = task_from_record(&record)? else {
                stale_index_keys.push(index_key);
                continue;
            };
            if task.number != index_key.number {
                stale_index_keys.push(index_key);
                continue;
            };
            if !filter.include_done && task.status == TaskStatus::Done {
                continue;
            }
            if filter.status.is_some_and(|status| task.status != status) {
                continue;
            }
            if filter
                .assignee
                .as_ref()
                .is_some_and(|candidate| task.assignee.as_ref() != Some(candidate))
            {
                continue;
            }
            if filter
                .creator
                .as_ref()
                .is_some_and(|creator| &task.creator != creator)
            {
                continue;
            }
            if filter
                .source_thread_id
                .as_deref()
                .is_some_and(|source_thread_id| {
                    !task_matches_source_thread(&task, source_thread_id)
                })
            {
                continue;
            }
            if filter
                .source_task_id
                .as_deref()
                .is_some_and(|source_task_id| !task_matches_source_task(&task, source_task_id))
            {
                continue;
            }
            if filter
                .source_bot_id
                .as_deref()
                .is_some_and(|source_bot_id| !task_matches_source_bot(&task, source_bot_id))
            {
                continue;
            }
            tasks.push(TaskSummary::from_task(key, &record, &task));
        }
        for index_key in stale_index_keys {
            task_index_remove(&index_key);
        }
        tasks.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.thread_id.cmp(&right.thread_id))
        });
        let total = tasks.len();
        let page = tasks
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let has_more = offset.saturating_add(page.len()) < total;
        Ok((page, total, has_more))
    }

    pub async fn task_history(
        &self,
        task_id: &str,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<TaskHistoryPage, TaskServiceError> {
        let (_, _, task) = self.get_task(task_id).await?;
        let limit = limit
            .unwrap_or(DEFAULT_TASK_LIST_LIMIT)
            .clamp(1, MAX_TASK_LIST_LIMIT);
        let mut events = task.events.into_iter().rev().collect::<Vec<_>>();
        if let Some(before) = before.map(str::trim).filter(|value| !value.is_empty()) {
            let Some(index) = events.iter().position(|event| event.event_id == before) else {
                return Err(TaskServiceError::NotFound(format!(
                    "task event not found: {before}"
                )));
            };
            events = events.into_iter().skip(index + 1).collect();
        }
        let has_more = events.len() > limit;
        events.truncate(limit);
        Ok(TaskHistoryPage { events, has_more })
    }

    pub async fn assign_task(
        &self,
        task_id: &str,
        to: Principal,
        actor: Option<Principal>,
    ) -> Result<ThreadTask, TaskServiceError> {
        validate_principal(&to)?;
        let actor = actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        self.mutate_task(task_id, move |task| {
            let previous = task.assignee.clone();
            task.assignee = Some(to.clone());
            let previous_status = task.status;
            push_event(
                task,
                actor.clone(),
                TaskEventKind::Assigned { from: previous, to },
                None,
            );
            if previous_status == TaskStatus::Todo {
                task.status = TaskStatus::InProgress;
                push_event(
                    task,
                    actor,
                    TaskEventKind::StatusChanged {
                        from: previous_status,
                        to: TaskStatus::InProgress,
                        note: Some("assigned".to_owned()),
                    },
                    None,
                );
            }
            Ok(())
        })
        .await
    }

    pub async fn unassign_task(
        &self,
        task_id: &str,
        actor: Option<Principal>,
    ) -> Result<ThreadTask, TaskServiceError> {
        let actor = actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        self.mutate_task(task_id, move |task| {
            let previous = task.assignee.clone().ok_or_else(|| {
                TaskServiceError::BadRequest("task is already unassigned".to_owned())
            })?;
            task.assignee = None;
            push_event(
                task,
                actor,
                TaskEventKind::Unassigned { from: previous },
                None,
            );
            Ok(())
        })
        .await
    }

    pub async fn update_status(
        &self,
        input: UpdateTaskStatusInput,
    ) -> Result<ThreadTask, TaskServiceError> {
        let actor = input.actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        self.mutate_task(&input.task_id, move |task| {
            let from = task.status;
            if from == input.to {
                return Ok(());
            }
            if !input.force && !is_allowed_transition(from, input.to) {
                return Err(TaskServiceError::InvalidTransition { from, to: input.to });
            }
            let event_kind = match (from, input.to, input.force) {
                (TaskStatus::Done, TaskStatus::Todo, false) => TaskEventKind::Reopened { from },
                _ => TaskEventKind::StatusChanged {
                    from,
                    to: input.to,
                    note: if input.force {
                        Some(
                            input
                                .note
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(ToOwned::to_owned)
                                .unwrap_or_else(|| "forced".to_owned()),
                        )
                    } else {
                        normalized_limited(input.note, 500)?
                    },
                },
            };
            task.status = input.to;
            push_event(task, actor, event_kind, None);
            Ok(())
        })
        .await
    }

    pub async fn set_title(
        &self,
        task_id: &str,
        title: String,
        actor: Option<Principal>,
    ) -> Result<ThreadTask, TaskServiceError> {
        let actor = actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        let next_title = normalized_limited(Some(title), 200)?
            .ok_or_else(|| TaskServiceError::BadRequest("title cannot be empty".to_owned()))?;
        self.mutate_task(task_id, move |task| {
            let previous = task.title.clone();
            if previous == next_title {
                return Ok(());
            }
            task.title = next_title.clone();
            push_event(
                task,
                actor,
                TaskEventKind::TitleChanged {
                    from: previous,
                    to: next_title,
                },
                None,
            );
            Ok(())
        })
        .await
    }

    async fn mutate_task<F>(&self, task_id: &str, f: F) -> Result<ThreadTask, TaskServiceError>
    where
        F: FnOnce(&mut ThreadTask) -> Result<(), TaskServiceError>,
    {
        let (thread_id, _) = self.resolve_task_record(task_id).await?;
        let lock = task_thread_lock(&thread_id);
        let _guard = lock.lock().await;
        let mut record = self
            .thread_store
            .get(&thread_id)
            .await
            .ok_or_else(|| TaskServiceError::NotFound(thread_id.clone()))?;
        let mut task = task_from_record(&record)?
            .ok_or_else(|| TaskServiceError::NotATask(thread_id.clone()))?;
        f(&mut task)?;
        set_task_on_record(&mut record, &task)?;
        self.thread_store.set(&thread_id, record).await;
        task_index_upsert(self.index_store_id(), &thread_id, &task);
        Ok(task)
    }

    async fn ensure_task_index(&self) -> Result<(), TaskServiceError> {
        let store_id = self.index_store_id();
        if task_index_is_bootstrapped(store_id) {
            return Ok(());
        }

        let mut rebuilt = HashMap::new();
        for key in self.thread_store.list_keys(None).await {
            if !is_thread_key(&key) {
                continue;
            }
            let Some(record) = self.thread_store.get(&key).await else {
                continue;
            };
            let Some(task) = task_from_record(&record)? else {
                continue;
            };
            rebuilt.insert(task_index_key(store_id, &task), key);
        }
        task_index_bootstrap(store_id, rebuilt);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_task(
        &self,
        title: String,
        status: TaskStatus,
        actor: Principal,
        assignee: Option<Principal>,
        notification_target: Option<TaskNotificationTarget>,
        source: Option<TaskSource>,
        event_kind: TaskEventKind,
    ) -> Result<ThreadTask, TaskServiceError> {
        self.ensure_task_index().await?;
        let store_id = self.index_store_id();
        let mut number = self.counter_store.allocate().await?;
        while task_index_lookup(&TaskIndexKey { store_id, number }).is_some()
            || number <= task_index_max_number(store_id)
        {
            number = self.counter_store.allocate().await?;
        }
        let now = Utc::now();
        let mut task = ThreadTask {
            schema_version: TASK_SCHEMA_VERSION_V1,
            number,
            title,
            status,
            creator: actor.clone(),
            assignee,
            notification_target,
            source,
            body: None,
            created_at: now,
            updated_at: now,
            updated_by: actor.clone(),
            events: Vec::new(),
        };
        push_event(&mut task, actor, event_kind, Some(now));
        Ok(task)
    }

    async fn load_record(&self, thread_id: &str) -> Result<Value, TaskServiceError> {
        if !is_thread_key(thread_id) {
            return Err(TaskServiceError::BadRequest(format!(
                "invalid thread id: {thread_id}"
            )));
        }
        let record = self
            .thread_store
            .get(thread_id)
            .await
            .ok_or_else(|| TaskServiceError::NotFound(thread_id.to_owned()))?;
        Ok(record)
    }

    async fn resolve_task_record(
        &self,
        task_id: &str,
    ) -> Result<(String, Value), TaskServiceError> {
        match TaskId::parse(task_id)? {
            TaskId::ThreadId(thread_id) => {
                let record = self
                    .thread_store
                    .get(&thread_id)
                    .await
                    .ok_or_else(|| TaskServiceError::NotFound(thread_id.clone()))?;
                Ok((thread_id, record))
            }
            TaskId::Number(number) => self.find_task_by_number(number).await,
        }
    }

    async fn find_task_by_number(&self, number: u64) -> Result<(String, Value), TaskServiceError> {
        self.ensure_task_index().await?;
        let index_key = TaskIndexKey {
            store_id: self.index_store_id(),
            number,
        };
        if let Some(thread_id) = task_index_lookup(&index_key) {
            if let Some(record) = self.thread_store.get(&thread_id).await
                && let Some(task) = task_from_record(&record)?
                && task.number == number
            {
                return Ok((thread_id, record));
            }
            task_index_remove(&index_key);
        }
        Err(TaskServiceError::NotFound(format!("#TASK-{number}")))
    }
}

pub async fn mark_thread_task_in_review_if_in_progress(
    thread_store: &Arc<dyn ThreadStore>,
    thread_id: &str,
    actor: Principal,
    note: Option<String>,
) -> Result<Option<ThreadTask>, TaskServiceError> {
    validate_principal(&actor)?;
    let lock = task_thread_lock(thread_id);
    let _guard = lock.lock().await;
    let Some(mut record) = thread_store.get(thread_id).await else {
        return Ok(None);
    };
    let Some(mut task) = task_from_record(&record)? else {
        return Ok(None);
    };
    if task.status != TaskStatus::InProgress {
        return Ok(None);
    }
    let from = task.status;
    task.status = TaskStatus::InReview;
    push_event(
        &mut task,
        actor,
        TaskEventKind::StatusChanged {
            from,
            to: TaskStatus::InReview,
            note: normalized_limited(note, 500)?,
        },
        None,
    );
    set_task_on_record(&mut record, &task)?;
    thread_store.set(thread_id, record).await;
    task_index_upsert(thread_store_id(thread_store), thread_id, &task);
    Ok(Some(task))
}

impl TaskSummary {
    fn from_task(thread_id: String, record: &Value, task: &ThreadTask) -> Self {
        Self {
            thread_id,
            task_id: canonical_task_id(task),
            number: task.number,
            title: task.title.clone(),
            status: task.status,
            creator: task.creator.clone(),
            assignee: task.assignee.clone(),
            source: task.source.clone(),
            updated_at: task.updated_at,
            updated_by: task.updated_by.clone(),
            runtime_agent_id: agent_id_from_value(record).unwrap_or_default(),
            reply_count: u32::try_from(history_message_count(record)).unwrap_or(u32::MAX),
        }
    }
}

pub fn canonical_task_id(task: &ThreadTask) -> String {
    format!("#TASK-{}", task.number)
}

pub fn task_from_record(record: &Value) -> Result<Option<ThreadTask>, TaskServiceError> {
    match record.get("task") {
        Some(Value::Null) | None => Ok(None),
        Some(value) => Ok(Some(serde_json::from_value(value.clone())?)),
    }
}

fn set_task_on_record(record: &mut Value, task: &ThreadTask) -> Result<(), TaskServiceError> {
    let obj = record
        .as_object_mut()
        .ok_or_else(|| TaskServiceError::Store("thread record is not an object".to_owned()))?;
    obj.insert("task".to_owned(), serde_json::to_value(task)?);
    obj.insert(
        "updated_at".to_owned(),
        Value::String(task.updated_at.to_rfc3339()),
    );
    Ok(())
}

fn task_thread_lock(thread_id: &str) -> TaskThreadLock {
    let locks = TASK_THREAD_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(thread_id.to_owned())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

fn task_index_state() -> &'static StdMutex<TaskIndexState> {
    TASK_INDEX.get_or_init(|| StdMutex::new(TaskIndexState::default()))
}

fn thread_store_id(thread_store: &Arc<dyn ThreadStore>) -> usize {
    Arc::as_ptr(thread_store) as *const () as usize
}

fn task_index_key(store_id: usize, task: &ThreadTask) -> TaskIndexKey {
    TaskIndexKey {
        store_id,
        number: task.number,
    }
}

fn task_index_is_bootstrapped(store_id: usize) -> bool {
    let state = task_index_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.bootstrapped_stores.contains(&store_id)
}

fn task_index_bootstrap(store_id: usize, rebuilt: HashMap<TaskIndexKey, String>) {
    let mut state = task_index_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.bootstrapped_stores.contains(&store_id) {
        return;
    }
    state
        .by_number
        .retain(|index_key, _| index_key.store_id != store_id);
    state.by_number.extend(rebuilt);
    state.bootstrapped_stores.insert(store_id);
}

fn task_index_upsert(store_id: usize, thread_id: &str, task: &ThreadTask) {
    let mut state = task_index_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state
        .by_number
        .insert(task_index_key(store_id, task), thread_id.to_owned());
}

fn task_index_lookup(index_key: &TaskIndexKey) -> Option<String> {
    let state = task_index_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.by_number.get(index_key).cloned()
}

fn task_index_entries(store_id: usize) -> Vec<(TaskIndexKey, String)> {
    let state = task_index_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut seen = HashSet::new();
    state
        .by_number
        .iter()
        .filter(|(key, _)| key.store_id == store_id)
        .filter_map(|(key, thread_id)| {
            if seen.insert(thread_id.clone()) {
                Some((key.clone(), thread_id.clone()))
            } else {
                None
            }
        })
        .collect()
}

fn task_index_max_number(store_id: usize) -> u64 {
    let state = task_index_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state
        .by_number
        .keys()
        .filter(|key| key.store_id == store_id)
        .map(|key| key.number)
        .max()
        .unwrap_or(0)
}

fn task_index_remove(index_key: &TaskIndexKey) {
    let mut state = task_index_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.by_number.remove(index_key);
}

fn push_event(
    task: &mut ThreadTask,
    actor: Principal,
    kind: TaskEventKind,
    at: Option<chrono::DateTime<Utc>>,
) {
    let at = at.unwrap_or_else(Utc::now);
    task.updated_at = at;
    task.updated_by = actor.clone();
    task.events.push(TaskEvent {
        event_id: Uuid::now_v7().to_string(),
        at,
        actor,
        kind,
    });
}

fn derive_title(input: Option<&str>, record: &Value) -> String {
    if let Some(title) = input
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(truncate_title)
    {
        return title;
    }
    let messages = record
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        if role != "user" {
            continue;
        }
        if let Some(text) = message_text(&message) {
            return truncate_title(&text);
        }
    }
    "Untitled task".to_owned()
}

fn message_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
    .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
    .filter(|text| !text.is_empty())
}

fn truncate_title(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let truncated: String = chars.by_ref().take(80).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else if truncated.is_empty() {
        "Untitled task".to_owned()
    } else {
        truncated
    }
}

fn normalized_limited(
    value: Option<String>,
    limit: usize,
) -> Result<Option<String>, TaskServiceError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > limit {
        return Err(TaskServiceError::BadRequest(format!(
            "value exceeds {limit} characters"
        )));
    }
    Ok(Some(trimmed.to_owned()))
}

fn normalized_nonempty_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_task_source(source: Option<TaskSource>) -> Option<TaskSource> {
    let mut source = source?;
    source.thread_id = normalized_nonempty_string(source.thread_id.as_deref());
    source.task_id = normalized_nonempty_string(source.task_id.as_deref());
    source.task_thread_id =
        normalized_nonempty_string(source.task_thread_id.as_deref()).or_else(|| {
            source
                .task_id
                .is_some()
                .then(|| source.thread_id.clone())
                .flatten()
        });
    source.bot_id = normalized_nonempty_string(source.bot_id.as_deref());
    source.channel = normalized_nonempty_string(source.channel.as_deref());
    source.account_id = normalized_nonempty_string(source.account_id.as_deref());
    if source.bot_id.is_none()
        && let (Some(channel), Some(account_id)) = (&source.channel, &source.account_id)
    {
        source.bot_id = Some(format!("{channel}:{account_id}"));
    }
    (source.thread_id.is_some()
        || source.task_id.is_some()
        || source.task_thread_id.is_some()
        || source.bot_id.is_some()
        || source.channel.is_some()
        || source.account_id.is_some())
    .then_some(source)
}

fn task_matches_source_thread(task: &ThreadTask, source_thread_id: &str) -> bool {
    let Some(source_thread_id) = normalized_nonempty_string(Some(source_thread_id)) else {
        return true;
    };
    let Some(source) = task.source.as_ref() else {
        return false;
    };
    source.thread_id.as_deref() == Some(source_thread_id.as_str())
        || source.task_thread_id.as_deref() == Some(source_thread_id.as_str())
}

fn task_matches_source_task(task: &ThreadTask, source_task_id: &str) -> bool {
    let Some(source_task_id) = normalized_nonempty_string(Some(source_task_id)) else {
        return true;
    };
    task.source
        .as_ref()
        .and_then(|source| source.task_id.as_deref())
        .is_some_and(|task_id| task_id.eq_ignore_ascii_case(&source_task_id))
}

fn task_matches_source_bot(task: &ThreadTask, source_bot_id: &str) -> bool {
    let Some(source_bot_id) = normalized_nonempty_string(Some(source_bot_id)) else {
        return true;
    };
    let Some(source) = task.source.as_ref() else {
        return false;
    };
    if source.bot_id.as_deref() == Some(source_bot_id.as_str()) {
        return true;
    }
    match (&source.channel, &source.account_id) {
        (Some(channel), Some(account_id)) => format!("{channel}:{account_id}") == source_bot_id,
        _ => false,
    }
}

fn validate_principal(principal: &Principal) -> Result<(), TaskServiceError> {
    let id = principal.id().trim();
    if id.is_empty()
        || !id.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == ':' || ch == '.'
        })
    {
        return Err(TaskServiceError::UnknownPrincipal(id.to_owned()));
    }
    Ok(())
}

fn validate_notification_target(
    target: Option<&TaskNotificationTarget>,
) -> Result<(), TaskServiceError> {
    let Some(target) = target else {
        return Ok(());
    };
    match target {
        TaskNotificationTarget::None => Ok(()),
        TaskNotificationTarget::Thread { thread_id } => {
            let trimmed = thread_id.trim();
            if trimmed.is_empty() || !is_thread_key(trimmed) {
                return Err(TaskServiceError::BadRequest(
                    "notification thread target must be a canonical thread id".to_owned(),
                ));
            }
            Ok(())
        }
        TaskNotificationTarget::Bot {
            channel,
            account_id,
        } => {
            if channel.trim().is_empty() || account_id.trim().is_empty() {
                return Err(TaskServiceError::BadRequest(
                    "notification bot target requires channel and account_id".to_owned(),
                ));
            }
            Ok(())
        }
    }
}

fn default_actor() -> Principal {
    Principal::Human {
        user_id: "owner".to_owned(),
    }
}

fn is_allowed_transition(from: TaskStatus, to: TaskStatus) -> bool {
    matches!(
        (from, to),
        (TaskStatus::Todo, TaskStatus::InProgress)
            | (TaskStatus::InProgress, TaskStatus::Todo)
            | (TaskStatus::InProgress, TaskStatus::InReview)
            | (TaskStatus::InReview, TaskStatus::InProgress)
            | (TaskStatus::InReview, TaskStatus::Done)
            | (TaskStatus::Done, TaskStatus::Todo)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InMemoryTaskCounterStore, InMemoryThreadStore};

    fn service() -> TaskService {
        TaskService::new(
            Arc::new(InMemoryThreadStore::new()),
            Arc::new(InMemoryTaskCounterStore::new()),
        )
    }

    #[tokio::test]
    async fn task_create_stores_task_overlay_without_task_messages() {
        let service = service();
        let (thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Audit daemons".to_owned()),
                body: Some("Look at launchctl".to_owned()),
                assignee: None,
                notification_target: None,
                source: None,
                start: false,
                actor: Some(Principal::Agent {
                    agent_id: "cindy".to_owned(),
                }),
                agent_id: Some("cindy".to_owned()),
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        assert!(task.number > 0);
        let record = service.thread_store.get(&thread_id).await.unwrap();
        assert!(record.get("task").is_some());
        let messages = record.get("messages").and_then(Value::as_array).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], Value::String("user".to_owned()));
    }

    #[tokio::test]
    async fn task_create_stores_source_and_list_filters_it() {
        let service = service();
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Child task".to_owned()),
                body: None,
                assignee: None,
                notification_target: None,
                source: Some(TaskSource {
                    thread_id: Some("thread::origin".to_owned()),
                    task_id: Some("#TASK-7".to_owned()),
                    task_thread_id: Some("thread::origin".to_owned()),
                    bot_id: Some("telegram:main".to_owned()),
                    channel: Some("telegram".to_owned()),
                    account_id: Some("main".to_owned()),
                }),
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        assert_eq!(
            task.source
                .as_ref()
                .and_then(|source| source.task_id.as_deref()),
            Some("#TASK-7")
        );

        let (filtered, total, has_more) = service
            .list_tasks(TaskListFilter {
                source_thread_id: Some("thread::origin".to_owned()),
                source_task_id: Some("#TASK-7".to_owned()),
                source_bot_id: Some("telegram:main".to_owned()),
                include_done: true,
                limit: None,
                offset: None,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert!(!has_more);
        assert_eq!(filtered[0].task_id, canonical_task_id(&task));
        assert_eq!(
            filtered[0]
                .source
                .as_ref()
                .and_then(|source| source.bot_id.as_deref()),
            Some("telegram:main")
        );

        let (filtered, total, _) = service
            .list_tasks(TaskListFilter {
                source_bot_id: Some("telegram:other".to_owned()),
                include_done: true,
                limit: None,
                offset: None,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn status_machine_rejects_illegal_transition() {
        let service = service();
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Review".to_owned()),
                body: None,
                assignee: None,
                notification_target: None,
                source: None,
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        let error = service
            .update_status(UpdateTaskStatusInput {
                task_id: canonical_task_id(&task),
                to: TaskStatus::Done,
                note: None,
                force: false,
                actor: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, TaskServiceError::InvalidTransition { .. }));
    }

    #[tokio::test]
    async fn status_update_does_not_assign_todo_task() {
        let service = service();
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Claim me".to_owned()),
                body: None,
                assignee: None,
                notification_target: None,
                source: None,
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        let updated = service
            .update_status(UpdateTaskStatusInput {
                task_id: canonical_task_id(&task),
                to: TaskStatus::InProgress,
                note: None,
                force: false,
                actor: Some(Principal::Agent {
                    agent_id: "cindy".to_owned(),
                }),
            })
            .await
            .unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.assignee, None);
    }

    #[tokio::test]
    async fn run_completion_marks_in_progress_task_in_review() {
        let service = service();
        let (thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Review when idle".to_owned()),
                body: None,
                assignee: Some(Principal::Agent {
                    agent_id: "codex".to_owned(),
                }),
                notification_target: None,
                source: None,
                start: true,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();

        let updated = mark_thread_task_in_review_if_in_progress(
            &service.thread_store,
            &thread_id,
            Principal::Agent {
                agent_id: "garyx".to_owned(),
            },
            Some("agent run completed".to_owned()),
        )
        .await
        .unwrap()
        .expect("in-progress task should move to review");

        assert_eq!(updated.status, TaskStatus::InReview);
        let (_, _, persisted) = service.get_task(&canonical_task_id(&task)).await.unwrap();
        assert_eq!(persisted.status, TaskStatus::InReview);
        assert!(matches!(
            persisted.events.last().map(|event| &event.kind),
            Some(TaskEventKind::StatusChanged {
                from: TaskStatus::InProgress,
                to: TaskStatus::InReview,
                note: Some(note),
            }) if note == "agent run completed"
        ));
    }

    #[tokio::test]
    async fn run_completion_leaves_non_progress_task_status_unchanged() {
        let service = service();
        let (thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Already reviewed".to_owned()),
                body: None,
                assignee: Some(Principal::Agent {
                    agent_id: "codex".to_owned(),
                }),
                notification_target: None,
                source: None,
                start: true,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        service
            .update_status(UpdateTaskStatusInput {
                task_id: canonical_task_id(&task),
                to: TaskStatus::InReview,
                note: None,
                force: false,
                actor: None,
            })
            .await
            .unwrap();

        let updated = mark_thread_task_in_review_if_in_progress(
            &service.thread_store,
            &thread_id,
            Principal::Agent {
                agent_id: "garyx".to_owned(),
            },
            Some("agent run completed".to_owned()),
        )
        .await
        .unwrap();

        assert!(updated.is_none());
        let (_, _, persisted) = service.get_task(&canonical_task_id(&task)).await.unwrap();
        assert_eq!(persisted.status, TaskStatus::InReview);
    }

    #[tokio::test]
    async fn assignee_can_mark_done_after_explicit_review_confirmation() {
        let service = service();
        let assignee = Principal::Agent {
            agent_id: "codex".to_owned(),
        };
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Review gate".to_owned()),
                body: None,
                assignee: Some(assignee.clone()),
                notification_target: None,
                source: None,
                start: true,
                actor: Some(Principal::Human {
                    user_id: "owner".to_owned(),
                }),
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        let task_id = canonical_task_id(&task);

        service
            .update_status(UpdateTaskStatusInput {
                task_id: task_id.clone(),
                to: TaskStatus::InReview,
                note: None,
                force: false,
                actor: Some(assignee.clone()),
            })
            .await
            .unwrap();

        let updated = service
            .update_status(UpdateTaskStatusInput {
                task_id,
                to: TaskStatus::Done,
                note: Some("review approved by owner".to_owned()),
                force: false,
                actor: Some(assignee),
            })
            .await
            .unwrap();

        assert_eq!(updated.status, TaskStatus::Done);
    }

    #[tokio::test]
    async fn reviewer_can_mark_reviewed_task_done() {
        let service = service();
        let assignee = Principal::Agent {
            agent_id: "codex".to_owned(),
        };
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Review pass".to_owned()),
                body: None,
                assignee: Some(assignee.clone()),
                notification_target: None,
                source: None,
                start: true,
                actor: Some(Principal::Human {
                    user_id: "owner".to_owned(),
                }),
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        let task_id = canonical_task_id(&task);

        service
            .update_status(UpdateTaskStatusInput {
                task_id: task_id.clone(),
                to: TaskStatus::InReview,
                note: None,
                force: false,
                actor: Some(assignee),
            })
            .await
            .unwrap();

        let updated = service
            .update_status(UpdateTaskStatusInput {
                task_id,
                to: TaskStatus::Done,
                note: None,
                force: false,
                actor: Some(Principal::Human {
                    user_id: "owner".to_owned(),
                }),
            })
            .await
            .unwrap();

        assert_eq!(updated.status, TaskStatus::Done);
    }

    #[tokio::test]
    async fn assign_starts_todo_task() {
        let service = service();
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Assign me".to_owned()),
                body: None,
                assignee: None,
                notification_target: None,
                source: None,
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        let assignee = Principal::Agent {
            agent_id: "cindy".to_owned(),
        };
        let updated = service
            .assign_task(&canonical_task_id(&task), assignee.clone(), Some(assignee))
            .await
            .unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(
            updated.assignee,
            Some(Principal::Agent {
                agent_id: "cindy".to_owned()
            })
        );
        assert_eq!(updated.events.len(), 3);
    }

    #[tokio::test]
    async fn concurrent_mutations_preserve_both_events() {
        let service = Arc::new(service());
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("Concurrent".to_owned()),
                body: None,
                assignee: None,
                notification_target: None,
                source: None,
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        let task_id = canonical_task_id(&task);

        let left_service = service.clone();
        let left_id = task_id.clone();
        let left = tokio::spawn(async move {
            left_service
                .assign_task(
                    &left_id,
                    Principal::Agent {
                        agent_id: "cindy".to_owned(),
                    },
                    None,
                )
                .await
                .unwrap();
        });
        let right_service = service.clone();
        let right = tokio::spawn(async move {
            right_service
                .set_title(&task_id, "Retitled".to_owned(), None)
                .await
                .unwrap();
        });
        left.await.unwrap();
        right.await.unwrap();

        let (_, _, task) = service.get_task(&canonical_task_id(&task)).await.unwrap();
        assert_eq!(task.events.len(), 4);
        assert_eq!(task.title, "Retitled");
        assert_eq!(
            task.assignee,
            Some(Principal::Agent {
                agent_id: "cindy".to_owned()
            })
        );
    }

    #[tokio::test]
    async fn task_history_supports_before_cursor() {
        let service = service();
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                title: Some("History".to_owned()),
                body: None,
                assignee: None,
                notification_target: None,
                source: None,
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .unwrap();
        let task_id = canonical_task_id(&task);
        service
            .assign_task(
                &task_id,
                Principal::Agent {
                    agent_id: "cindy".to_owned(),
                },
                None,
            )
            .await
            .unwrap();
        service
            .set_title(&task_id, "History updated".to_owned(), None)
            .await
            .unwrap();

        let first_page = service.task_history(&task_id, Some(1), None).await.unwrap();
        assert_eq!(first_page.events.len(), 1);
        assert!(first_page.has_more);
        let second_page = service
            .task_history(&task_id, Some(10), Some(&first_page.events[0].event_id))
            .await
            .unwrap();
        assert_eq!(second_page.events.len(), 3);
        assert!(!second_page.has_more);
    }

    #[tokio::test]
    async fn task_create_persists_runtime_fields() {
        let service = service();
        let (thread_id, _task) = service
            .create_task(CreateTaskInput {
                title: Some("Runtime".to_owned()),
                body: None,
                assignee: None,
                notification_target: None,
                source: None,
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: Some(TaskRuntimeInput {
                    agent_id: Some("codex".to_owned()),
                    workspace_dir: Some("/tmp/garyx-task".to_owned()),
                }),
            })
            .await
            .unwrap();
        let record = service.thread_store.get(&thread_id).await.unwrap();
        assert_eq!(record["agent_id"], Value::String("codex".to_owned()));
        assert_eq!(
            record["workspace_dir"],
            Value::String("/tmp/garyx-task".to_owned())
        );
    }
}
