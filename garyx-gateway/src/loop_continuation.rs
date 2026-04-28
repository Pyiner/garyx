use std::collections::HashMap;
use std::sync::Arc;

use garyx_router::loop_enabled_from_value;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};
use crate::managed_mcp_metadata::inject_managed_mcp_servers;
use crate::server::AppState;

pub(crate) const LOOP_CONTINUATION_MESSAGE: &str = "The user wants you to continue working on any remaining tasks. If all tasks are complete, call the stop_loop tool to exit loop mode.";

pub(crate) fn spawn_listener(state: Arc<AppState>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };

    let mut rx = state.ops.events.subscribe();
    handle.spawn(async move {
        loop {
            match rx.recv().await {
                Ok(raw_event) => {
                    let Ok(payload) = serde_json::from_str::<Value>(&raw_event) else {
                        continue;
                    };
                    let Some((thread_id, iteration)) = parse_loop_continue_event(&payload) else {
                        continue;
                    };
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(error) =
                            dispatch_loop_continuation(&state, &thread_id, iteration).await
                        {
                            warn!(
                                thread_id = %thread_id,
                                iteration,
                                error = %error,
                                "failed to dispatch loop continuation"
                            );
                        }
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn parse_loop_continue_event(payload: &Value) -> Option<(String, u64)> {
    let event_type = payload.get("type").and_then(Value::as_str)?;
    if event_type != "loop_continue" {
        return None;
    }
    let thread_id = payload.get("thread_id").and_then(Value::as_str)?.trim();
    if thread_id.is_empty() {
        return None;
    }
    let iteration = payload
        .get("iteration")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some((thread_id.to_owned(), iteration))
}

pub(crate) async fn dispatch_loop_continuation(
    state: &Arc<AppState>,
    thread_id: &str,
    iteration: u64,
) -> Result<(), String> {
    let Some(thread) = state.threads.thread_store.get(thread_id).await else {
        return Err(format!("thread not found: {thread_id}"));
    };
    if !loop_enabled_from_value(&thread) {
        info!(
            thread_id = %thread_id,
            iteration,
            "skipping loop continuation because loop mode is disabled"
        );
        return Ok(());
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let mut metadata = HashMap::from([
        ("loop_iteration".to_owned(), json!(iteration)),
        ("loop_continuation".to_owned(), Value::Bool(true)),
        (
            "loop_origin".to_owned(),
            Value::String("auto_continue".to_owned()),
        ),
        (
            "internal_kind".to_owned(),
            Value::String("loop_continuation".to_owned()),
        ),
    ]);
    inject_managed_mcp_servers(&state.config_snapshot().mcp_servers, &mut metadata);

    info!(
        thread_id = %thread_id,
        run_id = %run_id,
        iteration,
        "dispatching loop continuation via internal thread message"
    );

    dispatch_internal_message_to_thread(
        state,
        thread_id,
        &run_id,
        LOOP_CONTINUATION_MESSAGE,
        InternalDispatchOptions {
            extra_metadata: metadata,
            ..Default::default()
        },
    )
    .await
}

#[cfg(test)]
mod tests;
