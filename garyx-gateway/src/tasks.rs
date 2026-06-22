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
    AgentReference, Principal, TaskExecutor, TaskNotificationTarget, TaskSource, TaskStatus,
    ThreadTask,
};
use garyx_router::{
    CreateTaskInput, FileTaskCounterStore, TaskListFilter, TaskRuntimeInput, TaskService,
    TaskServiceError, UpdateTaskStatusInput, WorkspaceMode,
    mark_thread_task_in_review_if_in_progress, workspace_dir_from_value,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_identity::{
    default_workspace_dir_from_agent_reference, resolve_agent_reference_from_stores,
};
use crate::garyx_db::GaryxDbError;
use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};
use crate::server::AppState;
use crate::task_projection::backfill_task_projection_if_incomplete;
use crate::workflows::{
    WorkflowError, get_workflow_definition_package, spawn_workflow_task_entrypoint,
};
use crate::workspace_mode::worktree_base_dir_for_config;

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
    pub executor: Option<TaskExecutorBody>,
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
#[serde(rename_all = "snake_case", tag = "type")]
pub enum TaskExecutorBody {
    Agent {
        #[serde(alias = "agentId")]
        agent_id: String,
    },
    Team {
        #[serde(alias = "teamId")]
        team_id: String,
    },
    Workflow {
        #[serde(alias = "workflowId")]
        workflow_id: String,
        #[serde(default)]
        input: Option<Value>,
    },
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
    #[serde(default, alias = "workspaceMode")]
    pub workspace_mode: WorkspaceMode,
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
    let executor_body = body.executor;
    if executor_body.is_some() && body.assignee.is_some() {
        return task_error_response(TaskServiceError::BadRequest(
            "executor-backed tasks cannot also set an assignee".to_owned(),
        ));
    }
    let workspace_dir = body.workspace_dir;
    let normalized_workspace_dir = workspace_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let mut runtime = task_runtime_input(body.runtime, body.agent_id, workspace_dir.clone());
    let mut workflow_request = None;
    let mut workflow_definition = None;
    let mut task_executor = None;
    let mut executor_agent_id = None;
    let workflow_workspace_dir = match executor_body {
        Some(TaskExecutorBody::Agent { agent_id }) => {
            let reference = match resolve_task_executor_agent(&state, &agent_id).await {
                Ok(reference) => reference,
                Err(error) => return task_error_response(error),
            };
            let bound_agent_id = reference.bound_agent_id().to_owned();
            runtime = match task_runtime_for_executor(runtime, &bound_agent_id) {
                Ok(runtime) => runtime,
                Err(error) => return task_error_response(error),
            };
            executor_agent_id = Some(bound_agent_id.clone());
            task_executor = Some(TaskExecutor::Agent {
                agent_id: bound_agent_id,
            });
            None
        }
        Some(TaskExecutorBody::Team { team_id }) => {
            let reference = match resolve_task_executor_team(&state, &team_id).await {
                Ok(reference) => reference,
                Err(error) => return task_error_response(error),
            };
            let bound_team_id = reference.bound_agent_id().to_owned();
            runtime = match task_runtime_for_executor(runtime, &bound_team_id) {
                Ok(runtime) => runtime,
                Err(error) => return task_error_response(error),
            };
            executor_agent_id = Some(bound_team_id.clone());
            task_executor = Some(TaskExecutor::Team {
                team_id: bound_team_id,
            });
            None
        }
        Some(TaskExecutorBody::Workflow { workflow_id, input }) => {
            runtime = None;
            let definition =
                match get_workflow_definition_package(&state.config_snapshot(), &workflow_id) {
                    Ok(definition) => definition,
                    Err(error) => {
                        return task_error_response(TaskServiceError::BadRequest(
                            error.to_string(),
                        ));
                    }
                };
            task_executor = Some(TaskExecutor::Workflow {
                workflow_id: definition.record.workflow_id.clone(),
                workflow_version: Some(definition.record.version),
            });
            workflow_request = Some((
                definition.record.workflow_id.clone(),
                workflow_task_input_or_body(input, body.body.as_deref()),
            ));
            workflow_definition = Some(definition);
            normalized_workspace_dir.clone()
        }
        None => None,
    };
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
    let default_workspace_agent = executor_agent_id.as_deref().or(match &body.assignee {
        Some(Principal::Agent { agent_id }) => Some(agent_id.as_str()),
        _ => None,
    });
    let mut runtime =
        match task_runtime_with_default_workspace(&state, runtime, default_workspace_agent).await {
            Ok(runtime) => runtime,
            Err(error) => return task_error_response(error),
        };
    if let Some(runtime) = runtime.as_mut()
        && runtime.workspace_mode.is_worktree()
    {
        runtime.worktree_base_dir = Some(worktree_base_dir_for_config(&state.config_snapshot()));
    }
    let title_for_dispatch = body.title.clone();
    let body_for_dispatch = body.body.clone();
    match service
        .create_task(CreateTaskInput {
            title: body.title,
            body: body.body,
            assignee: body.assignee,
            notification_target,
            source: body.source,
            executor: task_executor,
            start: body.start || workflow_request.is_some() || executor_agent_id.is_some(),
            actor,
            agent_id: None,
            workspace_dir: workflow_workspace_dir.clone(),
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
            state.invalidate_gateway_sync_caches().await;
            let runtime_agent_id = runtime_agent_id_for_thread(&state, &thread_id).await;
            let mut payload = json!({
                "thread_id": thread_id,
                "task_id": garyx_router::tasks::canonical_task_id(&task),
                "number": task.number,
                "status": task.status,
                "runtime_agent_id": runtime_agent_id,
                "task": task,
            });
            if let (Some(definition), Some((_, input))) = (workflow_definition, workflow_request) {
                let task_id = garyx_router::tasks::canonical_task_id(&task);
                if let Err(error) = mark_task_thread_as_workflow_run(
                    &state,
                    &thread_id,
                    &definition.record.workflow_id,
                    definition.record.version,
                    workflow_workspace_dir.as_deref(),
                )
                .await
                {
                    return task_error_response(error);
                }
                match spawn_workflow_task_entrypoint(
                    state.clone(),
                    task_id,
                    thread_id.clone(),
                    definition.record.workflow_id,
                    input,
                    workflow_workspace_dir,
                ) {
                    Ok(dispatch) => payload["dispatch"] = dispatch,
                    Err(error) => {
                        let _ = mark_thread_task_in_review_if_in_progress(
                            &state.threads.thread_store,
                            &thread_id,
                            Principal::Agent {
                                agent_id: "workflow".to_owned(),
                            },
                            Some(format!("workflow entrypoint dispatch failed: {error}")),
                            None,
                        )
                        .await;
                        return workflow_error_as_task_response(error);
                    }
                }
            } else if let Some(dispatch) = spawn_task_auto_dispatch(
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
        let default_workspace_agent = match &item.assignee {
            Some(Principal::Agent { agent_id }) => Some(agent_id.as_str()),
            _ => None,
        };
        let mut runtime =
            match task_runtime_with_default_workspace(&state, runtime, default_workspace_agent)
                .await
            {
                Ok(runtime) => runtime,
                Err(error) => return task_error_response(error),
            };
        if let Some(runtime) = runtime.as_mut()
            && runtime.workspace_mode.is_worktree()
        {
            runtime.worktree_base_dir =
                Some(worktree_base_dir_for_config(&state.config_snapshot()));
        }
        match service
            .create_task(CreateTaskInput {
                title: item.title,
                body: item.body,
                assignee: item.assignee,
                notification_target: Some(notification_target),
                source: item.source.or_else(|| top_source.clone()),
                executor: None,
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
    state.invalidate_gateway_sync_caches().await;
    (StatusCode::CREATED, Json(json!({ "tasks": created })))
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

pub async fn list_task_forest(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TaskListQuery>,
) -> (StatusCode, Json<Value>) {
    if task_service(&state).is_none() {
        return tasks_disabled();
    }
    let filter = match task_list_filter(query) {
        Ok(filter) => filter,
        Err(error) => return task_error_response(error),
    };
    let projection_current_before = match state.ops.garyx_db.task_projection_is_current() {
        Ok(current) => current,
        Err(error) => return task_projection_error_response(error),
    };
    if !projection_current_before {
        backfill_task_projection_if_incomplete(&state.threads.thread_store, &state.ops.garyx_db)
            .await;
    }
    let projection_current = match state.ops.garyx_db.task_projection_is_current() {
        Ok(current) => current,
        Err(error) => return task_projection_error_response(error),
    };
    match state.ops.garyx_db.list_task_forest(&filter) {
        Ok((tasks, total)) => (
            StatusCode::OK,
            Json(json!({
                "tasks": tasks,
                "total": total,
                "projection_current": projection_current,
            })),
        ),
        Err(error) => task_projection_error_response(error),
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
    if let Ok((thread_id, _, _)) = service.get_task(&task_id).await
        && let Err(error) =
            validate_thread_runtime_allows_assignee(&state, &thread_id, &assignee).await
    {
        return task_error_response(error);
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
                let has_executor = payload["task"]
                    .get("executor")
                    .is_some_and(|value| !value.is_null());
                if !self_claim && !has_executor {
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
        Ok(task) => (StatusCode::OK, Json(json!({ "task": task }))),
        Err(error) => task_error_response(error),
    }
}

pub async fn stop_task(
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
    let (thread_id, _, _) = match service.get_task(&task_id).await {
        Ok(task) => task,
        Err(error) => return task_error_response(error),
    };
    let interrupt = crate::chat_control::execute_chat_interrupt(&state, thread_id.clone()).await;
    match service.stop_task(&task_id, actor).await {
        Ok(task) => (
            StatusCode::OK,
            Json(json!({
                "task": task,
                "thread_id": thread_id,
                "interrupted": interrupt.status == "interrupted",
                "interrupt_status": interrupt.status,
                "aborted_runs": interrupt.aborted_runs,
            })),
        ),
        Err(error) => task_error_response(error),
    }
}

pub async fn delete_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let Some(service) = task_service(&state) else {
        return tasks_disabled();
    };
    let (thread_id, _, task) = match service.get_task(&task_id).await {
        Ok(task) => task,
        Err(error) => return task_error_response(error),
    };
    let interrupt = crate::chat_control::execute_chat_interrupt(&state, thread_id.clone()).await;
    match service.delete_task(&task_id).await {
        Ok((deleted_thread_id, deleted_task)) => (
            StatusCode::OK,
            Json(json!({
                "deleted": true,
                "task_id": garyx_router::tasks::canonical_task_id(&deleted_task),
                "thread_id": deleted_thread_id,
                "task": task,
                "thread_retained": true,
                "transcripts_retained": true,
                "interrupted": interrupt.status == "interrupted",
                "interrupt_status": interrupt.status,
                "aborted_runs": interrupt.aborted_runs,
            })),
        ),
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
        Ok(task) => {
            state.invalidate_gateway_sync_caches().await;
            (StatusCode::OK, Json(json!({ "task": task })))
        }
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
            workspace_mode: value.workspace_mode,
            worktree_base_dir: None,
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
            workspace_mode: WorkspaceMode::Local,
            worktree_base_dir: None,
        });
    if input.agent_id.is_none() {
        input.agent_id = normalized_nonempty(legacy_agent_id);
    }
    if input.workspace_dir.is_none() {
        input.workspace_dir = normalized_nonempty(legacy_workspace_dir);
    }
    (input.agent_id.is_some()
        || input.workspace_dir.is_some()
        || input.workspace_mode.is_worktree())
    .then_some(input)
}

fn workflow_task_input_or_body(input: Option<Value>, task_body: Option<&str>) -> Value {
    input.unwrap_or_else(|| {
        task_body
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Value::String(value.to_owned()))
            .unwrap_or(Value::Null)
    })
}

async fn mark_task_thread_as_workflow_run(
    state: &Arc<AppState>,
    thread_id: &str,
    workflow_definition_id: &str,
    workflow_definition_version: u64,
    workspace_dir: Option<&str>,
) -> Result<(), TaskServiceError> {
    let mut record = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .ok_or_else(|| TaskServiceError::NotFound(thread_id.to_owned()))?;
    {
        let obj = record
            .as_object_mut()
            .ok_or_else(|| TaskServiceError::Store("thread record is not an object".to_owned()))?;
        obj.insert(
            "thread_kind".to_owned(),
            Value::String("workflow_run".to_owned()),
        );
        obj.insert(
            "workflow_run_id".to_owned(),
            Value::String(thread_id.to_owned()),
        );
        obj.insert(
            "workflow_definition_id".to_owned(),
            Value::String(workflow_definition_id.to_owned()),
        );
        obj.insert(
            "workflow_definition_version".to_owned(),
            Value::Number(serde_json::Number::from(workflow_definition_version)),
        );
        obj.insert(
            "workflow_status".to_owned(),
            Value::String("queued".to_owned()),
        );
        if let Some(workspace_dir) = workspace_dir
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            obj.insert(
                "workspace_dir".to_owned(),
                Value::String(workspace_dir.to_owned()),
            );
        }
        let metadata_value = obj
            .entry("metadata".to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !metadata_value.is_object() {
            *metadata_value = Value::Object(serde_json::Map::new());
        }
        if let Some(metadata) = metadata_value.as_object_mut() {
            metadata.insert("workflow_thread".to_owned(), Value::Bool(true));
            metadata.insert(
                "workflow_run_id".to_owned(),
                Value::String(thread_id.to_owned()),
            );
            metadata.insert(
                "workflow_definition_id".to_owned(),
                Value::String(workflow_definition_id.to_owned()),
            );
            metadata.insert(
                "workflow_definition_version".to_owned(),
                Value::Number(serde_json::Number::from(workflow_definition_version)),
            );
            metadata.insert(
                "workflow_status".to_owned(),
                Value::String("queued".to_owned()),
            );
        }
        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
    }
    state.threads.thread_store.set(thread_id, record).await;
    state.invalidate_gateway_sync_caches().await;
    Ok(())
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
    executor_or_assignee_agent_id: Option<&str>,
) -> Option<String> {
    executor_or_assignee_agent_id
        .map(ToOwned::to_owned)
        .and_then(|agent_id| normalized_nonempty(Some(agent_id)))
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
    executor_or_assignee_agent_id: Option<&str>,
) -> Result<Option<TaskRuntimeInput>, TaskServiceError> {
    if task_runtime_has_workspace(&runtime) {
        return Ok(runtime);
    }
    let Some(agent_id) =
        task_agent_id_for_default_workspace(&runtime, executor_or_assignee_agent_id)
    else {
        return Ok(runtime);
    };
    let Some(default_workspace_dir) = default_workspace_dir_for_agent(state, &agent_id).await?
    else {
        return Ok(runtime);
    };
    let mut input = runtime.unwrap_or(TaskRuntimeInput {
        agent_id: None,
        workspace_dir: None,
        workspace_mode: WorkspaceMode::Local,
        worktree_base_dir: None,
    });
    input.workspace_dir = Some(default_workspace_dir);
    Ok(Some(input))
}

async fn resolve_task_executor_agent(
    state: &Arc<AppState>,
    agent_id: &str,
) -> Result<AgentReference, TaskServiceError> {
    let reference = resolve_agent_reference_from_stores(
        state.ops.custom_agents.as_ref(),
        state.ops.agent_teams.as_ref(),
        agent_id,
    )
    .await
    .map_err(TaskServiceError::UnknownAgent)?;
    match reference {
        AgentReference::Standalone { .. } => Ok(reference),
        AgentReference::Team { .. } => Err(TaskServiceError::BadRequest(format!(
            "executor.type=agent requires a standalone agent; '{agent_id}' is an agent team"
        ))),
    }
}

async fn resolve_task_executor_team(
    state: &Arc<AppState>,
    team_id: &str,
) -> Result<AgentReference, TaskServiceError> {
    let reference = resolve_agent_reference_from_stores(
        state.ops.custom_agents.as_ref(),
        state.ops.agent_teams.as_ref(),
        team_id,
    )
    .await
    .map_err(TaskServiceError::UnknownAgent)?;
    match reference {
        AgentReference::Team { .. } => Ok(reference),
        AgentReference::Standalone { .. } => Err(TaskServiceError::BadRequest(format!(
            "executor.type=team requires an agent team; '{team_id}' is a standalone agent"
        ))),
    }
}

fn task_runtime_for_executor(
    runtime: Option<TaskRuntimeInput>,
    executor_agent_id: &str,
) -> Result<Option<TaskRuntimeInput>, TaskServiceError> {
    let mut runtime = runtime.unwrap_or(TaskRuntimeInput {
        agent_id: None,
        workspace_dir: None,
        workspace_mode: WorkspaceMode::Local,
        worktree_base_dir: None,
    });
    if let Some(existing_agent_id) = normalized_nonempty(runtime.agent_id.clone())
        && existing_agent_id != executor_agent_id
    {
        return Err(TaskServiceError::BadRequest(format!(
            "executor agent '{executor_agent_id}' does not match runtime agent '{existing_agent_id}'"
        )));
    }
    runtime.agent_id = Some(executor_agent_id.to_owned());
    Ok(Some(runtime))
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

pub(crate) async fn ensure_created_task_thread_provider_from_bound_agent(
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

pub(crate) fn spawn_task_auto_dispatch(
    state: Arc<AppState>,
    thread_id: String,
    task_value: Value,
    reason: &'static str,
    requested_title: Option<&str>,
    requested_body: Option<&str>,
) -> Option<Value> {
    let task: ThreadTask = serde_json::from_value(task_value).ok()?;
    let agent_id = task_dispatch_agent_id(&task)?;
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
    let dispatch_agent_id = agent_id.clone();
    tokio::spawn(async move {
        if !task_auto_dispatch_still_current(
            &state,
            &dispatch_thread_id,
            &task_id,
            &dispatch_agent_id,
        )
        .await
        {
            tracing::info!(
                task_id = %task_id,
                thread_id = %dispatch_thread_id,
                run_id = %dispatch_run_id,
                "task auto dispatch skipped because task is no longer active"
            );
            return;
        }
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

fn task_dispatch_agent_id(task: &ThreadTask) -> Option<String> {
    match task.executor.as_ref() {
        Some(TaskExecutor::Agent { agent_id }) => Some(agent_id.clone()),
        Some(TaskExecutor::Team { team_id }) => Some(team_id.clone()),
        Some(TaskExecutor::Workflow { .. }) => None,
        None => match task.assignee.as_ref() {
            Some(Principal::Agent { agent_id }) => Some(agent_id.clone()),
            _ => None,
        },
    }
}

async fn task_auto_dispatch_still_current(
    state: &Arc<AppState>,
    thread_id: &str,
    task_id: &str,
    agent_id: &str,
) -> bool {
    let Some(record) = state.threads.thread_store.get(thread_id).await else {
        return false;
    };
    let Ok(Some(task)) = garyx_router::tasks::task_from_record(&record) else {
        return false;
    };
    if garyx_router::tasks::canonical_task_id(&task) != task_id {
        return false;
    }
    if task.status != TaskStatus::InProgress {
        return false;
    }
    task_dispatch_agent_id(&task).as_deref() == Some(agent_id)
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

fn task_projection_error_response(error: GaryxDbError) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": error.to_string(),
            "code": "Internal",
        })),
    )
}

fn task_error_response(error: TaskServiceError) -> (StatusCode, Json<Value>) {
    let code = match &error {
        TaskServiceError::NotFound(_) => "NotFound",
        TaskServiceError::NotATask(_) => "NotATask",
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
        "NotATask" | "InvalidTransition" | "BadRequest" | "UnknownPrincipal" | "UnknownAgent" => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(json!({ "error": error.to_string(), "code": code })),
    )
}

fn workflow_error_as_task_response(error: WorkflowError) -> (StatusCode, Json<Value>) {
    let status = match &error {
        WorkflowError::BadRequest(_) => StatusCode::BAD_REQUEST,
        WorkflowError::NotFound(_) => StatusCode::NOT_FOUND,
        WorkflowError::Conflict(_) => StatusCode::CONFLICT,
        WorkflowError::Db(GaryxDbError::BadRequest(_)) => StatusCode::BAD_REQUEST,
        WorkflowError::Db(_) | WorkflowError::Bridge(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let code = match &error {
        WorkflowError::BadRequest(_) | WorkflowError::Db(GaryxDbError::BadRequest(_)) => {
            "BadRequest"
        }
        WorkflowError::NotFound(_) => "NotFound",
        WorkflowError::Conflict(_) => "Conflict",
        WorkflowError::Db(_) | WorkflowError::Bridge(_) => "Internal",
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
    use crate::garyx_db::{
        CURRENT_TASK_PROJECTION_VERSION, GaryxDbService, RecentThreadDraft, TASK_PROJECTION_NAME,
        TaskProjectionDraft,
    };
    use crate::server::AppStateBuilder;
    use garyx_models::ProviderType;
    use garyx_models::config::GaryxConfig;
    use std::fs;
    use tempfile::tempdir;

    fn route_task_source(thread_id: &str, task_id: &str) -> TaskSource {
        TaskSource {
            thread_id: Some(thread_id.to_owned()),
            task_id: Some(task_id.to_owned()),
            task_thread_id: Some(thread_id.to_owned()),
            bot_id: None,
            channel: None,
            account_id: None,
        }
    }

    fn route_task_projection_draft(
        thread_id: &str,
        number: u64,
        status: TaskStatus,
        updated_at: &str,
        source: Option<TaskSource>,
    ) -> TaskProjectionDraft {
        let creator = Principal::Agent {
            agent_id: "test-agent".to_owned(),
        };
        let assignee = Principal::Agent {
            agent_id: "reviewer".to_owned(),
        };
        let updated_by = creator.clone();
        let parent_task_number = source
            .as_ref()
            .and_then(|source| source.task_id.as_deref())
            .and_then(|task_id| task_id.strip_prefix("#TASK-"))
            .and_then(|number| number.parse::<u64>().ok());
        TaskProjectionDraft {
            thread_id: thread_id.to_owned(),
            number,
            status: status.as_str().to_owned(),
            title: format!("Route task {number}"),
            creator_json: serde_json::to_string(&creator).expect("creator json"),
            creator_id: creator.id().to_owned(),
            assignee_json: Some(serde_json::to_string(&assignee).expect("assignee json")),
            assignee_id: Some(assignee.id().to_owned()),
            updated_by_json: serde_json::to_string(&updated_by).expect("updated_by json"),
            executor_json: None,
            source_json: source
                .as_ref()
                .map(|source| serde_json::to_string(source).expect("source json")),
            source_thread_id: source.as_ref().and_then(|source| source.thread_id.clone()),
            source_task_thread_id: source
                .as_ref()
                .and_then(|source| source.task_thread_id.clone()),
            source_task_id: source.as_ref().and_then(|source| source.task_id.clone()),
            parent_task_number,
            source_bot_id: None,
            notification_thread_id: None,
            created_at: "2026-01-01T00:00:00.000Z".to_owned(),
            updated_at: updated_at.to_owned(),
            source_updated_at: updated_at.to_owned(),
            source_events_len: 1,
        }
    }

    async fn state_with_agent_default_workspace() -> Arc<AppState> {
        let custom_agents = Arc::new(CustomAgentStore::new());
        custom_agents
            .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
                agent_id: "reviewer".to_owned(),
                display_name: "Reviewer".to_owned(),
                provider_type: ProviderType::CodexAppServer,
                model: Some("gpt-5".to_owned()),
                model_reasoning_effort: Some(String::new()),
                model_service_tier: Some(String::new()),
                provider_env: None,
                auth_source: None,
                base_url: None,
                codex_home: None,
                max_tool_iterations: None,
                request_timeout_seconds: None,
                default_workspace_dir: Some("/tmp/agent-task-default".to_owned()),
                avatar_data_url: None,
                system_prompt: "Review carefully.".to_owned(),
            })
            .await
            .expect("custom agent");
        AppStateBuilder::new(GaryxConfig::default())
            .with_custom_agent_store(custom_agents)
            .with_agent_team_store(Arc::new(AgentTeamStore::new()))
            .build()
    }

    async fn state_with_task_executors() -> Arc<AppState> {
        let mut config = GaryxConfig::default();
        config.tasks.enabled = true;
        let custom_agents = Arc::new(CustomAgentStore::new());
        for agent_id in ["reviewer", "planner", "coder"] {
            custom_agents
                .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
                    agent_id: agent_id.to_owned(),
                    display_name: agent_id.to_owned(),
                    provider_type: ProviderType::CodexAppServer,
                    model: Some("gpt-5".to_owned()),
                    model_reasoning_effort: Some(String::new()),
                    model_service_tier: Some(String::new()),
                    provider_env: None,
                    auth_source: None,
                    base_url: None,
                    codex_home: None,
                    max_tool_iterations: None,
                    request_timeout_seconds: None,
                    default_workspace_dir: None,
                    avatar_data_url: None,
                    system_prompt: "Run the task.".to_owned(),
                })
                .await
                .expect("custom agent");
        }
        let agent_teams = Arc::new(AgentTeamStore::new());
        agent_teams
            .upsert_team(crate::agent_teams::UpsertAgentTeamRequest {
                team_id: "product-ship".to_owned(),
                display_name: "Product Ship".to_owned(),
                leader_agent_id: "planner".to_owned(),
                member_agent_ids: vec!["planner".to_owned(), "coder".to_owned()],
                workflow_text: "Coordinate the task.".to_owned(),
                avatar_data_url: None,
            })
            .await
            .expect("agent team");
        AppStateBuilder::new(config)
            .with_custom_agent_store(custom_agents)
            .with_agent_team_store(agent_teams)
            .build()
    }

    #[tokio::test]
    async fn list_task_forest_route_returns_projection_parent_and_run_state() {
        let state = state_with_task_executors().await;
        state
            .ops
            .garyx_db
            .replace_task_projection(route_task_projection_draft(
                "thread::route-parent",
                1,
                TaskStatus::InProgress,
                "2026-01-01T00:00:01.000Z",
                None,
            ))
            .expect("insert parent projection");
        state
            .ops
            .garyx_db
            .replace_task_projection(route_task_projection_draft(
                "thread::route-child",
                2,
                TaskStatus::Todo,
                "2026-01-01T00:00:02.000Z",
                Some(route_task_source("thread::route-parent", "#TASK-1")),
            ))
            .expect("insert child projection");
        state
            .ops
            .garyx_db
            .upsert_recent_thread(RecentThreadDraft {
                thread_id: "thread::route-child".to_owned(),
                title: "Route Child".to_owned(),
                workspace_dir: None,
                thread_type: "chat".to_owned(),
                provider_type: Some("claude_code".to_owned()),
                agent_id: Some("claude".to_owned()),
                message_count: 3,
                last_message_preview: "running".to_owned(),
                recent_run_id: Some("run::route-recent".to_owned()),
                active_run_id: Some("run::route-active".to_owned()),
                run_state: "running".to_owned(),
                updated_at: Some("2026-01-01T00:00:03.000Z".to_owned()),
                last_active_at: "2026-01-01T00:00:04.000Z".to_owned(),
            })
            .expect("insert route recent thread");
        state
            .ops
            .garyx_db
            .record_projection_state(TASK_PROJECTION_NAME, CURRENT_TASK_PROJECTION_VERSION, 2)
            .expect("mark projection current");

        let (status, Json(payload)) = list_task_forest(
            State(state),
            Query(TaskListQuery {
                status: None,
                assignee: None,
                creator: None,
                source_thread_id: None,
                source_task_id: None,
                source_bot_id: None,
                include_done: true,
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["total"], 2);
        assert_eq!(payload["projection_current"], true);
        let tasks = payload["tasks"].as_array().expect("tasks array");
        let child = tasks
            .iter()
            .find(|task| task["thread_id"] == "thread::route-child")
            .expect("child task");
        assert_eq!(child["parent_task_number"], 1);
        assert_eq!(child["parent_thread_id"], "thread::route-parent");
        assert_eq!(child["active_run_id"], "run::route-active");
        assert_eq!(child["run_state"], "running");
        assert_eq!(child["last_active_at"], "2026-01-01T00:00:04.000Z");
    }

    #[tokio::test]
    async fn task_runtime_uses_assignee_default_workspace_when_unset() {
        let state = state_with_agent_default_workspace().await;
        let runtime = task_runtime_with_default_workspace(&state, None, Some("reviewer"))
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
                workspace_mode: WorkspaceMode::Local,
                worktree_base_dir: None,
            }),
            Some("reviewer"),
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
        let runtime = task_runtime_with_default_workspace(&state, None, Some("claude"))
            .await
            .expect("runtime");

        assert!(runtime.is_none());
    }

    #[tokio::test]
    async fn agent_executor_creates_in_progress_task_and_dispatches_without_assignee() {
        let state = state_with_task_executors().await;

        let (status, Json(payload)) = create_task(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateTaskBody {
                title: Some("Agent executor".to_owned()),
                body: Some("Implement the slice.".to_owned()),
                assignee: None,
                notification_target: Some(TaskNotificationTargetBody::None),
                source: None,
                executor: Some(TaskExecutorBody::Agent {
                    agent_id: "reviewer".to_owned(),
                }),
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(payload["task"]["status"], "in_progress");
        assert!(payload["task"]["assignee"].is_null());
        assert_eq!(payload["task"]["executor"]["type"], "agent");
        assert_eq!(payload["task"]["executor"]["agent_id"], "reviewer");
        assert_eq!(payload["dispatch"]["queued"], true);
        assert_eq!(payload["dispatch"]["agent_id"], "reviewer");
        let thread_id = payload["thread_id"].as_str().expect("thread id");
        let stored = state
            .threads
            .thread_store
            .get(thread_id)
            .await
            .expect("stored thread");
        assert_eq!(stored["agent_id"], "reviewer");
        assert_eq!(stored["provider_type"], "codex_app_server");
    }

    #[tokio::test]
    async fn team_executor_binds_team_and_rejects_standalone_agent() {
        let state = state_with_task_executors().await;

        let (status, Json(payload)) = create_task(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateTaskBody {
                title: Some("Team executor".to_owned()),
                body: None,
                assignee: None,
                notification_target: Some(TaskNotificationTargetBody::None),
                source: None,
                executor: Some(TaskExecutorBody::Team {
                    team_id: "product-ship".to_owned(),
                }),
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(payload["task"]["status"], "in_progress");
        assert!(payload["task"]["assignee"].is_null());
        assert_eq!(payload["task"]["executor"]["type"], "team");
        assert_eq!(payload["task"]["executor"]["team_id"], "product-ship");
        assert_eq!(payload["dispatch"]["queued"], true);
        assert_eq!(payload["dispatch"]["agent_id"], "product-ship");
        let thread_id = payload["thread_id"].as_str().expect("thread id");
        let stored = state
            .threads
            .thread_store
            .get(thread_id)
            .await
            .expect("stored thread");
        assert_eq!(stored["agent_id"], "product-ship");
        assert_eq!(stored["provider_type"], "agent_team");

        let (status, Json(payload)) = create_task(
            State(state),
            HeaderMap::new(),
            Json(CreateTaskBody {
                title: Some("Bad team executor".to_owned()),
                body: None,
                assignee: None,
                notification_target: Some(TaskNotificationTargetBody::None),
                source: None,
                executor: Some(TaskExecutorBody::Team {
                    team_id: "reviewer".to_owned(),
                }),
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            payload["error"]
                .as_str()
                .unwrap()
                .contains("requires an agent team")
        );
    }

    #[tokio::test]
    async fn executor_rejects_assignee_and_team_as_agent() {
        let state = state_with_task_executors().await;

        let (status, Json(payload)) = create_task(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateTaskBody {
                title: Some("Mixed executor".to_owned()),
                body: None,
                assignee: Some(Principal::Agent {
                    agent_id: "reviewer".to_owned(),
                }),
                notification_target: Some(TaskNotificationTargetBody::None),
                source: None,
                executor: Some(TaskExecutorBody::Agent {
                    agent_id: "reviewer".to_owned(),
                }),
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            payload["error"]
                .as_str()
                .unwrap()
                .contains("cannot also set an assignee")
        );

        let (status, Json(payload)) = create_task(
            State(state),
            HeaderMap::new(),
            Json(CreateTaskBody {
                title: Some("Bad agent executor".to_owned()),
                body: None,
                assignee: None,
                notification_target: Some(TaskNotificationTargetBody::None),
                source: None,
                executor: Some(TaskExecutorBody::Agent {
                    agent_id: "product-ship".to_owned(),
                }),
                start: false,
                actor: None,
                agent_id: None,
                workspace_dir: None,
                runtime: None,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            payload["error"]
                .as_str()
                .unwrap()
                .contains("requires a standalone agent")
        );
    }

    #[tokio::test]
    async fn workflow_backed_task_creation_dispatches_workflow_entrypoint() {
        let data_dir = tempdir().expect("data dir");
        let mut config = GaryxConfig::default();
        config.tasks.enabled = true;
        config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
        let workflow_package = data_dir.path().join("workflows").join("unit");
        fs::create_dir_all(&workflow_package).expect("workflow package");
        fs::write(
            workflow_package.join("garyx.workflow.json"),
            r#"{
              "workflowId": "unit",
              "version": 4,
              "name": "Unit Workflow",
              "input": {"placeholder": "Unit request"},
              "defaults": {}
            }"#,
        )
        .expect("workflow manifest");
        fs::write(workflow_package.join("workflow.ts"), "export {};\n").expect("workflow source");
        let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
        let state = AppStateBuilder::new(config)
            .with_garyx_db(garyx_db)
            .with_custom_agent_store(Arc::new(CustomAgentStore::new()))
            .with_agent_team_store(Arc::new(AgentTeamStore::new()))
            .build();

        let task_workspace_dir = "/Users/test/workflow-task";
        let old_bun = std::env::var_os("GARYX_WORKFLOW_BUN_BIN");
        unsafe {
            std::env::set_var("GARYX_WORKFLOW_BUN_BIN", "/usr/bin/true");
        }
        let (status, Json(payload)) = create_task(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateTaskBody {
                title: Some("Run workflow".to_owned()),
                body: None,
                assignee: None,
                notification_target: Some(TaskNotificationTargetBody::None),
                source: None,
                executor: Some(TaskExecutorBody::Workflow {
                    workflow_id: "unit".to_owned(),
                    input: Some(json!({"question": "test"})),
                }),
                start: false,
                actor: None,
                agent_id: Some("claude".to_owned()),
                workspace_dir: Some(task_workspace_dir.to_owned()),
                runtime: None,
            }),
        )
        .await;
        unsafe {
            if let Some(value) = old_bun {
                std::env::set_var("GARYX_WORKFLOW_BUN_BIN", value);
            } else {
                std::env::remove_var("GARYX_WORKFLOW_BUN_BIN");
            }
        }

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(payload["dispatch"]["kind"], "workflow_entrypoint");
        assert_eq!(payload["dispatch"]["workflowId"], "unit");
        assert_eq!(payload["dispatch"]["workflowVersion"], 4);
        assert_eq!(payload["task"]["status"], "in_progress");
        assert_eq!(payload["task"]["executor"]["type"], "workflow");
        assert_eq!(payload["task"]["executor"]["workflow_id"], "unit");
        assert_eq!(payload["task"]["executor"]["workflow_version"], 4);
        let task_thread_id = payload["thread_id"].as_str().expect("thread id");
        let thread_record = state
            .threads
            .thread_store
            .get(task_thread_id)
            .await
            .expect("task thread");
        assert_eq!(thread_record["workspace_dir"], task_workspace_dir);
    }

    #[test]
    fn workflow_task_input_defaults_to_task_body() {
        assert_eq!(
            workflow_task_input_or_body(None, Some("  run this workflow  ")),
            json!("run this workflow")
        );
        assert_eq!(
            workflow_task_input_or_body(Some(json!({"explicit": true})), Some("ignored")),
            json!({"explicit": true})
        );
        assert_eq!(workflow_task_input_or_body(None, Some("   ")), Value::Null);
        assert_eq!(workflow_task_input_or_body(None, None), Value::Null);
    }
}
