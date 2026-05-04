use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use garyx_models::local_paths::default_session_data_dir;
use garyx_router::tasks::{canonical_task_id, task_from_record};
use garyx_router::{ThreadStore, is_thread_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};
use crate::server::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingRestartWake {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub message: String,
    pub created_at: String,
}

pub fn queue_pending_restart_wake(
    kind: &str,
    target: &str,
    message: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let wake = PendingRestartWake {
        id: Uuid::new_v4().to_string(),
        kind: kind.trim().to_owned(),
        target: target.trim().to_owned(),
        message: message.to_owned(),
        created_at: Utc::now().to_rfc3339(),
    };
    let dir = pending_restart_wake_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", wake.id));
    let temp_path = dir.join(format!("{}.tmp", wake.id));
    let bytes = serde_json::to_vec_pretty(&wake)?;
    fs::write(&temp_path, bytes)?;
    fs::rename(&temp_path, &path)?;
    Ok(path)
}

pub async fn drain_pending_restart_wakes(state: Arc<AppState>) {
    let dir = pending_restart_wake_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if let Err(error) = drain_pending_restart_wake_file(state.clone(), path.clone()).await {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "failed to drain pending restart wake"
            );
            move_pending_wake_to_failed(&path);
        }
    }
}

fn move_pending_wake_to_failed(path: &Path) {
    if path.exists() {
        let _ = fs::rename(path, path.with_extension("failed.json"));
        return;
    }
    let processing_path = path.with_extension("processing.json");
    if processing_path.exists() {
        let _ = fs::rename(&processing_path, path.with_extension("failed.json"));
    }
}

async fn drain_pending_restart_wake_file(
    state: Arc<AppState>,
    path: PathBuf,
) -> Result<(), String> {
    let processing_path = path.with_extension("processing.json");
    fs::rename(&path, &processing_path).map_err(|error| error.to_string())?;
    let bytes = fs::read(&processing_path).map_err(|error| error.to_string())?;
    let wake: PendingRestartWake =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let thread_id = resolve_wake_thread_id(&state, &wake).await?;
    let mut extra_metadata = HashMap::new();
    extra_metadata.insert("restart_wake".to_owned(), Value::Bool(true));
    extra_metadata.insert("restart_wake_id".to_owned(), Value::String(wake.id.clone()));
    extra_metadata.insert(
        "restart_wake_kind".to_owned(),
        Value::String(wake.kind.clone()),
    );
    extra_metadata.insert(
        "restart_wake_target".to_owned(),
        Value::String(wake.target.clone()),
    );

    dispatch_internal_message_to_thread(
        &state,
        &thread_id,
        &format!("restart-wake-{}", wake.id),
        &wake.message,
        InternalDispatchOptions {
            extra_metadata,
            ..Default::default()
        },
    )
    .await?;
    fs::remove_file(&processing_path).map_err(|error| error.to_string())?;
    tracing::info!(
        wake_id = %wake.id,
        kind = %wake.kind,
        target = %wake.target,
        thread_id = %thread_id,
        "pending restart wake dispatched"
    );
    Ok(())
}

async fn resolve_wake_thread_id(
    state: &Arc<AppState>,
    wake: &PendingRestartWake,
) -> Result<String, String> {
    match wake.kind.as_str() {
        "thread" => {
            let target = wake.target.trim();
            if is_thread_key(target) {
                Ok(target.to_owned())
            } else {
                Err(format!(
                    "restart wake thread target must be canonical thread id: {}",
                    wake.target
                ))
            }
        }
        "task" => resolve_task_thread_id(state.threads.thread_store.clone(), &wake.target).await,
        "bot" => resolve_bot_thread_id(state, &wake.target).await,
        other => Err(format!("unknown restart wake kind: {other}")),
    }
}

async fn resolve_task_thread_id(
    store: Arc<dyn ThreadStore>,
    task_id: &str,
) -> Result<String, String> {
    for key in store.list_keys(None).await {
        if !is_thread_key(&key) {
            continue;
        }
        let Some(record) = store.get(&key).await else {
            continue;
        };
        let Ok(Some(task)) = task_from_record(&record) else {
            continue;
        };
        if canonical_task_id(&task) == task_id {
            return Ok(key);
        }
    }
    Err(format!("restart wake task target not found: {task_id}"))
}

async fn resolve_bot_thread_id(state: &Arc<AppState>, bot: &str) -> Result<String, String> {
    let Some((channel, account_id)) = bot.split_once(':') else {
        return Err(format!(
            "restart wake bot target must be channel:account_id: {bot}"
        ));
    };
    let endpoint = crate::routes::resolve_main_endpoint_by_bot(state, channel, account_id)
        .await
        .ok_or_else(|| format!("restart wake bot target has no main endpoint: {bot}"))?;
    if let Some(thread_id) = endpoint
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(thread_id.to_owned());
    }

    let mut metadata = HashMap::new();
    metadata.insert(
        "chat_id".to_owned(),
        Value::String(endpoint.chat_id.clone()),
    );
    metadata.insert(
        "display_label".to_owned(),
        Value::String(endpoint.display_label.clone()),
    );
    metadata.insert(
        "thread_binding_key".to_owned(),
        Value::String(endpoint.binding_key.clone()),
    );
    metadata.insert(
        "delivery_target_type".to_owned(),
        Value::String(endpoint.delivery_target_type.clone()),
    );
    metadata.insert(
        "delivery_target_id".to_owned(),
        Value::String(endpoint.delivery_target_id.clone()),
    );
    metadata.insert(
        "delivery_thread_id".to_owned(),
        endpoint
            .delivery_thread_id
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    let mut router = state.threads.router.lock().await;
    Ok(router
        .resolve_or_create_inbound_thread(
            &endpoint.channel,
            &endpoint.account_id,
            &endpoint.binding_key,
            &metadata,
        )
        .await)
}

fn pending_restart_wake_dir() -> PathBuf {
    default_session_data_dir().join("restart-wake")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_restart_wake_serializes_target() {
        let wake = PendingRestartWake {
            id: "wake-1".to_owned(),
            kind: "thread".to_owned(),
            target: "thread::abc".to_owned(),
            message: "continue".to_owned(),
            created_at: "2026-05-02T00:00:00Z".to_owned(),
        };
        let value = serde_json::to_value(&wake).unwrap();
        assert_eq!(value["kind"], "thread");
        assert_eq!(value["target"], "thread::abc");
    }
}
