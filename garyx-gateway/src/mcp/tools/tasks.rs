use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use garyx_models::local_paths::default_session_data_dir;
use garyx_models::{Principal, TaskScope, TaskStatus};
use garyx_router::{
    CreateTaskInput, FileTaskCounterStore, PromoteTaskInput, TaskListFilter, TaskService,
    UpdateTaskStatusInput,
};
use serde_json::{Value, json};

use super::super::*;

pub(crate) async fn create(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: TaskCreateParams,
) -> Result<String, String> {
    record_task_tool(server, "task_create", async {
        let service = task_service(server)?;
        let run_ctx = RunContext::from_request_context(&ctx);
        let actor = actor_from_context(server, &run_ctx).await?;
        let scope = task_scope(params.scope);
        let agent_id = normalized_nonempty(params.agent_id);
        let mut created = Vec::new();

        if let Some(items) = params.tasks {
            if items.is_empty() {
                return Err("tasks cannot be empty".to_owned());
            }
            for item in items {
                let (thread_id, task) = service
                    .create_task(CreateTaskInput {
                        scope: scope.clone(),
                        title: item.title,
                        body: item.body,
                        assignee: item.assignee.map(principal_from_input).transpose()?,
                        start: item.start,
                        actor: Some(actor.clone()),
                        agent_id: agent_id.clone(),
                    })
                    .await
                    .map_err(|error| error.to_string())?;
                created.push(task_result(thread_id, task));
            }
        } else {
            let (thread_id, task) = service
                .create_task(CreateTaskInput {
                    scope,
                    title: params.title,
                    body: params.body,
                    assignee: params.assignee.map(principal_from_input).transpose()?,
                    start: params.start,
                    actor: Some(actor),
                    agent_id,
                })
                .await
                .map_err(|error| error.to_string())?;
            created.push(task_result(thread_id, task));
        }

        Ok(json!({
            "tool": "task_create",
            "status": "ok",
            "tasks": created,
        }))
    })
    .await
}

pub(crate) async fn promote(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: TaskPromoteParams,
) -> Result<String, String> {
    record_task_tool(server, "task_promote", async {
        let service = task_service(server)?;
        let run_ctx = RunContext::from_request_context(&ctx);
        let actor = actor_from_context(server, &run_ctx).await?;
        let task = service
            .promote_task(PromoteTaskInput {
                thread_id: params.thread_id,
                title: params.title,
                assignee: params.assignee.map(principal_from_input).transpose()?,
                actor: Some(actor),
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "tool": "task_promote",
            "status": "ok",
            "task_ref": garyx_router::tasks::canonical_task_ref(&task),
            "number": task.number,
            "task": task,
        }))
    })
    .await
}

pub(crate) async fn get(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: TaskGetParams,
) -> Result<String, String> {
    record_task_tool(server, "task_get", async {
        let service = task_service(server)?;
        let run_ctx = RunContext::from_request_context(&ctx);
        let task_ref = task_ref_from(run_ctx.thread_id, params.task_ref, params.thread_id)?;
        let (thread_id, thread, task) = service
            .get_task(&task_ref, None)
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "tool": "task_get",
            "status": "ok",
            "thread_id": thread_id,
            "task_ref": garyx_router::tasks::canonical_task_ref(&task),
            "task": task,
            "thread": thread,
        }))
    })
    .await
}

pub(crate) async fn list(server: &GaryMcpServer, params: TaskListParams) -> Result<String, String> {
    record_task_tool(server, "task_list", async {
        let service = task_service(server)?;
        let (tasks, total, has_more) = service
            .list_tasks(TaskListFilter {
                scope: Some(task_scope(params.scope)),
                status: params.status.as_deref().map(parse_status).transpose()?,
                assignee: params.assignee.map(principal_from_input).transpose()?,
                creator: params.creator.map(principal_from_input).transpose()?,
                include_done: params.include_done,
                limit: params.limit,
                offset: params.offset,
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "tool": "task_list",
            "status": "ok",
            "tasks": tasks,
            "total": total,
            "has_more": has_more,
        }))
    })
    .await
}

pub(crate) async fn history(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: TaskHistoryParams,
) -> Result<String, String> {
    record_task_tool(server, "task_history", async {
        let service = task_service(server)?;
        let run_ctx = RunContext::from_request_context(&ctx);
        let task_ref = task_ref_from(run_ctx.thread_id, params.task_ref, params.thread_id)?;
        let events = service
            .task_history(&task_ref, None, params.limit)
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "tool": "task_history",
            "status": "ok",
            "events": events,
        }))
    })
    .await
}

pub(crate) async fn assign(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: TaskAssignParams,
) -> Result<String, String> {
    record_task_tool(server, "task_assign", async {
        let service = task_service(server)?;
        let run_ctx = RunContext::from_request_context(&ctx);
        let actor = actor_from_context(server, &run_ctx).await?;
        let task_ref = task_ref_from(run_ctx.thread_id.clone(), params.task_ref, params.thread_id)?;
        let task = service
            .assign_task(
                &task_ref,
                principal_from_input(params.to)?,
                Some(actor),
                None,
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "tool": "task_assign",
            "status": "ok",
            "task": task,
        }))
    })
    .await
}

pub(crate) async fn unassign(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: TaskGetParams,
) -> Result<String, String> {
    record_task_tool(server, "task_unassign", async {
        let service = task_service(server)?;
        let run_ctx = RunContext::from_request_context(&ctx);
        let actor = actor_from_context(server, &run_ctx).await?;
        let task_ref = task_ref_from(run_ctx.thread_id.clone(), params.task_ref, params.thread_id)?;
        let task = service
            .unassign_task(&task_ref, Some(actor), None)
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "tool": "task_unassign",
            "status": "ok",
            "task": task,
        }))
    })
    .await
}

pub(crate) async fn update_status(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: TaskUpdateStatusParams,
) -> Result<String, String> {
    record_task_tool(server, "task_update_status", async {
        let service = task_service(server)?;
        let run_ctx = RunContext::from_request_context(&ctx);
        let actor = actor_from_context(server, &run_ctx).await?;
        let task_ref = task_ref_from(run_ctx.thread_id.clone(), params.task_ref, params.thread_id)?;
        let task = service
            .update_status(
                UpdateTaskStatusInput {
                    task_ref,
                    to: parse_status(&params.to)?,
                    note: params.note,
                    force: params.force,
                    actor: Some(actor),
                },
                None,
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "tool": "task_update_status",
            "status": "ok",
            "task": task,
        }))
    })
    .await
}

pub(crate) async fn set_title(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: TaskSetTitleParams,
) -> Result<String, String> {
    record_task_tool(server, "task_set_title", async {
        let service = task_service(server)?;
        let run_ctx = RunContext::from_request_context(&ctx);
        let actor = actor_from_context(server, &run_ctx).await?;
        let task_ref = task_ref_from(run_ctx.thread_id.clone(), params.task_ref, params.thread_id)?;
        let task = service
            .set_title(&task_ref, params.title, Some(actor), None)
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "tool": "task_set_title",
            "status": "ok",
            "task": task,
        }))
    })
    .await
}

async fn record_task_tool<F>(
    server: &GaryMcpServer,
    tool: &'static str,
    future: F,
) -> Result<String, String>
where
    F: std::future::Future<Output = Result<Value, String>>,
{
    let started = Instant::now();
    let result = future
        .await
        .map(|payload| serde_json::to_string(&payload).unwrap_or_default());
    server.record_tool_metric(
        tool,
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result
}

fn task_service(server: &GaryMcpServer) -> Result<TaskService, String> {
    let config = server.app_state.config_snapshot();
    if !config.tasks.enabled {
        return Err("tasks are disabled".to_owned());
    }
    let data_dir = config
        .sessions
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(default_session_data_dir);
    Ok(TaskService::new(
        server.app_state.threads.thread_store.clone(),
        Arc::new(FileTaskCounterStore::new(data_dir)),
    ))
}

async fn actor_from_context(
    server: &GaryMcpServer,
    run_ctx: &RunContext,
) -> Result<Principal, String> {
    let thread_id = run_ctx
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "task mutation requires a thread_id in MCP context".to_owned())?;
    let record = server
        .app_state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .ok_or_else(|| format!("thread not found: {thread_id}"))?;
    let agent_id = record
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("thread {thread_id} has no agent_id"))?;
    Ok(Principal::Agent {
        agent_id: agent_id.to_owned(),
    })
}

fn task_scope(scope: McpTaskScope) -> TaskScope {
    TaskScope::new(scope.channel, scope.account_id)
}

fn principal_from_input(input: McpPrincipalInput) -> Result<Principal, String> {
    match input {
        McpPrincipalInput::Principal(McpPrincipal::Human { user_id }) => Ok(Principal::Human {
            user_id: user_id.trim().to_owned(),
        }),
        McpPrincipalInput::Principal(McpPrincipal::Agent { agent_id }) => Ok(Principal::Agent {
            agent_id: agent_id.trim().to_owned(),
        }),
        McpPrincipalInput::String(value) => principal_from_string(&value),
    }
}

fn principal_from_string(value: &str) -> Result<Principal, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("principal cannot be empty".to_owned());
    }
    if let Some(user_id) = value.strip_prefix("human:") {
        return Ok(Principal::Human {
            user_id: user_id.trim().to_owned(),
        });
    }
    if let Some(agent_id) = value
        .strip_prefix("agent:")
        .or_else(|| value.strip_prefix('@'))
    {
        return Ok(Principal::Agent {
            agent_id: agent_id.trim().to_owned(),
        });
    }
    Ok(Principal::Agent {
        agent_id: value.to_owned(),
    })
}

fn parse_status(value: &str) -> Result<TaskStatus, String> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "todo" | "to_do" | "open" => Ok(TaskStatus::Todo),
        "in_progress" | "progress" | "doing" | "claimed" => Ok(TaskStatus::InProgress),
        "in_review" | "review" | "reviewing" => Ok(TaskStatus::InReview),
        "done" | "complete" | "completed" | "closed" => Ok(TaskStatus::Done),
        _ => Err(format!("unknown task status: {value}")),
    }
}

fn task_ref_from(
    context_thread_id: Option<String>,
    task_ref: Option<String>,
    thread_id: Option<String>,
) -> Result<String, String> {
    task_ref
        .or(thread_id)
        .or(context_thread_id)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "task_ref or thread_id is required".to_owned())
}

fn normalized_nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn task_result(thread_id: String, task: garyx_models::ThreadTask) -> Value {
    json!({
        "thread_id": thread_id,
        "task_ref": garyx_router::tasks::canonical_task_ref(&task),
        "number": task.number,
        "status": task.status,
        "task": task,
    })
}
