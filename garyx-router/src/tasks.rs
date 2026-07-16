use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use async_trait::async_trait;
use chrono::Utc;
use garyx_models::{
    AgentBindingError, Principal, ResolvedAgentBinding, SERVER_OWNED_AGENT_METADATA_KEYS,
    TASK_SCHEMA_VERSION_V1, TaskEvent, TaskEventKind, TaskExecutor, TaskNotificationTarget,
    TaskSource, TaskStatus, ThreadTask,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{TaskCounterError, TaskCounterStore};
use crate::{ThreadEnsureOptions, ThreadStore, WorkspaceMode, create_thread_record, is_thread_key};

const DEFAULT_TASK_LIST_LIMIT: usize = 50;
const MAX_TASK_LIST_LIMIT: usize = 200;
const TASK_THREAD_TITLE_SOURCE: &str = "task";
type TaskThreadLock = Arc<tokio::sync::Mutex<()>>;

static TASK_THREAD_LOCKS: OnceLock<StdMutex<HashMap<String, TaskThreadLock>>> = OnceLock::new();

#[derive(Debug, thiserror::Error)]
pub enum TaskServiceError {
    #[error("NotFound: {0}")]
    NotFound(String),
    #[error("NotATask: {0}")]
    NotATask(String),
    #[error("InvalidTransition: {from:?} -> {to:?}")]
    InvalidTransition { from: TaskStatus, to: TaskStatus },
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("UnknownPrincipal: {0}")]
    UnknownPrincipal(String),
    #[error("UnknownAgent: {0}")]
    UnknownAgent(String),
    #[error(transparent)]
    AgentBinding(#[from] AgentBindingError),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<TaskExecutor>,
    #[serde(default)]
    pub start: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Principal>,
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
    #[serde(default, skip_serializing_if = "is_local_workspace_mode")]
    pub workspace_mode: WorkspaceMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_base_dir: Option<std::path::PathBuf>,
}

fn is_local_workspace_mode(value: &WorkspaceMode) -> bool {
    *value == WorkspaceMode::Local
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

#[derive(Debug, Clone, PartialEq)]
pub struct EnterReview {
    pub task: ThreadTask,
    pub handoff: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<TaskExecutor>,
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

/// Read seam over the SQL task projection (`task_projection` table).
/// Projections derive in the same transaction as every record write
/// (#TASK-1864), so readers are structurally current — there is no
/// staleness gate and no repair path. Errors are backend failures
/// (SQLite/IO) and must surface to the caller instead of degrading
/// into empty results.
#[async_trait]
pub trait TaskProjectionReader: Send + Sync {
    async fn thread_id_for_number(&self, number: u64) -> Result<Option<String>, String>;
    async fn has_running_subtask_targeting(&self, thread_id: &str) -> Result<bool, String>;
    async fn list_task_summaries(
        &self,
        filter: &TaskListFilter,
    ) -> Result<(Vec<TaskSummary>, usize, bool), String>;
}

/// Mandatory fail-closed gate for every task thread that acquires a new agent
/// binding. Gateway supplies the store-backed implementation; TaskService
/// never resolves or defaults an agent on its own.
#[async_trait]
pub trait NewTaskAgentGate: Send + Sync {
    async fn resolve_new_task_agent(
        &self,
        requested_agent_id: Option<&str>,
    ) -> Result<ResolvedAgentBinding, AgentBindingError>;

    /// Resolve an already-bound task agent without applying the enabled gate.
    /// This is the assign/rework exemption: only the same canonical agent may
    /// be returned for an existing binding.
    async fn resolve_existing_task_agent(
        &self,
        current_agent_id: &str,
    ) -> Result<ResolvedAgentBinding, AgentBindingError>;
}

/// Scan-backed task projection for stores without SQL projections.
/// Answers the same queries by walking the store; only correct for
/// in-memory stores, where the walk is a hash-map iteration.
pub struct ScanTaskProjectionReader {
    store: Arc<dyn ThreadStore>,
}

impl ScanTaskProjectionReader {
    pub fn new(store: Arc<dyn ThreadStore>) -> Self {
        Self { store }
    }

    async fn tasks(&self) -> Result<Vec<(String, Value, ThreadTask)>, String> {
        let mut tasks = Vec::new();
        let keys = self
            .store
            .list_keys(None)
            .await
            .map_err(|error| error.to_string())?;
        for key in keys {
            if !is_thread_key(&key) {
                continue;
            }
            let Some(record) = self
                .store
                .get(&key)
                .await
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            let Ok(Some(task)) = task_from_record(&record) else {
                continue;
            };
            tasks.push((key, record, task));
        }
        Ok(tasks)
    }
}

fn scan_task_summary(thread_id: String, record: &Value, task: &ThreadTask) -> TaskSummary {
    TaskSummary {
        thread_id,
        task_id: canonical_task_id(task),
        number: task.number,
        title: task.title.clone(),
        status: task.status,
        creator: task.creator.clone(),
        assignee: task.assignee.clone(),
        source: task.source.clone(),
        executor: task.executor.clone(),
        updated_at: task.updated_at,
        updated_by: task.updated_by.clone(),
        runtime_agent_id: crate::agent_id_from_value(record).unwrap_or_default(),
        reply_count: u32::try_from(crate::history_message_count(record)).unwrap_or(u32::MAX),
    }
}

fn scan_matches_source_thread(task: &ThreadTask, source_thread_id: &str) -> bool {
    let Some(source) = task.source.as_ref() else {
        return false;
    };
    source.thread_id.as_deref() == Some(source_thread_id)
        || source.task_thread_id.as_deref() == Some(source_thread_id)
}

fn scan_matches_source_task(task: &ThreadTask, source_task_id: &str) -> bool {
    task.source
        .as_ref()
        .and_then(|source| source.task_id.as_deref())
        .is_some_and(|task_id| task_id.eq_ignore_ascii_case(source_task_id))
}

fn scan_matches_source_bot(task: &ThreadTask, source_bot_id: &str) -> bool {
    let Some(source) = task.source.as_ref() else {
        return false;
    };
    if source.bot_id.as_deref() == Some(source_bot_id) {
        return true;
    }
    match (&source.channel, &source.account_id) {
        (Some(channel), Some(account_id)) => format!("{channel}:{account_id}") == source_bot_id,
        _ => false,
    }
}

#[async_trait]
impl TaskProjectionReader for ScanTaskProjectionReader {
    async fn thread_id_for_number(&self, number: u64) -> Result<Option<String>, String> {
        Ok(self
            .tasks()
            .await?
            .into_iter()
            .find(|(_, _, task)| task.number == number)
            .map(|(thread_id, _, _)| thread_id))
    }

    async fn has_running_subtask_targeting(&self, thread_id: &str) -> Result<bool, String> {
        Ok(self.tasks().await?.into_iter().any(|(key, _, task)| {
            key != thread_id
                && task.status == TaskStatus::InProgress
                && matches!(
                    task.notification_target.as_ref(),
                    Some(TaskNotificationTarget::Thread { thread_id: target }) if target == thread_id
                )
        }))
    }

    async fn list_task_summaries(
        &self,
        filter: &TaskListFilter,
    ) -> Result<(Vec<TaskSummary>, usize, bool), String> {
        let limit = filter.limit.unwrap_or(DEFAULT_TASK_LIST_LIMIT);
        let offset = filter.offset.unwrap_or(0);
        let mut summaries = Vec::new();
        for (key, record, task) in self.tasks().await? {
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
                .is_some_and(|value| !scan_matches_source_thread(&task, value))
            {
                continue;
            }
            if filter
                .source_task_id
                .as_deref()
                .is_some_and(|value| !scan_matches_source_task(&task, value))
            {
                continue;
            }
            if filter
                .source_bot_id
                .as_deref()
                .is_some_and(|value| !scan_matches_source_bot(&task, value))
            {
                continue;
            }
            summaries.push(scan_task_summary(key, &record, &task));
        }
        summaries.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.thread_id.cmp(&right.thread_id))
        });
        let total = summaries.len();
        let page = summaries
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let has_more = offset.saturating_add(page.len()) < total;
        Ok((page, total, has_more))
    }
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
    projection_reader: Option<Arc<dyn TaskProjectionReader>>,
    new_task_agent_gate: Arc<dyn NewTaskAgentGate>,
}

impl TaskService {
    pub fn new(
        thread_store: Arc<dyn ThreadStore>,
        counter_store: Arc<dyn TaskCounterStore>,
        new_task_agent_gate: Arc<dyn NewTaskAgentGate>,
    ) -> Self {
        Self {
            thread_store,
            counter_store,
            projection_reader: None,
            new_task_agent_gate,
        }
    }

    pub fn with_projection_reader(mut self, reader: Arc<dyn TaskProjectionReader>) -> Self {
        self.projection_reader = Some(reader);
        self
    }

    fn projection_reader(&self) -> Arc<dyn TaskProjectionReader> {
        self.projection_reader
            .clone()
            .unwrap_or_else(|| task_projection_reader_for(&self.thread_store))
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
        let auto_start = input.start || input.assignee.is_some() || input.executor.is_some();
        let requested_agent_id = runtime
            .as_ref()
            .and_then(|runtime| normalized_nonempty_string(runtime.agent_id.as_deref()))
            .or_else(|| match &input.executor {
                Some(TaskExecutor::Agent { agent_id }) => Some(agent_id.clone()),
                None => None,
            })
            .or_else(|| match &input.assignee {
                Some(Principal::Agent { agent_id }) => Some(agent_id.clone()),
                _ => None,
            })
            .or_else(|| match (&actor, auto_start) {
                (Principal::Agent { agent_id }, true) => Some(agent_id.clone()),
                (Principal::Human { .. }, true) => None,
                (_, false) => None,
            });
        let should_bind_agent = auto_start || requested_agent_id.is_some();
        let resolved_binding = if should_bind_agent {
            Some(
                self.new_task_agent_gate
                    .resolve_new_task_agent(requested_agent_id.as_deref())
                    .await?,
            )
        } else {
            None
        };
        let workspace_dir = runtime
            .as_ref()
            .and_then(|runtime| normalized_nonempty_string(runtime.workspace_dir.as_deref()))
            .or_else(|| normalized_nonempty_string(input.workspace_dir.as_deref()))
            .or_else(|| {
                resolved_binding
                    .as_ref()
                    .and_then(|binding| binding.default_workspace_dir.clone())
            });
        let workspace_mode = runtime
            .as_ref()
            .map(|runtime| runtime.workspace_mode)
            .unwrap_or_default();
        let worktree_base_dir = runtime
            .as_ref()
            .and_then(|runtime| runtime.worktree_base_dir.clone());

        let (thread_id, mut record) = create_thread_record(
            &self.thread_store,
            ThreadEnsureOptions {
                label: input.title.clone(),
                workspace_dir,
                workspace_mode,
                worktree_base_dir,
                agent_id: resolved_binding
                    .as_ref()
                    .map(|binding| binding.agent_id.clone()),
                provider_type: resolved_binding
                    .as_ref()
                    .map(|binding| binding.provider_type.clone()),
                metadata: resolved_binding
                    .as_ref()
                    .map(|binding| binding.runtime_metadata.clone())
                    .unwrap_or_default(),
                thread_kind: Some("task".to_owned()),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            if error.starts_with("workspace_mode=worktree") {
                TaskServiceError::BadRequest(error)
            } else {
                TaskServiceError::Store(error)
            }
        })?;

        // The task body is no longer seeded into a record `messages` copy
        // (#TASK-1864 batch 1c): `task.body` is the canonical source and the
        // dispatch run writes the body to the transcript as its user turn.
        let body = normalized_limited(input.body, 8_000)?;

        let title = derive_title(input.title.as_deref(), body.as_deref());
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
                input.executor,
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
        set_task_thread_title(&mut record, &task)?;
        set_task_on_record(&mut record, &task)?;
        self.thread_store
            .set(&thread_id, record)
            .await
            .map_err(store_error)?;
        Ok((thread_id, task))
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
        let filter = TaskListFilter {
            limit: Some(limit),
            offset: Some(offset),
            ..filter
        };
        self.projection_reader()
            .list_task_summaries(&filter)
            .await
            .map_err(TaskServiceError::Store)
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
        self.assign_task_with_record(task_id, to, actor)
            .await
            .map(|(_, _, task)| task)
    }

    /// Assign and, when required, stamp the task thread's first agent binding
    /// in the same per-thread critical section and record write. The returned
    /// record is the exact committed state used by gateway dispatch.
    pub async fn assign_task_with_record(
        &self,
        task_id: &str,
        to: Principal,
        actor: Option<Principal>,
    ) -> Result<(String, Value, ThreadTask), TaskServiceError> {
        validate_principal(&to)?;
        let actor = actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        let (thread_id, _) = self.resolve_task_record(task_id).await?;
        let lock = task_thread_lock(&thread_id);
        let _guard = lock.lock().await;
        let mut record = self
            .thread_store
            .get(&thread_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| TaskServiceError::NotFound(thread_id.clone()))?;
        let mut task = task_from_record(&record)?
            .ok_or_else(|| TaskServiceError::NotATask(thread_id.clone()))?;

        if let Principal::Agent { agent_id } = &to {
            let requested_agent_id = agent_id.trim();
            let current_agent_id = crate::agent_id_from_value(&record);
            let (binding, is_new_binding) = if let Some(current_agent_id) = current_agent_id {
                let binding = self
                    .new_task_agent_gate
                    .resolve_existing_task_agent(&current_agent_id)
                    .await?;
                if binding.agent_id != requested_agent_id {
                    return Err(TaskServiceError::BadRequest(format!(
                        "task thread {thread_id} is bound to agent {}; cannot assign it to agent {requested_agent_id}",
                        binding.agent_id
                    )));
                }
                (binding, false)
            } else {
                (
                    self.new_task_agent_gate
                        .resolve_new_task_agent(Some(requested_agent_id))
                        .await?,
                    true,
                )
            };
            if binding.agent_id != requested_agent_id {
                return Err(TaskServiceError::BadRequest(format!(
                    "task thread {thread_id} resolved agent {}; cannot assign it to agent {requested_agent_id}",
                    binding.agent_id
                )));
            }
            if let Some(thread_provider_type) = record
                .get("provider_type")
                .cloned()
                .and_then(|value| serde_json::from_value::<garyx_models::ProviderType>(value).ok())
                && thread_provider_type != binding.provider_type
            {
                return Err(TaskServiceError::BadRequest(format!(
                    "task thread {thread_id} is bound to provider {thread_provider_type:?}; cannot assign it to provider {:?}",
                    binding.provider_type
                )));
            }
            apply_assignment_agent_binding(&mut record, &binding, is_new_binding)?;
        }

        let previous = task.assignee.clone();
        task.assignee = Some(to.clone());
        let previous_status = task.status;
        push_event(
            &mut task,
            actor.clone(),
            TaskEventKind::Assigned { from: previous, to },
            None,
        );
        if previous_status == TaskStatus::Todo {
            task.status = TaskStatus::InProgress;
            push_event(
                &mut task,
                actor,
                TaskEventKind::StatusChanged {
                    from: previous_status,
                    to: TaskStatus::InProgress,
                    note: Some("assigned".to_owned()),
                },
                None,
            );
        }
        set_task_on_record(&mut record, &task)?;
        self.thread_store
            .set(&thread_id, record.clone())
            .await
            .map_err(store_error)?;
        Ok((thread_id, record, task))
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

    pub async fn stop_task(
        &self,
        task_id: &str,
        actor: Option<Principal>,
    ) -> Result<ThreadTask, TaskServiceError> {
        let actor = actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        self.mutate_task(task_id, move |task| {
            let from_status = task.status;
            if from_status == TaskStatus::InProgress {
                task.status = TaskStatus::Todo;
                push_event(
                    task,
                    actor.clone(),
                    TaskEventKind::StatusChanged {
                        from: from_status,
                        to: TaskStatus::Todo,
                        note: Some("stopped".to_owned()),
                    },
                    None,
                );
            }
            if let Some(previous_assignee) = task.assignee.take() {
                push_event(
                    task,
                    actor,
                    TaskEventKind::Released {
                        previous_assignee: Some(previous_assignee),
                    },
                    None,
                );
            }
            Ok(())
        })
        .await
    }

    pub async fn delete_task(
        &self,
        task_id: &str,
    ) -> Result<(String, ThreadTask), TaskServiceError> {
        let (thread_id, _) = self.resolve_task_record(task_id).await?;
        let lock = task_thread_lock(&thread_id);
        let _guard = lock.lock().await;
        let mut record = self
            .thread_store
            .get(&thread_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| TaskServiceError::NotFound(thread_id.clone()))?;
        let task = task_from_record(&record)?
            .ok_or_else(|| TaskServiceError::NotATask(thread_id.clone()))?;
        remove_task_from_record(&mut record)?;
        self.thread_store
            .set(&thread_id, record)
            .await
            .map_err(store_error)?;
        Ok((thread_id, task))
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
        let (thread_id, _) = self.resolve_task_record(task_id).await?;
        let lock = task_thread_lock(&thread_id);
        let _guard = lock.lock().await;
        let mut record = self
            .thread_store
            .get(&thread_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| TaskServiceError::NotFound(thread_id.clone()))?;
        let mut task = task_from_record(&record)?
            .ok_or_else(|| TaskServiceError::NotATask(thread_id.clone()))?;
        let should_update_thread_title = is_task_thread_title_managed(&record, &task);
        let previous = task.title.clone();
        if previous == next_title {
            return Ok(task);
        }
        task.title = next_title.clone();
        push_event(
            &mut task,
            actor,
            TaskEventKind::TitleChanged {
                from: previous,
                to: next_title,
            },
            None,
        );
        if should_update_thread_title {
            set_task_thread_title(&mut record, &task)?;
        }
        set_task_on_record(&mut record, &task)?;
        self.thread_store
            .set(&thread_id, record)
            .await
            .map_err(store_error)?;
        Ok(task)
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
            .map_err(store_error)?
            .ok_or_else(|| TaskServiceError::NotFound(thread_id.clone()))?;
        let mut task = task_from_record(&record)?
            .ok_or_else(|| TaskServiceError::NotATask(thread_id.clone()))?;
        f(&mut task)?;
        set_task_on_record(&mut record, &task)?;
        self.thread_store
            .set(&thread_id, record)
            .await
            .map_err(store_error)?;
        Ok(task)
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
        executor: Option<TaskExecutor>,
        event_kind: TaskEventKind,
    ) -> Result<ThreadTask, TaskServiceError> {
        // The counter store owns the uniqueness invariant: every
        // allocation returns a number strictly greater than any number
        // it has handed out before and any number present in the task
        // projection (enforced atomically by the SQLite-backed store).
        let number = self.counter_store.allocate().await?;
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
            executor,
            body: None,
            created_at: now,
            updated_at: now,
            updated_by: actor.clone(),
            events: Vec::new(),
        };
        push_event(&mut task, actor, event_kind, Some(now));
        Ok(task)
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
                    .map_err(store_error)?
                    .ok_or_else(|| TaskServiceError::NotFound(thread_id.clone()))?;
                Ok((thread_id, record))
            }
            TaskId::Number(number) => self.find_task_by_number(number).await,
        }
    }

    async fn find_task_by_number(&self, number: u64) -> Result<(String, Value), TaskServiceError> {
        let thread_id = self
            .projection_reader()
            .thread_id_for_number(number)
            .await
            .map_err(TaskServiceError::Store)?
            .ok_or_else(|| TaskServiceError::NotFound(format!("#TASK-{number}")))?;
        // Projections derive in the same write transaction as the record,
        // so a projection row without a matching record body indicates a
        // bug, not staleness — surface it as NotFound without repair.
        if let Some(record) = self
            .thread_store
            .get(&thread_id)
            .await
            .map_err(store_error)?
            && let Some(task) = task_from_record(&record)?
            && task.number == number
        {
            return Ok((thread_id, record));
        }
        tracing::warn!(
            number,
            thread_id = %thread_id,
            "task projection row does not match its thread record"
        );
        Err(TaskServiceError::NotFound(format!("#TASK-{number}")))
    }
}

pub async fn mark_thread_task_in_review_if_in_progress(
    thread_store: &Arc<dyn ThreadStore>,
    thread_id: &str,
    actor: Principal,
    note: Option<String>,
    handoff: Option<String>,
) -> Result<Option<EnterReview>, TaskServiceError> {
    validate_principal(&actor)?;
    let lock = task_thread_lock(thread_id);
    let _guard = lock.lock().await;
    let Some(mut record) = thread_store.get(thread_id).await.map_err(store_error)? else {
        return Ok(None);
    };
    let Some(mut task) = task_from_record(&record)? else {
        return Ok(None);
    };
    if task.status != TaskStatus::InProgress {
        return Ok(None);
    }
    // A run that ends while this task still has running subtasks is not the
    // task's final result: each subtask's completion callback re-activates
    // this thread, and the task acts again before its own outcome is ready.
    // Leaving the task in progress defers the ready-for-review transition
    // (and the parent notification it triggers) to the run that ends with no
    // running subtasks left.
    if thread_task_has_running_subtasks(thread_store, thread_id).await? {
        tracing::info!(
            thread_id = %thread_id,
            task_id = %canonical_task_id(&task),
            "task run ended with running subtasks; leaving task in progress"
        );
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
    thread_store
        .set(thread_id, record)
        .await
        .map_err(store_error)?;
    Ok(Some(EnterReview { task, handoff }))
}

pub async fn mark_thread_task_in_progress_on_wake(
    thread_store: &Arc<dyn ThreadStore>,
    thread_id: &str,
    actor: Principal,
) -> Result<Option<ThreadTask>, TaskServiceError> {
    validate_principal(&actor)?;
    let lock = task_thread_lock(thread_id);
    let _guard = lock.lock().await;
    let Some(mut record) = thread_store.get(thread_id).await.map_err(store_error)? else {
        return Ok(None);
    };
    let Some(mut task) = task_from_record(&record)? else {
        return Ok(None);
    };
    if !matches!(task.status, TaskStatus::InReview | TaskStatus::Done) {
        return Ok(None);
    }
    let from = task.status;
    task.status = TaskStatus::InProgress;
    push_event(
        &mut task,
        actor,
        TaskEventKind::StatusChanged {
            from,
            to: TaskStatus::InProgress,
            note: None,
        },
        None,
    );
    set_task_on_record(&mut record, &task)?;
    thread_store
        .set(thread_id, record)
        .await
        .map_err(store_error)?;
    Ok(Some(task))
}

/// True when another task is still running and targets this thread for its
/// completion notification. Such a task is a subtask of this thread's task:
/// it was created from this thread and will call back into it when done.
///
/// Answered by the store's SQL task projection, or the scan reader for
/// stores without one (in-memory embedders, unit tests).
pub async fn thread_task_has_running_subtasks(
    thread_store: &Arc<dyn ThreadStore>,
    thread_id: &str,
) -> Result<bool, TaskServiceError> {
    task_projection_reader_for(thread_store)
        .has_running_subtask_targeting(thread_id)
        .await
        .map_err(TaskServiceError::Store)
}

/// The task projection for this store: the store's own SQL reader when the
/// backend maintains one (SQLite), else [`ScanTaskProjectionReader`] — the
/// structural equivalent for in-memory stores. Lifetime is tied to the
/// store; there is no process-global registry.
pub fn task_projection_reader_for(store: &Arc<dyn ThreadStore>) -> Arc<dyn TaskProjectionReader> {
    store
        .task_projection()
        .unwrap_or_else(|| Arc::new(ScanTaskProjectionReader::new(store.clone())))
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
    obj.insert("thread_kind".to_owned(), Value::String("task".to_owned()));
    obj.insert("task".to_owned(), serde_json::to_value(task)?);
    obj.insert(
        "updated_at".to_owned(),
        Value::String(task.updated_at.to_rfc3339()),
    );
    Ok(())
}

fn task_thread_title(task: &ThreadTask) -> String {
    format!("{} {}", canonical_task_id(task), task.title)
}

fn set_task_thread_title(record: &mut Value, task: &ThreadTask) -> Result<(), TaskServiceError> {
    let obj = record
        .as_object_mut()
        .ok_or_else(|| TaskServiceError::Store("thread record is not an object".to_owned()))?;
    obj.insert("label".to_owned(), Value::String(task_thread_title(task)));
    obj.insert(
        "thread_title_source".to_owned(),
        Value::String(TASK_THREAD_TITLE_SOURCE.to_owned()),
    );
    obj.remove("provider_thread_title");
    Ok(())
}

fn is_task_thread_title_managed(record: &Value, task: &ThreadTask) -> bool {
    if record
        .get("thread_title_source")
        .and_then(Value::as_str)
        .map(str::trim)
        != Some(TASK_THREAD_TITLE_SOURCE)
    {
        return false;
    }
    record
        .get("label")
        .and_then(Value::as_str)
        .is_some_and(|label| label == task_thread_title(task))
}

fn apply_assignment_agent_binding(
    record: &mut Value,
    binding: &ResolvedAgentBinding,
    is_new_binding: bool,
) -> Result<(), TaskServiceError> {
    let should_fill_workspace = crate::workspace_dir_from_value(record).is_none();
    let obj = record
        .as_object_mut()
        .ok_or_else(|| TaskServiceError::Store("thread record is not an object".to_owned()))?;
    if is_new_binding {
        obj.insert(
            "agent_id".to_owned(),
            Value::String(binding.agent_id.clone()),
        );
        obj.insert(
            "provider_type".to_owned(),
            serde_json::to_value(&binding.provider_type)?,
        );
        let metadata = obj
            .entry("metadata".to_owned())
            .or_insert_with(|| Value::Object(Default::default()));
        if !metadata.is_object() {
            *metadata = Value::Object(Default::default());
        }
        let metadata = metadata.as_object_mut().ok_or_else(|| {
            TaskServiceError::Store("thread metadata is not an object".to_owned())
        })?;
        for (key, value) in &binding.runtime_metadata {
            if SERVER_OWNED_AGENT_METADATA_KEYS.contains(&key.as_str()) {
                metadata.insert(key.clone(), value.clone());
            } else {
                metadata.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    if should_fill_workspace
        && let Some(workspace_dir) = binding
            .default_workspace_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        obj.insert(
            "workspace_dir".to_owned(),
            Value::String(workspace_dir.to_owned()),
        );
    }
    Ok(())
}

fn remove_task_from_record(record: &mut Value) -> Result<(), TaskServiceError> {
    let obj = record
        .as_object_mut()
        .ok_or_else(|| TaskServiceError::Store("thread record is not an object".to_owned()))?;
    obj.insert("thread_kind".to_owned(), Value::String("task".to_owned()));
    obj.remove("task");
    obj.insert(
        "updated_at".to_owned(),
        Value::String(Utc::now().to_rfc3339()),
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

fn derive_title(input: Option<&str>, body: Option<&str>) -> String {
    // Title fallback reads the task body directly (#TASK-1864 batch 1) —
    // the former scan of the freshly created record's `messages` only ever
    // found the seeded copy of this same body.
    if let Some(title) = input
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(truncate_title)
    {
        return title;
    }
    if let Some(title) = body
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(truncate_title)
    {
        return title;
    }
    "Untitled task".to_owned()
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

fn store_error(error: crate::ThreadStoreError) -> TaskServiceError {
    TaskServiceError::Store(error.to_string())
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
            | (TaskStatus::Done, TaskStatus::InProgress)
            | (TaskStatus::Done, TaskStatus::Todo)
    )
}

#[cfg(test)]
mod tests;
