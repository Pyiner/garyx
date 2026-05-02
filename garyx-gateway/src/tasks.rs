use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use garyx_models::local_paths::default_session_data_dir;
use garyx_models::{Principal, TaskScope, TaskStatus};
use garyx_router::{
    CreateTaskInput, FileTaskCounterStore, PromoteTaskInput, TaskListFilter, TaskRuntimeInput,
    TaskService, TaskServiceError, UpdateTaskStatusInput,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent_identity::resolve_agent_reference_from_stores;
use crate::server::AppState;

const ACTOR_HEADER: &str = "x-garyx-actor";

#[derive(Debug, Deserialize)]
pub struct CreateTaskBody {
    pub scope: TaskScope,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assignee: Option<Principal>,
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
    pub scope: TaskScope,
    pub tasks: Vec<BatchTaskItem>,
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
pub struct BatchTaskItem {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assignee: Option<Principal>,
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

#[derive(Debug, Deserialize)]
pub struct PromoteTaskBody {
    pub thread_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub assignee: Option<Principal>,
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
    pub scope: Option<String>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub creator: Option<String>,
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
    match service
        .create_task(CreateTaskInput {
            scope: body.scope,
            title: body.title,
            body: body.body,
            assignee: body.assignee,
            start: body.start,
            actor,
            agent_id: None,
            workspace_dir: None,
            runtime,
        })
        .await
    {
        Ok((thread_id, task)) => {
            let runtime_agent_id = runtime_agent_id_for_thread(&state, &thread_id).await;
            (
                StatusCode::CREATED,
                Json(json!({
                    "thread_id": thread_id,
                    "task_ref": garyx_router::tasks::canonical_task_ref(&task),
                    "number": task.number,
                    "status": task.status,
                    "runtime_agent_id": runtime_agent_id,
                    "task": task,
                })),
            )
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
    if let Err(error) = validate_runtime_agent(&state, &top_runtime).await {
        return task_error_response(error);
    }
    let mut created = Vec::new();
    for item in body.tasks {
        let runtime = item
            .runtime
            .map(TaskRuntimeInput::from)
            .or_else(|| top_runtime.clone());
        if let Err(error) = validate_runtime_agent(&state, &runtime).await {
            return task_error_response(error);
        }
        match service
            .create_task(CreateTaskInput {
                scope: body.scope.clone(),
                title: item.title,
                body: item.body,
                assignee: item.assignee,
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
                created.push(json!({
                    "thread_id": thread_id,
                    "task_ref": garyx_router::tasks::canonical_task_ref(&task),
                    "number": task.number,
                    "status": task.status,
                    "runtime_agent_id": runtime_agent_id,
                    "task": task,
                }))
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
    let thread_id = body.thread_id;
    match service
        .promote_task(PromoteTaskInput {
            thread_id: thread_id.clone(),
            title: body.title,
            assignee: body.assignee,
            actor,
        })
        .await
    {
        Ok(task) => {
            let runtime_agent_id = runtime_agent_id_for_thread(&state, &thread_id).await;
            (
                StatusCode::OK,
                Json(json!({
                    "task_ref": garyx_router::tasks::canonical_task_ref(&task),
                    "number": task.number,
                    "status": task.status,
                    "runtime_agent_id": runtime_agent_id,
                    "task": task,
                })),
            )
        }
        Err(error) => task_error_response(error),
    }
}

pub async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(task_ref): Path<String>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    match service.get_task(&task_ref, None).await {
        Ok((thread_id, thread, task)) => (
            StatusCode::OK,
            Json(json!({
                "thread_id": thread_id,
                "task_ref": garyx_router::tasks::canonical_task_ref(&task),
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
    Path(task_ref): Path<String>,
    Query(query): Query<TaskHistoryQuery>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    match service
        .task_history(&task_ref, None, query.limit, query.before.as_deref())
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
    Path(task_ref): Path<String>,
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
    match service.assign_task(&task_ref, body.to, actor, None).await {
        Ok(task) => (StatusCode::OK, Json(json!({ "task": task }))),
        Err(error) => task_error_response(error),
    }
}

pub async fn unassign_task(
    State(state): State<Arc<AppState>>,
    Path(task_ref): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    let actor = match actor_from_request(None, &headers) {
        Ok(actor) => actor,
        Err(error) => return task_error_response(error),
    };
    match service.unassign_task(&task_ref, actor, None).await {
        Ok(task) => (StatusCode::OK, Json(json!({ "task": task }))),
        Err(error) => task_error_response(error),
    }
}

pub async fn update_task_status(
    State(state): State<Arc<AppState>>,
    Path(task_ref): Path<String>,
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
        .update_status(
            UpdateTaskStatusInput {
                task_ref,
                to: body.to,
                note: body.note,
                force: body.force,
                actor,
            },
            None,
        )
        .await
    {
        Ok(task) => (StatusCode::OK, Json(json!({ "task": task }))),
        Err(error) => task_error_response(error),
    }
}

pub async fn set_task_title(
    State(state): State<Arc<AppState>>,
    Path(task_ref): Path<String>,
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
    match service.set_title(&task_ref, body.title, actor, None).await {
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
        scope: query.scope.as_deref().map(parse_scope).transpose()?,
        status: query.status,
        assignee: query.assignee.as_deref().map(parse_principal).transpose()?,
        creator: query.creator.as_deref().map(parse_principal).transpose()?,
        include_done: query.include_done,
        limit: query.limit,
        offset: query.offset,
    })
}

fn parse_scope(value: &str) -> Result<TaskScope, TaskServiceError> {
    let parts: Vec<&str> = value.trim().split('/').collect();
    if parts.len() != 2 {
        return Err(TaskServiceError::BadRequest(
            "scope must be <channel>/<account_id>".to_owned(),
        ));
    }
    Ok(TaskScope::new(parts[0], parts[1]))
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
        TaskServiceError::InvalidScope(_) => "InvalidScope",
        TaskServiceError::BadRequest(_) => "BadRequest",
        TaskServiceError::UnknownPrincipal(_) => "UnknownPrincipal",
        TaskServiceError::UnknownAgent(_) => "UnknownAgent",
        TaskServiceError::Store(_) | TaskServiceError::Counter(_) | TaskServiceError::Serde(_) => {
            "Internal"
        }
    };
    let status = match code {
        "NotFound" => StatusCode::NOT_FOUND,
        "NotATask" | "AlreadyATask" | "InvalidTransition" | "InvalidScope" | "BadRequest"
        | "UnknownPrincipal" | "UnknownAgent" => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(json!({ "error": error.to_string(), "code": code })),
    )
}
