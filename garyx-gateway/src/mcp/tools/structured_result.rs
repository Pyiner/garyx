use super::super::*;
use rmcp::model::{CallToolResult, JsonObject};
use serde_json::json;

pub(crate) async fn run(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    arguments: Option<JsonObject>,
) -> Result<CallToolResult, String> {
    let run_ctx = RunContext::from_request_context(&ctx);
    let thread_id = run_ctx
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "submit_result requires the current MCP request to carry a thread id".to_owned()
        })?;
    let payload = Value::Object(arguments.unwrap_or_default());

    let submitted = crate::workflows::submit_structured_result_for_thread(
        &server.app_state,
        thread_id,
        payload,
    )
    .await
    .map_err(|error| error.to_string())?;

    Ok(CallToolResult::structured(json!({
        "tool": "submit_result",
        "status": "ok",
        "workflowRunId": submitted.workflow_id,
        "workflowChildRunId": submitted.workflow_child_run_id,
        "threadId": submitted.thread_id,
    })))
}
