use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};
use crate::managed_mcp_metadata::inject_managed_mcp_servers;
use crate::server::AppState;

fn parse_entry(value: &Value) -> Option<(String, String, Option<String>)> {
    let thread_id = value.get("thread_id").and_then(Value::as_str)?.trim();
    if thread_id.is_empty() {
        return None;
    }
    let message = value.get("message").and_then(Value::as_str)?.trim();
    if message.is_empty() {
        return None;
    }
    let run_id = value
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Some((thread_id.to_owned(), message.to_owned(), run_id))
}

fn build_replay_dispatch(original_run_id: Option<String>) -> (String, HashMap<String, Value>) {
    let replay_run_id = format!("restart-resume-{}", Uuid::new_v4());
    let mut metadata = HashMap::from([
        ("restart_resume".to_owned(), Value::Bool(true)),
        (
            "internal_kind".to_owned(),
            Value::String("restart_resume".to_owned()),
        ),
        (
            "restart_origin".to_owned(),
            Value::String("mcp_restart".to_owned()),
        ),
    ]);
    if let Some(original_run_id) = original_run_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        metadata.insert(
            "restart_origin_run_id".to_owned(),
            Value::String(original_run_id),
        );
    }
    (replay_run_id, metadata)
}

pub(crate) fn spawn_replay(state: Arc<AppState>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        let entries = match crate::restart::read_pending_continuations() {
            Ok(entries) => entries,
            Err(error) => {
                warn!(error = %error, "failed to read pending restart continuations");
                return;
            }
        };
        if entries.is_empty() {
            return;
        }

        info!(
            count = entries.len(),
            "replaying pending restart continuations"
        );
        let mut failed_entries = Vec::new();
        for entry in entries {
            let raw_entry = entry.clone();
            let Some((thread_id, message, original_run_id)) = parse_entry(&entry) else {
                continue;
            };
            let (run_id, mut metadata) = build_replay_dispatch(original_run_id.clone());
            inject_managed_mcp_servers(&state.config_snapshot().mcp_servers, &mut metadata);

            if let Err(error) = dispatch_internal_message_to_thread(
                &state,
                &thread_id,
                &run_id,
                &message,
                InternalDispatchOptions {
                    extra_metadata: metadata,
                    ..Default::default()
                },
            )
            .await
            {
                warn!(
                    thread_id = %thread_id,
                    run_id = %run_id,
                    original_run_id = ?original_run_id,
                    error = %error,
                    "failed to replay pending restart continuation"
                );
                failed_entries.push(raw_entry);
            } else {
                info!(
                    thread_id = %thread_id,
                    run_id = %run_id,
                    original_run_id = ?original_run_id,
                    "replayed pending restart continuation"
                );
            }
        }

        if failed_entries.is_empty() {
            if let Err(error) = crate::restart::clear_pending_continuations() {
                warn!(error = %error, "failed to clear pending restart continuation queue");
            } else {
                info!("cleared pending restart continuation queue");
            }
        } else if let Err(error) = crate::restart::write_pending_continuations(&failed_entries) {
            warn!(
                error = %error,
                remaining = failed_entries.len(),
                "failed to persist pending restart continuation retries"
            );
        } else {
            warn!(
                remaining = failed_entries.len(),
                "retained failed restart continuations for retry"
            );
        }
    });
}

#[cfg(test)]
mod tests;
