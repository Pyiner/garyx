use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use chrono::Utc;
use garyx_models::local_paths::default_session_data_dir;
use garyx_models::{
    Principal, TaskEventKind, TaskNotificationTarget, TaskSource, TaskStatus, ThreadTask,
};
use garyx_router::{
    CreateTaskInput, FileTaskCounterStore, PromoteTaskInput, TaskListFilter, TaskRuntimeInput,
    TaskService, TaskServiceError, UpdateTaskStatusInput, workspace_dir_from_value,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_identity::{
    default_workspace_dir_from_agent_reference, resolve_agent_reference_from_stores,
};
use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};
use crate::server::AppState;

const ACTOR_HEADER: &str = "x-garyx-actor";

#[derive(Debug, Deserialize)]
pub struct CreateTaskBody {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assignee: Option<Principal>,
    #[serde(default, alias = "notificationTarget")]
    pub notification_target: Option<TaskNotificationTargetBody>,
    #[serde(default)]
    pub source: Option<TaskSource>,
    #[serde(default)]
    pub start: bool,
    #[serde(default)]
    pub actor: Option<Principal>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub runtime: Option<TaskRuntimeBody>,
}

#[derive(Debug, Deserialize)]
pub struct BatchCreateTaskBody {
    pub tasks: Vec<BatchTaskItem>,
    #[serde(default)]
    pub actor: Option<Principal>,
    #[serde(default, alias = "notificationTarget")]
    pub notification_target: Option<TaskNotificationTargetBody>,
    #[serde(default)]
    pub source: Option<TaskSource>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub runtime: Option<TaskRuntimeBody>,
}

#[derive(Debug, Deserialize)]
pub struct BatchTaskItem {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assignee: Option<Principal>,
    #[serde(default, alias = "notificationTarget")]
    pub notification_target: Option<TaskNotificationTargetBody>,
    #[serde(default)]
    pub source: Option<TaskSource>,
    #[serde(default)]
    pub start: bool,
    #[serde(default)]
    pub runtime: Option<TaskRuntimeBody>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskRuntimeBody {
    #[serde(default, alias = "agentId")]
    pub agent_id: Option<String>,
    #[serde(default, alias = "workspaceDir")]
    pub workspace_dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TaskNotificationTargetBody {
    None,
    Thread {
        #[serde(alias = "threadId")]
        thread_id: String,
    },
    Bot {
        channel: String,
        #[serde(alias = "accountId")]
        account_id: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct PromoteTaskBody {
    pub thread_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub assignee: Option<Principal>,
    #[serde(default, alias = "notificationTarget")]
    pub notification_target: Option<TaskNotificationTargetBody>,
    #[serde(default)]
    pub source: Option<TaskSource>,
    #[serde(default)]
    pub actor: Option<Principal>,
}

#[derive(Debug, Deserialize)]
pub struct AssignTaskBody {
    pub to: Principal,
    #[serde(default)]
    pub actor: Option<Principal>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTaskStatusBody {
    pub to: TaskStatus,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub actor: Option<Principal>,
}

#[derive(Debug, Deserialize)]
pub struct SetTaskTitleBody {
    pub title: String,
    #[serde(default)]
    pub actor: Option<Principal>,
}

#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(default, alias = "sourceThreadId")]
    pub source_thread_id: Option<String>,
    #[serde(default, alias = "sourceTaskId")]
    pub source_task_id: Option<String>,
    #[serde(default, alias = "sourceBotId")]
    pub source_bot_id: Option<String>,
    #[serde(default)]
    pub include_done: bool,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct TaskHistoryQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub before: Option<String>,
}

pub async fn create_task(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateTaskBody>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    let actor = match actor_from_request(body.actor, &headers) {
        Ok(actor) => actor,
        Err(error) => return task_error_response(error),
    };
    let runtime = task_runtime_input(body.runtime, body.agent_id, body.workspace_dir);
    if let Err(error) = validate_runtime_agent(&state, &runtime).await {
        return task_error_response(error);
    }
    if let Err(error) = validate_task_assignee_agent(&state, body.assignee.as_ref()).await {
        return task_error_response(error);
    }
    let notification_target = match required_notification_target(body.notification_target) {
        Ok(target) => target,
        Err(error) => return task_error_response(error),
    };
    let runtime =
        match task_runtime_with_default_workspace(&state, runtime, body.assignee.as_ref()).await {
            Ok(runtime) => runtime,
            Err(error) => return task_error_response(error),
        };
    let title_for_dispatch = body.title.clone();
    let body_for_dispatch = body.body.clone();
    match service
        .create_task(CreateTaskInput {
            title: body.title,
            body: body.body,
            assignee: body.assignee,
            notification_target,
            source: body.source,
            start: body.start,
            actor,
            agent_id: None,
            workspace_dir: None,
            runtime,
        })
        .await
    {
        Ok((thread_id, task)) => {
            if let Err(error) =
                ensure_created_task_thread_provider_from_bound_agent(&state, &thread_id).await
            {
                return task_error_response(error);
            }
            let runtime_agent_id = runtime_agent_id_for_thread(&state, &thread_id).await;
            let mut payload = json!({
                "thread_id": thread_id,
                "task_id": garyx_router::tasks::canonical_task_id(&task),
                "number": task.number,
                "status": task.status,
                "runtime_agent_id": runtime_agent_id,
                "task": task,
            });
            if let Some(dispatch) = spawn_task_auto_dispatch(
                state.clone(),
                payload["thread_id"].as_str().unwrap_or_default().to_owned(),
                payload["task"].clone(),
                "create",
                title_for_dispatch.as_deref(),
                body_for_dispatch.as_deref(),
            ) {
                payload["dispatch"] = dispatch;
            }
            (StatusCode::CREATED, Json(payload))
        }
        Err(error) => task_error_response(error),
    }
}

pub async fn create_tasks_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BatchCreateTaskBody>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    if body.tasks.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "tasks cannot be empty", "code": "BadRequest"})),
        );
    }
    let actor = match actor_from_request(body.actor, &headers) {
        Ok(actor) => actor,
        Err(error) => return task_error_response(error),
    };
    let top_runtime = task_runtime_input(body.runtime, body.agent_id, body.workspace_dir);
    let top_notification_target = body.notification_target.map(TaskNotificationTarget::from);
    let top_source = body.source;
    if let Err(error) = validate_runtime_agent(&state, &top_runtime).await {
        return task_error_response(error);
    }
    let mut created = Vec::new();
    for item in body.tasks {
        let title_for_dispatch = item.title.clone();
        let body_for_dispatch = item.body.clone();
        let runtime = item
            .runtime
            .map(TaskRuntimeInput::from)
            .or_else(|| top_runtime.clone());
        let notification_target = match item
            .notification_target
            .map(TaskNotificationTarget::from)
            .or_else(|| top_notification_target.clone())
        {
            Some(target) => target,
            None => {
                return task_error_response(TaskServiceError::BadRequest(
                    "notification_target is required; choose a bot, thread, or none".to_owned(),
                ));
            }
        };
        if let Err(error) = validate_runtime_agent(&state, &runtime).await {
            return task_error_response(error);
        }
        if let Err(error) = validate_task_assignee_agent(&state, item.assignee.as_ref()).await {
            return task_error_response(error);
        }
        let runtime = match task_runtime_with_default_workspace(
            &state,
            runtime,
            item.assignee.as_ref(),
        )
        .await
        {
            Ok(runtime) => runtime,
            Err(error) => return task_error_response(error),
        };
        match service
            .create_task(CreateTaskInput {
                title: item.title,
                body: item.body,
                assignee: item.assignee,
                notification_target: Some(notification_target),
                source: item.source.or_else(|| top_source.clone()),
                start: item.start,
                actor: actor.clone(),
                agent_id: None,
                workspace_dir: None,
                runtime,
            })
            .await
        {
            Ok((thread_id, task)) => {
                let runtime_agent_id = runtime_agent_id_for_thread(&state, &thread_id).await;
                let mut payload = json!({
                    "thread_id": thread_id,
                    "task_id": garyx_router::tasks::canonical_task_id(&task),
                    "number": task.number,
                    "status": task.status,
                    "runtime_agent_id": runtime_agent_id,
                    "task": task,
                });
                if let Some(dispatch) = spawn_task_auto_dispatch(
                    state.clone(),
                    payload["thread_id"].as_str().unwrap_or_default().to_owned(),
                    payload["task"].clone(),
                    "create",
                    title_for_dispatch.as_deref(),
                    body_for_dispatch.as_deref(),
                ) {
                    payload["dispatch"] = dispatch;
                }
                created.push(payload)
            }
            Err(error) => return task_error_response(error),
        }
    }
    (StatusCode::CREATED, Json(json!({ "tasks": created })))
}

pub async fn promote_task(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<PromoteTaskBody>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    let actor = match actor_from_request(body.actor, &headers) {
        Ok(actor) => actor,
        Err(error) => return task_error_response(error),
    };
    if let Err(error) = validate_task_assignee_agent(&state, body.assignee.as_ref()).await {
        return task_error_response(error);
    }
    let notification_target = match required_notification_target(body.notification_target) {
        Ok(target) => target,
        Err(error) => return task_error_response(error),
    };
    let thread_id = body.thread_id;
    let title_for_dispatch = body.title.clone();
    match service
        .promote_task(PromoteTaskInput {
            thread_id: thread_id.clone(),
            title: body.title,
            assignee: body.assignee,
            notification_target,
            source: body.source,
            actor,
        })
        .await
    {
        Ok(task) => {
            if let Err(error) = ensure_thread_workspace_from_assignee_default(
                &state,
                &thread_id,
                task.assignee.as_ref(),
            )
            .await
            {
                return task_error_response(error);
            }
            let runtime_agent_id = runtime_agent_id_for_thread(&state, &thread_id).await;
            let mut payload = json!({
                "task_id": garyx_router::tasks::canonical_task_id(&task),
                "number": task.number,
                "status": task.status,
                "runtime_agent_id": runtime_agent_id,
                "task": task,
            });
            if let Some(dispatch) = spawn_task_auto_dispatch(
                state.clone(),
                thread_id,
                payload["task"].clone(),
                "promote",
                title_for_dispatch.as_deref(),
                None,
            ) {
                payload["dispatch"] = dispatch;
            }
            (StatusCode::OK, Json(payload))
        }
        Err(error) => task_error_response(error),
    }
}

pub async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    match service.get_task(&task_id).await {
        Ok((thread_id, thread, task)) => (
            StatusCode::OK,
            Json(json!({
                "thread_id": thread_id,
                "task_id": garyx_router::tasks::canonical_task_id(&task),
                "task": task,
                "thread": thread,
            })),
        ),
        Err(error) => task_error_response(error),
    }
}

pub async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TaskListQuery>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    let filter = match task_list_filter(query) {
        Ok(filter) => filter,
        Err(error) => return task_error_response(error),
    };
    match service.list_tasks(filter).await {
        Ok((tasks, total, has_more)) => (
            StatusCode::OK,
            Json(json!({
                "tasks": tasks,
                "total": total,
                "has_more": has_more,
            })),
        ),
        Err(error) => task_error_response(error),
    }
}

pub async fn task_history(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    Query(query): Query<TaskHistoryQuery>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    match service
        .task_history(&task_id, query.limit, query.before.as_deref())
        .await
    {
        Ok(page) => (
            StatusCode::OK,
            Json(json!({ "events": page.events, "has_more": page.has_more })),
        ),
        Err(error) => task_error_response(error),
    }
}

pub async fn assign_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<AssignTaskBody>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    let actor = match actor_from_request(body.actor, &headers) {
        Ok(actor) => actor,
        Err(error) => return task_error_response(error),
    };
    if let Err(error) = validate_task_assignee_agent(&state, Some(&body.to)).await {
        return task_error_response(error);
    }
    let assignee = body.to;
    let self_claim = actor.as_ref() == Some(&assignee);
    if let Ok((thread_id, _, _)) = service.get_task(&task_id).await {
        if let Err(error) =
            validate_thread_runtime_allows_assignee(&state, &thread_id, &assignee).await
        {
            return task_error_response(error);
        }
    }
    match service.assign_task(&task_id, assignee, actor).await {
        Ok(task) => {
            let assignee_for_workspace = task.assignee.clone();
            let mut payload = json!({ "task": task });
            if let Ok((thread_id, record, _)) = service.get_task(&task_id).await {
                if let Err(error) = ensure_thread_workspace_from_assignee_default(
                    &state,
                    &thread_id,
                    assignee_for_workspace.as_ref(),
                )
                .await
                {
                    return task_error_response(error);
                }
                if !self_claim {
                    let body_for_dispatch = task_body_for_dispatch(&payload["task"])
                        .or_else(|| task_body_from_record(&record));
                    if let Some(dispatch) = spawn_task_auto_dispatch(
                        state.clone(),
                        thread_id,
                        payload["task"].clone(),
                        "assign",
                        None,
                        body_for_dispatch.as_deref(),
                    ) {
                        payload["dispatch"] = dispatch;
                    }
                }
            }
            (StatusCode::OK, Json(payload))
        }
        Err(error) => task_error_response(error),
    }
}

pub async fn unassign_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    let actor = match actor_from_request(None, &headers) {
        Ok(actor) => actor,
        Err(error) => return task_error_response(error),
    };
    match service.unassign_task(&task_id, actor).await {
        Ok(task) => (StatusCode::OK, Json(json!({ "task": task }))),
        Err(error) => task_error_response(error),
    }
}

pub async fn update_task_status(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpdateTaskStatusBody>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    let actor = match actor_from_request(body.actor, &headers) {
        Ok(actor) => actor,
        Err(error) => return task_error_response(error),
    };
    match service
        .update_status(UpdateTaskStatusInput {
            task_id: task_id.clone(),
            to: body.to,
            note: body.note,
            force: body.force,
            actor,
        })
        .await
    {
        Ok(task) => {
            if task_ready_for_review_transition(&task)
                && let Ok((thread_id, _, _)) = service.get_task(&task_id).await
            {
                let task_id_for_notification = garyx_router::tasks::canonical_task_id(&task);
                let state_for_notification = state.clone();
                tokio::spawn(async move {
                    let event = crate::task_notifications::TaskReadyForReviewEvent {
                        thread_id,
                        task_id: task_id_for_notification,
                        run_id: None,
                        final_message: None,
                    };
                    if let Err(error) = crate::task_notifications::dispatch_task_ready_notification(
                        &state_for_notification,
                        event,
                    )
                    .await
                    {
                        tracing::warn!(error = ?error, "manual task ready notification failed");
                    }
                });
            }
            (StatusCode::OK, Json(json!({ "task": task })))
        }
        Err(error) => task_error_response(error),
    }
}

pub async fn set_task_title(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SetTaskTitleBody>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    let actor = match actor_from_request(body.actor, &headers) {
        Ok(actor) => actor,
        Err(error) => return task_error_response(error),
    };
    match service.set_title(&task_id, body.title, actor).await {
        Ok(task) => (StatusCode::OK, Json(json!({ "task": task }))),
        Err(error) => task_error_response(error),
    }
}

fn task_service(state: &Arc<AppState>) -> Option<TaskService> {
    let config = state.config_snapshot();
    if !config.tasks.enabled {
        return None;
    }
    let data_dir = config
        .sessions
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(default_session_data_dir);
    Some(TaskService::new(
        state.threads.thread_store.clone(),
        Arc::new(FileTaskCounterStore::new(data_dir)),
    ))
}

fn task_list_filter(query: TaskListQuery) -> Result<TaskListFilter, TaskServiceError> {
    Ok(TaskListFilter {
        status: query.status,
        assignee: query.assignee.as_deref().map(parse_principal).transpose()?,
        creator: query.creator.as_deref().map(parse_principal).transpose()?,
        source_thread_id: normalized_nonempty(query.source_thread_id),
        source_task_id: normalized_nonempty(query.source_task_id),
        source_bot_id: normalized_nonempty(query.source_bot_id),
        include_done: query.include_done,
        limit: query.limit,
        offset: query.offset,
    })
}

fn parse_principal(value: &str) -> Result<Principal, TaskServiceError> {
    let trimmed = value.trim();
    if let Some(agent_id) = trimmed.strip_prefix("agent:") {
        return Ok(Principal::Agent {
            agent_id: agent_id.trim().to_owned(),
        });
    }
    if let Some(user_id) = trimmed.strip_prefix("human:") {
        return Ok(Principal::Human {
            user_id: user_id.trim().to_owned(),
        });
    }
    Ok(Principal::Agent {
        agent_id: trimmed.to_owned(),
    })
}

fn actor_from_request(
    body_actor: Option<Principal>,
    headers: &HeaderMap,
) -> Result<Option<Principal>, TaskServiceError> {
    if body_actor.is_some() {
        return Ok(body_actor);
    }
    let Some(value) = headers.get(ACTOR_HEADER) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| TaskServiceError::BadRequest("invalid X-Garyx-Actor header".to_owned()))?;
    parse_principal(value).map(Some)
}

impl From<TaskRuntimeBody> for TaskRuntimeInput {
    fn from(value: TaskRuntimeBody) -> Self {
        Self {
            agent_id: normalized_nonempty(value.agent_id),
            workspace_dir: normalized_nonempty(value.workspace_dir),
        }
    }
}

impl From<TaskNotificationTargetBody> for TaskNotificationTarget {
    fn from(value: TaskNotificationTargetBody) -> Self {
        match value {
            TaskNotificationTargetBody::None => Self::None,
            TaskNotificationTargetBody::Thread { thread_id } => Self::Thread { thread_id },
            TaskNotificationTargetBody::Bot {
                channel,
                account_id,
            } => Self::Bot {
                channel,
                account_id,
            },
        }
    }
}

fn required_notification_target(
    value: Option<TaskNotificationTargetBody>,
) -> Result<Option<TaskNotificationTarget>, TaskServiceError> {
    value
        .map(TaskNotificationTarget::from)
        .map(Some)
        .ok_or_else(|| {
            TaskServiceError::BadRequest(
                "notification_target is required; choose a bot, thread, or none".to_owned(),
            )
        })
}

fn task_ready_for_review_transition(task: &ThreadTask) -> bool {
    matches!(
        task.events.last().map(|event| &event.kind),
        Some(TaskEventKind::StatusChanged {
            from: TaskStatus::InProgress,
            to: TaskStatus::InReview,
            ..
        })
    )
}

fn task_runtime_input(
    runtime: Option<TaskRuntimeBody>,
    legacy_agent_id: Option<String>,
    legacy_workspace_dir: Option<String>,
) -> Option<TaskRuntimeInput> {
    let mut input = runtime
        .map(TaskRuntimeInput::from)
        .unwrap_or(TaskRuntimeInput {
            agent_id: None,
            workspace_dir: None,
        });
    if input.agent_id.is_none() {
        input.agent_id = normalized_nonempty(legacy_agent_id);
    }
    if input.workspace_dir.is_none() {
        input.workspace_dir = normalized_nonempty(legacy_workspace_dir);
    }
    (input.agent_id.is_some() || input.workspace_dir.is_some()).then_some(input)
}

fn task_runtime_has_workspace(runtime: &Option<TaskRuntimeInput>) -> bool {
    runtime
        .as_ref()
        .and_then(|runtime| runtime.workspace_dir.as_deref())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

fn task_agent_id_for_default_workspace(
    runtime: &Option<TaskRuntimeInput>,
    assignee: Option<&Principal>,
) -> Option<String> {
    match assignee {
        Some(Principal::Agent { agent_id }) => normalized_nonempty(Some(agent_id.clone())),
        _ => None,
    }
    .or_else(|| {
        runtime
            .as_ref()
            .and_then(|runtime| normalized_nonempty(runtime.agent_id.clone()))
    })
}

async fn default_workspace_dir_for_agent(
    state: &Arc<AppState>,
    agent_id: &str,
) -> Result<Option<String>, TaskServiceError> {
    resolve_agent_reference_from_stores(
        state.ops.custom_agents.as_ref(),
        state.ops.agent_teams.as_ref(),
        agent_id,
    )
    .await
    .map(|reference| default_workspace_dir_from_agent_reference(&reference))
    .map_err(TaskServiceError::UnknownAgent)
}

async fn task_runtime_with_default_workspace(
    state: &Arc<AppState>,
    runtime: Option<TaskRuntimeInput>,
    assignee: Option<&Principal>,
) -> Result<Option<TaskRuntimeInput>, TaskServiceError> {
    if task_runtime_has_workspace(&runtime) {
        return Ok(runtime);
    }
    let Some(agent_id) = task_agent_id_for_default_workspace(&runtime, assignee) else {
        return Ok(runtime);
    };
    let Some(default_workspace_dir) = default_workspace_dir_for_agent(state, &agent_id).await?
    else {
        return Ok(runtime);
    };
    let mut input = runtime.unwrap_or(TaskRuntimeInput {
        agent_id: None,
        workspace_dir: None,
    });
    input.workspace_dir = Some(default_workspace_dir);
    Ok(Some(input))
}

async fn validate_thread_runtime_allows_assignee(
    state: &Arc<AppState>,
    thread_id: &str,
    assignee: &Principal,
) -> Result<(), TaskServiceError> {
    let Principal::Agent { agent_id } = assignee else {
        return Ok(());
    };
    let Some(thread) = state.threads.thread_store.get(thread_id).await else {
        return Ok(());
    };
    let reference = resolve_agent_reference_from_stores(
        state.ops.custom_agents.as_ref(),
        state.ops.agent_teams.as_ref(),
        agent_id,
    )
    .await
    .map_err(TaskServiceError::UnknownAgent)?;

    let thread_agent_id = thread
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let Some(thread_agent_id) = thread_agent_id else {
        return Ok(());
    };
    if thread_agent_id != reference.bound_agent_id() {
        return Err(TaskServiceError::BadRequest(format!(
            "task thread {thread_id} is bound to agent {thread_agent_id}; cannot assign it to agent {}",
            reference.bound_agent_id()
        )));
    }

    if let Some(thread_provider_type) = thread
        .get("provider_type")
        .cloned()
        .and_then(|value| serde_json::from_value::<garyx_models::ProviderType>(value).ok())
    {
        let reference_provider_type = reference.provider_type();
        if thread_provider_type != reference_provider_type {
            return Err(TaskServiceError::BadRequest(format!(
                "task thread {thread_id} is bound to provider {thread_provider_type:?}; cannot assign it to provider {reference_provider_type:?}"
            )));
        }
    }

    Ok(())
}

async fn ensure_thread_workspace_from_assignee_default(
    state: &Arc<AppState>,
    thread_id: &str,
    assignee: Option<&Principal>,
) -> Result<(), TaskServiceError> {
    let Some(Principal::Agent { agent_id }) = assignee else {
        return Ok(());
    };
    let Some(mut updated) = state.threads.thread_store.get(thread_id).await else {
        return Ok(());
    };
    let reference = resolve_agent_reference_from_stores(
        state.ops.custom_agents.as_ref(),
        state.ops.agent_teams.as_ref(),
        agent_id,
    )
    .await
    .map_err(TaskServiceError::UnknownAgent)?;
    let existing_workspace_dir = workspace_dir_from_value(&updated);
    let default_workspace_dir = if existing_workspace_dir.is_none() {
        default_workspace_dir_for_agent(state, agent_id).await?
    } else {
        None
    };
    let should_bind_agent = updated
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none();
    let Some(obj) = updated.as_object_mut() else {
        return Err(TaskServiceError::Store(format!(
            "thread payload is not an object: {thread_id}"
        )));
    };
    if let Some(default_workspace_dir) = default_workspace_dir {
        obj.insert(
            "workspace_dir".to_owned(),
            Value::String(default_workspace_dir),
        );
    }
    if should_bind_agent {
        obj.insert(
            "agent_id".to_owned(),
            Value::String(reference.bound_agent_id().to_owned()),
        );
        obj.insert("provider_type".to_owned(), json!(reference.provider_type()));
    }
    obj.insert(
        "updated_at".to_owned(),
        Value::String(Utc::now().to_rfc3339()),
    );
    state
        .threads
        .thread_store
        .set(thread_id, updated.clone())
        .await;
    state
        .integration
        .bridge
        .set_thread_workspace_binding(thread_id, workspace_dir_from_value(&updated))
        .await;
    Ok(())
}

async fn ensure_created_task_thread_provider_from_bound_agent(
    state: &Arc<AppState>,
    thread_id: &str,
) -> Result<(), TaskServiceError> {
    let Some(mut updated) = state.threads.thread_store.get(thread_id).await else {
        return Ok(());
    };
    if updated.get("provider_type").is_some() {
        return Ok(());
    }
    let Some(agent_id) = updated
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
    else {
        return Ok(());
    };
    let reference = resolve_agent_reference_from_stores(
        state.ops.custom_agents.as_ref(),
        state.ops.agent_teams.as_ref(),
        &agent_id,
    )
    .await
    .map_err(TaskServiceError::UnknownAgent)?;
    let Some(obj) = updated.as_object_mut() else {
        return Err(TaskServiceError::Store(format!(
            "thread payload is not an object: {thread_id}"
        )));
    };
    obj.insert(
        "agent_id".to_owned(),
        Value::String(reference.bound_agent_id().to_owned()),
    );
    obj.insert("provider_type".to_owned(), json!(reference.provider_type()));
    obj.insert(
        "updated_at".to_owned(),
        Value::String(Utc::now().to_rfc3339()),
    );
    state.threads.thread_store.set(thread_id, updated).await;
    Ok(())
}

fn spawn_task_auto_dispatch(
    state: Arc<AppState>,
    thread_id: String,
    task_value: Value,
    reason: &'static str,
    requested_title: Option<&str>,
    requested_body: Option<&str>,
) -> Option<Value> {
    let task: ThreadTask = serde_json::from_value(task_value).ok()?;
    let Principal::Agent { agent_id } = task.assignee.as_ref()? else {
        return None;
    };
    let agent_id = agent_id.clone();
    if task.status != TaskStatus::InProgress {
        return None;
    }
    let task_id = garyx_router::tasks::canonical_task_id(&task);
    let run_id = format!("task-auto-{}-{}", task.number, Uuid::now_v7());
    let body = requested_body
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| task.body.clone());
    let message =
        task_auto_dispatch_message(&task_id, &task.title, requested_title, body.as_deref());
    let mut extra_metadata = HashMap::new();
    extra_metadata.insert("task_auto_start".to_owned(), Value::Bool(true));
    extra_metadata.insert(
        "task_dispatch_reason".to_owned(),
        Value::String(reason.to_owned()),
    );
    extra_metadata.insert("task_id".to_owned(), Value::String(task_id.clone()));
    let dispatch_run_id = run_id.clone();
    let dispatch_thread_id = thread_id.clone();
    tokio::spawn(async move {
        if let Err(error) = dispatch_internal_message_to_thread(
            &state,
            &dispatch_thread_id,
            &dispatch_run_id,
            &message,
            InternalDispatchOptions {
                extra_metadata,
                ..Default::default()
            },
        )
        .await
        {
            tracing::warn!(
                task_id = %task_id,
                thread_id = %dispatch_thread_id,
                run_id = %dispatch_run_id,
                error = %error,
                "task auto dispatch failed"
            );
        }
    });
    Some(json!({
        "queued": true,
        "run_id": run_id,
        "agent_id": agent_id,
    }))
}

fn task_auto_dispatch_message(
    task_id: &str,
    task_title: &str,
    requested_title: Option<&str>,
    requested_body: Option<&str>,
) -> String {
    let title = requested_title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(task_title);
    let body = requested_body
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match body {
        Some(body) => {
            format!("Task {task_id} has been assigned to you and is already in progress.\n\n{body}")
        }
        None => format!(
            "Task {task_id} has been assigned to you and is already in progress.\n\nTitle: {title}"
        ),
    }
}

fn task_body_for_dispatch(task_value: &Value) -> Option<String> {
    task_value
        .get("body")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn task_body_from_record(record: &Value) -> Option<String> {
    task_body_for_dispatch(record.get("task")?).or_else(|| {
        record
            .get("messages")
            .and_then(Value::as_array)?
            .iter()
            .find_map(|message| {
                if message.get("role").and_then(Value::as_str) != Some("user") {
                    return None;
                }
                message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            })
    })
}

fn normalized_nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

async fn validate_runtime_agent(
    state: &Arc<AppState>,
    runtime: &Option<TaskRuntimeInput>,
) -> Result<(), TaskServiceError> {
    let Some(agent_id) = runtime
        .as_ref()
        .and_then(|runtime| runtime.agent_id.as_deref())
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })
    else {
        return Ok(());
    };
    resolve_agent_reference_from_stores(
        state.ops.custom_agents.as_ref(),
        state.ops.agent_teams.as_ref(),
        agent_id,
    )
    .await
    .map(|_| ())
    .map_err(TaskServiceError::UnknownAgent)
}

async fn validate_task_assignee_agent(
    state: &Arc<AppState>,
    assignee: Option<&Principal>,
) -> Result<(), TaskServiceError> {
    let Some(Principal::Agent { agent_id }) = assignee else {
        return Ok(());
    };
    resolve_agent_reference_from_stores(
        state.ops.custom_agents.as_ref(),
        state.ops.agent_teams.as_ref(),
        agent_id,
    )
    .await
    .map(|_| ())
    .map_err(TaskServiceError::UnknownAgent)
}

async fn runtime_agent_id_for_thread(state: &Arc<AppState>, thread_id: &str) -> String {
    state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .and_then(|record| garyx_router::agent_id_from_value(&record))
        .unwrap_or_default()
}

fn tasks_disabled() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": "tasks are disabled",
            "code": "TasksDisabled",
        })),
    )
}

fn task_error_response(error: TaskServiceError) -> (StatusCode, Json<Value>) {
    let code = match &error {
        TaskServiceError::NotFound(_) => "NotFound",
        TaskServiceError::NotATask(_) => "NotATask",
        TaskServiceError::AlreadyATask(_) => "AlreadyATask",
        TaskServiceError::InvalidTransition { .. } => "InvalidTransition",
        TaskServiceError::BadRequest(_) => "BadRequest",
        TaskServiceError::UnknownPrincipal(_) => "UnknownPrincipal",
        TaskServiceError::UnknownAgent(_) => "UnknownAgent",
        TaskServiceError::Store(_) | TaskServiceError::Counter(_) | TaskServiceError::Serde(_) => {
            "Internal"
        }
    };
    let status = match code {
        "NotFound" => StatusCode::NOT_FOUND,
        "NotATask" | "AlreadyATask" | "InvalidTransition" | "BadRequest" | "UnknownPrincipal"
        | "UnknownAgent" => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(json!({ "error": error.to_string(), "code": code })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_teams::AgentTeamStore;
    use crate::custom_agents::CustomAgentStore;
    use crate::server::AppStateBuilder;
    use garyx_models::ProviderType;
    use garyx_models::config::GaryxConfig;

    async fn state_with_agent_default_workspace() -> Arc<AppState> {
        let custom_agents = Arc::new(CustomAgentStore::new());
        custom_agents
            .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
                agent_id: "reviewer".to_owned(),
                display_name: "Reviewer".to_owned(),
                provider_type: ProviderType::CodexAppServer,
                model: "gpt-5".to_owned(),
                default_workspace_dir: Some("/tmp/agent-task-default".to_owned()),
                system_prompt: "Review carefully.".to_owned(),
            })
            .await
            .expect("custom agent");
        AppStateBuilder::new(GaryxConfig::default())
            .with_custom_agent_store(custom_agents)
            .with_agent_team_store(Arc::new(AgentTeamStore::new()))
            .build()
    }

    #[tokio::test]
    async fn task_runtime_uses_assignee_default_workspace_when_unset() {
        let state = state_with_agent_default_workspace().await;
        let runtime = task_runtime_with_default_workspace(
            &state,
            None,
            Some(&Principal::Agent {
                agent_id: "reviewer".to_owned(),
            }),
        )
        .await
        .expect("runtime");

        assert_eq!(
            runtime.and_then(|runtime| runtime.workspace_dir).as_deref(),
            Some("/tmp/agent-task-default")
        );
    }

    #[tokio::test]
    async fn task_runtime_explicit_workspace_overrides_agent_default() {
        let state = state_with_agent_default_workspace().await;
        let runtime = task_runtime_with_default_workspace(
            &state,
            Some(TaskRuntimeInput {
                agent_id: Some("reviewer".to_owned()),
                workspace_dir: Some("/tmp/task-explicit".to_owned()),
            }),
            Some(&Principal::Agent {
                agent_id: "reviewer".to_owned(),
            }),
        )
        .await
        .expect("runtime");

        assert_eq!(
            runtime.and_then(|runtime| runtime.workspace_dir).as_deref(),
            Some("/tmp/task-explicit")
        );
    }

    #[tokio::test]
    async fn task_runtime_without_agent_default_keeps_workspace_unset() {
        let state = AppStateBuilder::new(GaryxConfig::default())
            .with_custom_agent_store(Arc::new(CustomAgentStore::new()))
            .with_agent_team_store(Arc::new(AgentTeamStore::new()))
            .build();
        let runtime = task_runtime_with_default_workspace(
            &state,
            None,
            Some(&Principal::Agent {
                agent_id: "claude".to_owned(),
            }),
        )
        .await
        .expect("runtime");

        assert!(runtime.is_none());
    }
}
