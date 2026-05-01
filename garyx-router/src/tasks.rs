use std::sync::Arc;

use chrono::Utc;
use garyx_models::{
    Principal, TASK_SCHEMA_VERSION_V1, TaskEvent, TaskEventKind, TaskScope, TaskStatus, ThreadTask,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{TaskCounterError, TaskCounterStore};
use crate::{ThreadEnsureOptions, ThreadStore, create_thread_record, is_thread_key};

const DEFAULT_TASK_LIST_LIMIT: usize = 50;
const MAX_TASK_LIST_LIMIT: usize = 200;
const DEFAULT_TASK_AGENT_ID: &str = "claude";

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
    #[error("InvalidScope: {0}")]
    InvalidScope(String),
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("UnknownPrincipal: {0}")]
    UnknownPrincipal(String),
    #[error("store error: {0}")]
    Store(String),
    #[error(transparent)]
    Counter(#[from] TaskCounterError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskInput {
    pub scope: TaskScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Principal>,
    #[serde(default)]
    pub start: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Principal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteTaskInput {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Principal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Principal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTaskStatusInput {
    pub task_ref: String,
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
    pub scope: Option<TaskScope>,
    pub status: Option<TaskStatus>,
    pub assignee: Option<Principal>,
    pub creator: Option<Principal>,
    pub include_done: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub thread_id: String,
    pub task_ref: String,
    pub number: u64,
    pub title: String,
    pub status: TaskStatus,
    pub scope: TaskScope,
    pub creator: Principal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Principal>,
    pub updated_at: chrono::DateTime<Utc>,
    pub updated_by: Principal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRef {
    ThreadId(String),
    Qualified { scope: TaskScope, number: u64 },
    Short(u64),
}

impl TaskRef {
    pub fn parse(input: &str) -> Result<Self, TaskServiceError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(TaskServiceError::BadRequest("task_ref is empty".to_owned()));
        }
        if is_thread_key(trimmed) {
            return Ok(Self::ThreadId(trimmed.to_owned()));
        }
        let Some(rest) = trimmed.strip_prefix('#') else {
            return Err(TaskServiceError::BadRequest(format!(
                "unrecognized task_ref: {trimmed}"
            )));
        };
        if let Ok(number) = rest.parse::<u64>() {
            return Ok(Self::Short(number));
        }
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != 3 {
            return Err(TaskServiceError::BadRequest(format!(
                "unrecognized task_ref: {trimmed}"
            )));
        }
        let number = parts[2].parse::<u64>().map_err(|_| {
            TaskServiceError::BadRequest(format!("invalid task number: {}", parts[2]))
        })?;
        Ok(Self::Qualified {
            scope: TaskScope::new(parts[0], parts[1]),
            number,
        })
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

    pub async fn create_task(
        &self,
        input: CreateTaskInput,
    ) -> Result<(String, ThreadTask), TaskServiceError> {
        validate_scope(&input.scope)?;
        let actor = input.actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        if let Some(assignee) = &input.assignee {
            validate_principal(assignee)?;
        }
        let thread_agent_id = input.agent_id.clone().or_else(|| match &actor {
            Principal::Agent { agent_id } => Some(agent_id.clone()),
            Principal::Human { .. } => Some(DEFAULT_TASK_AGENT_ID.to_owned()),
        });

        let (thread_id, mut record) = create_thread_record(
            &self.thread_store,
            ThreadEnsureOptions {
                label: input.title.clone(),
                agent_id: thread_agent_id,
                origin_channel: Some(input.scope.channel.clone()),
                origin_account_id: Some(input.scope.account_id.clone()),
                ..Default::default()
            },
        )
        .await
        .map_err(TaskServiceError::Store)?;

        if let Some(body) = normalized_limited(input.body, 8_000)? {
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
                input.scope,
                title,
                if input.start {
                    TaskStatus::InProgress
                } else {
                    TaskStatus::Todo
                },
                actor,
                input.assignee,
                TaskEventKind::Created {
                    initial_status: if input.start {
                        TaskStatus::InProgress
                    } else {
                        TaskStatus::Todo
                    },
                    assignee: None,
                },
            )
            .await?;

        let mut task = task;
        if let TaskEventKind::Created { assignee, .. } = &mut task.events[0].kind {
            *assignee = task.assignee.clone();
        }
        set_task_on_record(&mut record, &task)?;
        self.thread_store.set(&thread_id, record).await;
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
        let (mut record, scope) = self.load_record_and_scope(&input.thread_id).await?;
        if task_from_record(&record)?.is_some() {
            return Err(TaskServiceError::AlreadyATask(input.thread_id));
        }
        let title = derive_title(input.title.as_deref(), &record);
        let task = self
            .build_task(
                scope,
                title,
                TaskStatus::Todo,
                actor,
                input.assignee,
                TaskEventKind::Promoted {
                    initial_status: TaskStatus::Todo,
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
        Ok(task)
    }

    pub async fn get_task(
        &self,
        task_ref: &str,
        context_scope: Option<&TaskScope>,
    ) -> Result<(String, Value, ThreadTask), TaskServiceError> {
        let (thread_id, record) = self.resolve_task_record(task_ref, context_scope).await?;
        let task = task_from_record(&record)?
            .ok_or_else(|| TaskServiceError::NotATask(thread_id.clone()))?;
        Ok((thread_id, record, task))
    }

    pub async fn list_tasks(
        &self,
        filter: TaskListFilter,
    ) -> Result<(Vec<TaskSummary>, usize, bool), TaskServiceError> {
        if let Some(scope) = &filter.scope {
            validate_scope(scope)?;
        }
        let limit = filter
            .limit
            .unwrap_or(DEFAULT_TASK_LIST_LIMIT)
            .clamp(1, MAX_TASK_LIST_LIMIT);
        let offset = filter.offset.unwrap_or(0);
        let mut tasks = Vec::new();
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
            if !filter.include_done && task.status == TaskStatus::Done {
                continue;
            }
            if filter
                .scope
                .as_ref()
                .is_some_and(|scope| &task.scope != scope)
            {
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
            tasks.push(TaskSummary::from_task(key, &task));
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
        task_ref: &str,
        context_scope: Option<&TaskScope>,
        limit: Option<usize>,
    ) -> Result<Vec<TaskEvent>, TaskServiceError> {
        let (_, _, task) = self.get_task(task_ref, context_scope).await?;
        let limit = limit
            .unwrap_or(DEFAULT_TASK_LIST_LIMIT)
            .clamp(1, MAX_TASK_LIST_LIMIT);
        Ok(task.events.into_iter().rev().take(limit).collect())
    }

    pub async fn assign_task(
        &self,
        task_ref: &str,
        to: Principal,
        actor: Option<Principal>,
        context_scope: Option<&TaskScope>,
    ) -> Result<ThreadTask, TaskServiceError> {
        validate_principal(&to)?;
        let actor = actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        let (thread_id, mut record, mut task) = self.get_task(task_ref, context_scope).await?;
        let previous = task.assignee.clone();
        let was_self_claim = task.status == TaskStatus::Todo && to == actor;
        task.assignee = Some(to.clone());
        if was_self_claim {
            task.status = TaskStatus::InProgress;
            push_event(
                &mut task,
                actor,
                TaskEventKind::Claimed { from: previous },
                None,
            );
        } else {
            push_event(
                &mut task,
                actor,
                TaskEventKind::Assigned { from: previous, to },
                None,
            );
        }
        set_task_on_record(&mut record, &task)?;
        self.thread_store.set(&thread_id, record).await;
        Ok(task)
    }

    pub async fn unassign_task(
        &self,
        task_ref: &str,
        actor: Option<Principal>,
        context_scope: Option<&TaskScope>,
    ) -> Result<ThreadTask, TaskServiceError> {
        let actor = actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        let (thread_id, mut record, mut task) = self.get_task(task_ref, context_scope).await?;
        let previous = task
            .assignee
            .clone()
            .ok_or_else(|| TaskServiceError::BadRequest("task is already unassigned".to_owned()))?;
        task.assignee = None;
        push_event(
            &mut task,
            actor,
            TaskEventKind::Unassigned { from: previous },
            None,
        );
        set_task_on_record(&mut record, &task)?;
        self.thread_store.set(&thread_id, record).await;
        Ok(task)
    }

    pub async fn update_status(
        &self,
        input: UpdateTaskStatusInput,
        context_scope: Option<&TaskScope>,
    ) -> Result<ThreadTask, TaskServiceError> {
        let actor = input.actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        let (thread_id, mut record, mut task) =
            self.get_task(&input.task_ref, context_scope).await?;
        let from = task.status;
        if from == input.to {
            return Ok(task);
        }
        if !input.force && !is_allowed_transition(from, input.to) {
            return Err(TaskServiceError::InvalidTransition { from, to: input.to });
        }
        let event_kind = match (from, input.to, input.force) {
            (TaskStatus::Todo, TaskStatus::InProgress, false) => {
                let previous = task.assignee.clone();
                if task.assignee.is_none() {
                    task.assignee = Some(actor.clone());
                }
                TaskEventKind::Claimed { from: previous }
            }
            (TaskStatus::InProgress, TaskStatus::Todo, false) => {
                let previous_assignee = task.assignee.take();
                TaskEventKind::Released { previous_assignee }
            }
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
        push_event(&mut task, actor, event_kind, None);
        set_task_on_record(&mut record, &task)?;
        self.thread_store.set(&thread_id, record).await;
        Ok(task)
    }

    pub async fn set_title(
        &self,
        task_ref: &str,
        title: String,
        actor: Option<Principal>,
        context_scope: Option<&TaskScope>,
    ) -> Result<ThreadTask, TaskServiceError> {
        let actor = actor.unwrap_or_else(default_actor);
        validate_principal(&actor)?;
        let next_title = normalized_limited(Some(title), 200)?
            .ok_or_else(|| TaskServiceError::BadRequest("title cannot be empty".to_owned()))?;
        let (thread_id, mut record, mut task) = self.get_task(task_ref, context_scope).await?;
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
        set_task_on_record(&mut record, &task)?;
        self.thread_store.set(&thread_id, record).await;
        Ok(task)
    }

    async fn build_task(
        &self,
        scope: TaskScope,
        title: String,
        status: TaskStatus,
        actor: Principal,
        assignee: Option<Principal>,
        event_kind: TaskEventKind,
    ) -> Result<ThreadTask, TaskServiceError> {
        let number = self.counter_store.allocate(&scope).await?;
        let now = Utc::now();
        let mut task = ThreadTask {
            schema_version: TASK_SCHEMA_VERSION_V1,
            scope,
            number,
            title,
            status,
            creator: actor.clone(),
            assignee,
            created_at: now,
            updated_at: now,
            updated_by: actor.clone(),
            events: Vec::new(),
        };
        push_event(&mut task, actor, event_kind, Some(now));
        Ok(task)
    }

    async fn load_record_and_scope(
        &self,
        thread_id: &str,
    ) -> Result<(Value, TaskScope), TaskServiceError> {
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
        let scope = scope_from_thread_record(&record).ok_or_else(|| {
            TaskServiceError::InvalidScope(format!(
                "thread {thread_id} does not carry channel/account scope"
            ))
        })?;
        Ok((record, scope))
    }

    async fn resolve_task_record(
        &self,
        task_ref: &str,
        context_scope: Option<&TaskScope>,
    ) -> Result<(String, Value), TaskServiceError> {
        match TaskRef::parse(task_ref)? {
            TaskRef::ThreadId(thread_id) => {
                let record = self
                    .thread_store
                    .get(&thread_id)
                    .await
                    .ok_or_else(|| TaskServiceError::NotFound(thread_id.clone()))?;
                Ok((thread_id, record))
            }
            TaskRef::Qualified { scope, number } => self.find_task_by_number(&scope, number).await,
            TaskRef::Short(number) => {
                let scope = context_scope.ok_or_else(|| {
                    TaskServiceError::BadRequest(
                        "short task ref requires a channel/account scope".to_owned(),
                    )
                })?;
                self.find_task_by_number(scope, number).await
            }
        }
    }

    async fn find_task_by_number(
        &self,
        scope: &TaskScope,
        number: u64,
    ) -> Result<(String, Value), TaskServiceError> {
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
            if &task.scope == scope && task.number == number {
                return Ok((key, record));
            }
        }
        Err(TaskServiceError::NotFound(format!(
            "#{}/{}",
            scope.canonical(),
            number
        )))
    }
}

impl TaskSummary {
    fn from_task(thread_id: String, task: &ThreadTask) -> Self {
        Self {
            thread_id,
            task_ref: canonical_task_ref(task),
            number: task.number,
            title: task.title.clone(),
            status: task.status,
            scope: task.scope.clone(),
            creator: task.creator.clone(),
            assignee: task.assignee.clone(),
            updated_at: task.updated_at,
            updated_by: task.updated_by.clone(),
        }
    }
}

pub fn canonical_task_ref(task: &ThreadTask) -> String {
    format!(
        "#{}/{}/{}",
        task.scope.channel, task.scope.account_id, task.number
    )
}

pub fn task_from_record(record: &Value) -> Result<Option<ThreadTask>, TaskServiceError> {
    match record.get("task") {
        Some(Value::Null) | None => Ok(None),
        Some(value) => Ok(Some(serde_json::from_value(value.clone())?)),
    }
}

pub fn scope_from_thread_record(record: &Value) -> Option<TaskScope> {
    let channel = record
        .get("channel")
        .or_else(|| record.pointer("/origin/channel"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let account_id = record
        .get("account_id")
        .or_else(|| record.pointer("/origin/account_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(TaskScope::new(channel, account_id))
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

fn validate_scope(scope: &TaskScope) -> Result<(), TaskServiceError> {
    validate_scope_part(&scope.channel)?;
    validate_scope_part(&scope.account_id)?;
    Ok(())
}

fn validate_scope_part(value: &str) -> Result<(), TaskServiceError> {
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    {
        return Err(TaskServiceError::InvalidScope(value.to_owned()));
    }
    Ok(())
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
                scope: TaskScope::new("telegram", "main"),
                title: Some("Audit daemons".to_owned()),
                body: Some("Look at launchctl".to_owned()),
                assignee: None,
                start: false,
                actor: Some(Principal::Agent {
                    agent_id: "cindy".to_owned(),
                }),
                agent_id: Some("cindy".to_owned()),
            })
            .await
            .unwrap();
        assert_eq!(task.number, 1);
        let record = service.thread_store.get(&thread_id).await.unwrap();
        assert!(record.get("task").is_some());
        let messages = record.get("messages").and_then(Value::as_array).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], Value::String("user".to_owned()));
    }

    #[tokio::test]
    async fn status_machine_rejects_illegal_transition() {
        let service = service();
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                scope: TaskScope::new("telegram", "main"),
                title: Some("Review".to_owned()),
                body: None,
                assignee: None,
                start: false,
                actor: None,
                agent_id: None,
            })
            .await
            .unwrap();
        let error = service
            .update_status(
                UpdateTaskStatusInput {
                    task_ref: canonical_task_ref(&task),
                    to: TaskStatus::Done,
                    note: None,
                    force: false,
                    actor: None,
                },
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, TaskServiceError::InvalidTransition { .. }));
    }

    #[tokio::test]
    async fn status_machine_claims_todo() {
        let service = service();
        let (_thread_id, task) = service
            .create_task(CreateTaskInput {
                scope: TaskScope::new("telegram", "main"),
                title: Some("Claim me".to_owned()),
                body: None,
                assignee: None,
                start: false,
                actor: None,
                agent_id: None,
            })
            .await
            .unwrap();
        let updated = service
            .update_status(
                UpdateTaskStatusInput {
                    task_ref: canonical_task_ref(&task),
                    to: TaskStatus::InProgress,
                    note: None,
                    force: false,
                    actor: Some(Principal::Agent {
                        agent_id: "cindy".to_owned(),
                    }),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(
            updated.assignee,
            Some(Principal::Agent {
                agent_id: "cindy".to_owned()
            })
        );
    }
}
