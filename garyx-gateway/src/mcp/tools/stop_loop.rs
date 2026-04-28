use super::super::*;
use crate::loop_continuation::LOOP_CONTINUATION_MESSAGE;
use serde_json::Value;

fn is_pending_loop_continuation(value: &Value) -> bool {
    value
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|text| text == LOOP_CONTINUATION_MESSAGE)
}

fn apply_stop_loop_to_thread_value(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    obj.insert("loop_enabled".to_owned(), serde_json::Value::Bool(false));
    obj.insert("loop_iteration_count".to_owned(), serde_json::json!(0));
    if let Some(pending_inputs) = obj
        .get_mut("pending_user_inputs")
        .and_then(Value::as_array_mut)
    {
        pending_inputs.retain(|item| !is_pending_loop_continuation(item));
    }
}

pub(crate) async fn run(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
) -> Result<String, String> {
    let started = Instant::now();
    let run_ctx = RunContext::from_request_context(&ctx);

    let thread_id = run_ctx
        .thread_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("stop_loop requires a thread_id in MCP context")?;

    let store = &server.app_state.threads.thread_store;
    match store.get(thread_id).await {
        Some(mut value) => {
            apply_stop_loop_to_thread_value(&mut value);
            store.set(thread_id, value).await;
            server.record_tool_metric("stop_loop", "ok", started.elapsed());
            Ok(serde_json::to_string(&json!({
                "tool": "stop_loop",
                "status": "ok",
                "thread_id": thread_id,
                "message": "Loop mode disabled. The agent will not auto-continue after this run."
            }))
            .unwrap_or_default())
        }
        None => {
            server.record_tool_metric("stop_loop", "error", started.elapsed());
            Err(format!("thread not found: {thread_id}"))
        }
    }
}

#[cfg(test)]
mod tests;
