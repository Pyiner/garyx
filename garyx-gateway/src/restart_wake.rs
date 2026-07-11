use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::Utc;
use garyx_models::local_paths::{default_garyx_database_path, default_session_data_dir};
use garyx_router::is_thread_key;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::garyx_db::{GaryxDbService, RecentThreadRecord};
use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};
use crate::server::AppState;

const WAKE_ALL_KIND: &str = "all";
const WAKE_ALL_TARGET: &str = "all";
/// Default message injected when a restart wakes a thread and the caller did not
/// pass an explicit `--wake-message`. Wrapped in the `garyx_restarted` tag so
/// clients render it as a restart-notice card (mirrors the task-notification
/// card); an agent receiving it should simply continue its interrupted work.
pub const RESTART_WAKE_DEFAULT_MESSAGE: &str =
    "<garyx_restarted>Garyx has restarted. Continue your task.</garyx_restarted>";
const MAX_RESTART_WAKE_ALL_THREADS: usize = 16;
const MAX_RESTART_WAKE_ALL_ATTEMPTS: u32 = 8;
const STALE_PROCESSING_WAKE_AGE: Duration = Duration::from_secs(120);
const OVERLOADED_ERROR_PREFIX: &str = "bridge overloaded:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingRestartWake {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub message: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_ordinals: Vec<usize>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<PendingRestartWakeError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingRestartWakeError {
    pub target: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedRestartWakeAll {
    pub path: PathBuf,
    pub targets: Vec<String>,
    pub truncated_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RestartWakeAllSnapshot {
    targets: Vec<String>,
    truncated_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RestartWakeDispatchError {
    RetryableOverload(String),
    Other(String),
}

fn is_zero(value: &u32) -> bool {
    *value == 0
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
        targets: Vec::new(),
        target_ordinals: Vec::new(),
        attempt: 0,
        errors: Vec::new(),
    };
    let dir = pending_restart_wake_dir();
    write_pending_restart_wake(&dir, &wake)
}

pub fn queue_pending_restart_wake_all(
    message: &str,
) -> Result<QueuedRestartWakeAll, Box<dyn std::error::Error>> {
    let garyx_db = GaryxDbService::open(default_garyx_database_path())?;
    let snapshot = restart_wake_all_snapshot_from_db(&garyx_db)?;
    queue_pending_restart_wake_all_targets(message, snapshot)
}

fn queue_pending_restart_wake_all_targets(
    message: &str,
    snapshot: RestartWakeAllSnapshot,
) -> Result<QueuedRestartWakeAll, Box<dyn std::error::Error>> {
    let wake = PendingRestartWake {
        id: Uuid::new_v4().to_string(),
        kind: WAKE_ALL_KIND.to_owned(),
        target: WAKE_ALL_TARGET.to_owned(),
        message: message.to_owned(),
        created_at: Utc::now().to_rfc3339(),
        target_ordinals: (0..snapshot.targets.len()).collect(),
        targets: snapshot.targets.clone(),
        attempt: 0,
        errors: Vec::new(),
    };
    let dir = pending_restart_wake_dir();
    let path = write_pending_restart_wake(&dir, &wake)?;
    Ok(QueuedRestartWakeAll {
        path,
        targets: snapshot.targets,
        truncated_count: snapshot.truncated_count,
    })
}

fn write_pending_restart_wake(
    dir: &Path,
    wake: &PendingRestartWake,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", wake.id));
    write_pending_restart_wake_path(&path, wake).map_err(std::io::Error::other)?;
    Ok(path)
}

pub async fn drain_pending_restart_wakes(state: Arc<AppState>) {
    let dir = pending_restart_wake_dir();
    drain_pending_restart_wakes_from_dir(state, dir).await;
}

async fn drain_pending_restart_wakes_from_dir(state: Arc<AppState>, dir: PathBuf) {
    recover_stale_processing_wakes(&dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_fresh_pending_wake_file(&path) {
            continue;
        }
        if let Err(error) =
            drain_pending_restart_wake_file(state.clone(), dir.clone(), path.clone()).await
        {
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
    dir: PathBuf,
    path: PathBuf,
) -> Result<(), String> {
    let processing_path = path.with_extension("processing.json");
    if let Err(error) = fs::rename(&path, &processing_path) {
        if error.kind() == std::io::ErrorKind::NotFound {
            tracing::debug!(
                path = %path.display(),
                "pending restart wake was already claimed by another drain"
            );
            return Ok(());
        }
        return Err(error.to_string());
    }
    let bytes = fs::read(&processing_path).map_err(|error| error.to_string())?;
    let wake: PendingRestartWake =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if wake.kind == WAKE_ALL_KIND {
        return drain_pending_restart_wake_all_file(state, dir, path, processing_path, wake).await;
    }
    let thread_id = resolve_wake_thread_id(&state, &wake).await?;

    dispatch_restart_wake_to_thread(
        &state,
        &thread_id,
        &format!("restart-wake-{}", wake.id),
        &wake.message,
        restart_wake_metadata(&wake, &wake.target),
    )
    .await
    .map_err(|error| error.into_message())?;
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

async fn drain_pending_restart_wake_all_file(
    state: Arc<AppState>,
    dir: PathBuf,
    path: PathBuf,
    processing_path: PathBuf,
    wake: PendingRestartWake,
) -> Result<(), String> {
    let (targets, ordinals) = normalized_wake_all_targets(&wake);
    if targets.is_empty() {
        fs::remove_file(&processing_path).map_err(|error| error.to_string())?;
        tracing::info!(wake_id = %wake.id, "empty restart wake-all drained");
        return Ok(());
    }

    persist_wake_all_remaining(
        &processing_path,
        &wake,
        &targets,
        &ordinals,
        0,
        wake.attempt,
    )?;

    let mut failures = Vec::<PendingRestartWakeError>::new();
    for index in 0..targets.len() {
        let thread_id = &targets[index];
        let ordinal = ordinals[index];
        let run_id = format!("restart-wake-{}-{}", wake.id, ordinal);
        match dispatch_restart_wake_to_thread(
            &state,
            thread_id,
            &run_id,
            &wake.message,
            restart_wake_metadata(&wake, thread_id),
        )
        .await
        {
            Ok(()) => {
                persist_wake_all_remaining(
                    &processing_path,
                    &wake,
                    &targets,
                    &ordinals,
                    index + 1,
                    wake.attempt,
                )?;
                tracing::info!(
                    wake_id = %wake.id,
                    thread_id = %thread_id,
                    run_id = %run_id,
                    "restart wake-all target dispatched"
                );
            }
            Err(RestartWakeDispatchError::RetryableOverload(error)) => {
                write_wake_all_failures(&path, &wake, &failures)?;
                if wake.attempt >= MAX_RESTART_WAKE_ALL_ATTEMPTS {
                    let mut retry_failures = failures;
                    retry_failures.extend(targets[index..].iter().map(|target| {
                        PendingRestartWakeError {
                            target: target.clone(),
                            error: format!(
                                "restart wake-all retry limit reached after overload: {error}"
                            ),
                        }
                    }));
                    write_wake_all_failures(&path, &wake, &retry_failures)?;
                    fs::remove_file(&processing_path).map_err(|error| error.to_string())?;
                    return Ok(());
                }

                let next_attempt = wake.attempt + 1;
                persist_wake_all_remaining(
                    &processing_path,
                    &wake,
                    &targets,
                    &ordinals,
                    index,
                    next_attempt,
                )?;
                fs::rename(&processing_path, &path).map_err(|error| error.to_string())?;
                schedule_delayed_restart_wake_drain(state, dir, next_attempt);
                tracing::warn!(
                    wake_id = %wake.id,
                    thread_id = %thread_id,
                    attempt = next_attempt,
                    error = %error,
                    "restart wake-all overloaded; requeued remaining targets"
                );
                return Ok(());
            }
            Err(RestartWakeDispatchError::Other(error)) => {
                failures.push(PendingRestartWakeError {
                    target: thread_id.clone(),
                    error,
                });
                persist_wake_all_remaining(
                    &processing_path,
                    &wake,
                    &targets,
                    &ordinals,
                    index + 1,
                    wake.attempt,
                )?;
            }
        }
    }

    write_wake_all_failures(&path, &wake, &failures)?;
    fs::remove_file(&processing_path).map_err(|error| error.to_string())?;
    tracing::info!(
        wake_id = %wake.id,
        target_count = targets.len(),
        failure_count = failures.len(),
        "restart wake-all drained"
    );
    Ok(())
}

fn restart_wake_all_snapshot_from_db(
    garyx_db: &GaryxDbService,
) -> Result<RestartWakeAllSnapshot, crate::garyx_db::GaryxDbError> {
    let records = garyx_db.list_recent_threads(usize::MAX, 0)?;
    Ok(restart_wake_all_snapshot_from_records(&records))
}

fn restart_wake_all_snapshot_from_records(
    records: &[RecentThreadRecord],
) -> RestartWakeAllSnapshot {
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for record in records {
        if !is_restart_wake_all_candidate(record) {
            continue;
        }
        if seen.insert(record.thread_id.clone()) {
            targets.push(record.thread_id.clone());
        }
    }
    let truncated_count = targets.len().saturating_sub(MAX_RESTART_WAKE_ALL_THREADS);
    targets.truncate(MAX_RESTART_WAKE_ALL_THREADS);
    RestartWakeAllSnapshot {
        targets,
        truncated_count,
    }
}

fn is_restart_wake_all_candidate(record: &RecentThreadRecord) -> bool {
    if !is_thread_key(&record.thread_id) {
        return false;
    }
    record.run_state == "running"
        || record
            .active_run_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

fn restart_wake_metadata(wake: &PendingRestartWake, target: &str) -> HashMap<String, Value> {
    let mut extra_metadata = HashMap::new();
    extra_metadata.insert("restart_wake".to_owned(), Value::Bool(true));
    extra_metadata.insert("restart_wake_id".to_owned(), Value::String(wake.id.clone()));
    extra_metadata.insert(
        "restart_wake_kind".to_owned(),
        Value::String(wake.kind.clone()),
    );
    extra_metadata.insert(
        "restart_wake_target".to_owned(),
        Value::String(target.to_owned()),
    );
    if wake.kind == WAKE_ALL_KIND {
        extra_metadata.insert("restart_wake_all".to_owned(), Value::Bool(true));
        extra_metadata.insert(
            "restart_wake_attempt".to_owned(),
            Value::Number(serde_json::Number::from(wake.attempt)),
        );
    }
    extra_metadata
}

async fn dispatch_restart_wake_to_thread(
    state: &Arc<AppState>,
    thread_id: &str,
    run_id: &str,
    message: &str,
    extra_metadata: HashMap<String, Value>,
) -> Result<(), RestartWakeDispatchError> {
    dispatch_internal_message_to_thread(
        state,
        thread_id,
        run_id,
        message,
        InternalDispatchOptions {
            extra_metadata,
            ..Default::default()
        },
    )
    .await
    .map(|_outcome| ())
    .map_err(RestartWakeDispatchError::from_dispatch_error)
}

impl RestartWakeDispatchError {
    fn from_dispatch_error(error: String) -> Self {
        if error.trim_start().starts_with(OVERLOADED_ERROR_PREFIX) {
            RestartWakeDispatchError::RetryableOverload(error)
        } else {
            RestartWakeDispatchError::Other(error)
        }
    }

    fn into_message(self) -> String {
        match self {
            RestartWakeDispatchError::RetryableOverload(error)
            | RestartWakeDispatchError::Other(error) => error,
        }
    }
}

fn normalized_wake_all_targets(wake: &PendingRestartWake) -> (Vec<String>, Vec<usize>) {
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    let mut ordinals = Vec::new();
    for (index, target) in wake.targets.iter().enumerate() {
        let target = target.trim();
        if !is_thread_key(target) || !seen.insert(target.to_owned()) {
            continue;
        }
        targets.push(target.to_owned());
        ordinals.push(wake.target_ordinals.get(index).copied().unwrap_or(index));
    }
    (targets, ordinals)
}

fn persist_wake_all_remaining(
    processing_path: &Path,
    wake: &PendingRestartWake,
    targets: &[String],
    ordinals: &[usize],
    start: usize,
    attempt: u32,
) -> Result<(), String> {
    let remaining_wake =
        wake_for_remaining_targets(wake, &targets[start..], &ordinals[start..], attempt);
    write_pending_restart_wake_path(processing_path, &remaining_wake)
}

fn wake_for_remaining_targets(
    wake: &PendingRestartWake,
    targets: &[String],
    ordinals: &[usize],
    attempt: u32,
) -> PendingRestartWake {
    PendingRestartWake {
        id: wake.id.clone(),
        kind: wake.kind.clone(),
        target: wake.target.clone(),
        message: wake.message.clone(),
        created_at: wake.created_at.clone(),
        targets: targets.to_vec(),
        target_ordinals: ordinals.to_vec(),
        attempt,
        errors: Vec::new(),
    }
}

fn write_wake_all_failures(
    original_path: &Path,
    wake: &PendingRestartWake,
    failures: &[PendingRestartWakeError],
) -> Result<(), String> {
    if failures.is_empty() {
        return Ok(());
    }
    let failed_path = original_path.with_extension("failed.json");
    let failed_wake = PendingRestartWake {
        id: wake.id.clone(),
        kind: wake.kind.clone(),
        target: wake.target.clone(),
        message: wake.message.clone(),
        created_at: wake.created_at.clone(),
        targets: failures
            .iter()
            .map(|failure| failure.target.clone())
            .collect(),
        target_ordinals: Vec::new(),
        attempt: wake.attempt,
        errors: failures.to_vec(),
    };
    write_pending_restart_wake_path(&failed_path, &failed_wake)
}

fn schedule_delayed_restart_wake_drain(state: Arc<AppState>, dir: PathBuf, attempt: u32) {
    let delay = restart_wake_retry_delay(attempt);
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        drain_pending_restart_wakes_from_dir(state, dir).await;
    });
}

fn restart_wake_retry_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(4);
    Duration::from_secs(5 * (1u64 << exponent))
}

fn write_pending_restart_wake_path(path: &Path, wake: &PendingRestartWake) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temp_path = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(wake).map_err(|error| error.to_string())?;
    fs::write(&temp_path, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temp_path, path).map_err(|error| error.to_string())
}

fn recover_stale_processing_wakes(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_processing_wake_file(&path) {
            continue;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default()
            < STALE_PROCESSING_WAKE_AGE
        {
            continue;
        }
        let Some(fresh_path) = fresh_path_for_processing_wake(&path) else {
            continue;
        };
        if fresh_path.exists() {
            continue;
        }
        if let Err(error) = fs::rename(&path, &fresh_path) {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "failed to recover stale processing restart wake"
            );
        }
    }
}

fn is_fresh_pending_wake_file(path: &Path) -> bool {
    if path.extension().and_then(|value| value.to_str()) != Some("json") {
        return false;
    }
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    !stem.contains('.')
}

fn is_processing_wake_file(path: &Path) -> bool {
    path.extension().and_then(|value| value.to_str()) == Some("json")
        && path
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|stem| stem.ends_with(".processing"))
}

fn fresh_path_for_processing_wake(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let fresh_stem = stem.strip_suffix(".processing")?;
    Some(path.with_file_name(format!("{fresh_stem}.json")))
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
        "task" => resolve_task_thread_id(state, &wake.target).await,
        "bot" => resolve_bot_thread_id(state, &wake.target).await,
        other => Err(format!("unknown restart wake kind: {other}")),
    }
}

async fn resolve_task_thread_id(state: &Arc<AppState>, task_id: &str) -> Result<String, String> {
    // Resolve through the task projection (indexed by task number) instead
    // of enumerating every thread record: the old full scan read multi-MB
    // thread files for the whole store right on the restart path.
    let service = crate::tasks::task_service(state);
    match service.get_task(task_id).await {
        Ok((thread_id, _record, _task)) => Ok(thread_id),
        Err(garyx_router::tasks::TaskServiceError::NotFound(_)) => {
            // Projections derive in the same transaction as every record
            // write (#TASK-1864): a missing row means the task genuinely
            // does not exist — the former forced repair walk is retired.
            Err(format!("restart wake task target not found: {task_id}"))
        }
        Err(error) => Err(format!(
            "restart wake task target not found: {task_id} ({error})"
        )),
    }
}

async fn resolve_bot_thread_id(state: &Arc<AppState>, bot: &str) -> Result<String, String> {
    let Some((channel, account_id)) = bot.split_once(':') else {
        return Err(format!(
            "restart wake bot target must be channel:account_id: {bot}"
        ));
    };
    let endpoint = crate::routes::resolve_main_endpoint_by_bot(state, channel, account_id)
        .await
        .map_err(|error| {
            format!("thread store error resolving restart wake bot target {bot}: {error}")
        })?
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
    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;
    use garyx_bridge::MultiProviderBridge;
    use garyx_bridge::provider_trait::{ProviderRuntime, BridgeError, StreamCallback};
    use garyx_models::config::GaryxConfig;
    use garyx_models::provider::{
        ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent,
    };
    use serde_json::json;
    use tokio::sync::Notify;

    type ProviderCall = (String, String, HashMap<String, Value>);

    #[derive(Default)]
    struct RecordingProvider {
        calls: StdMutex<Vec<ProviderCall>>,
        block_runs: bool,
        release: Notify,
    }

    #[async_trait]
    impl ProviderRuntime for RecordingProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::ClaudeCode
        }

        fn is_ready(&self) -> bool {
            true
        }

        async fn initialize(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn run_streaming(
            &self,
            options: &ProviderRunOptions,
            on_chunk: StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            self.calls.lock().unwrap().push((
                options.thread_id.clone(),
                options.message.clone(),
                options.metadata.clone(),
            ));
            if self.block_runs {
                self.release.notified().await;
            }
            on_chunk(StreamEvent::Delta {
                text: "ok".to_owned(),
            });
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "recording-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "ok".to_owned(),
                session_messages: vec![],
                sdk_session_id: None,
                actual_model: None,
                thread_title: None,
                success: true,
                error: None,
                input_tokens: 0,
                output_tokens: 0,
                cost: 0.0,
                duration_ms: 0,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(session_key.to_owned())
        }
    }

    async fn test_state(
        max_concurrent_runs: usize,
        block_runs: bool,
    ) -> (Arc<AppState>, Arc<RecordingProvider>) {
        let bridge = Arc::new(MultiProviderBridge::new_with_max_concurrent_runs(
            max_concurrent_runs,
        ));
        let provider = Arc::new(RecordingProvider {
            calls: StdMutex::new(Vec::new()),
            block_runs,
            release: Notify::new(),
        });
        bridge
            .register_provider("test-provider", provider.clone())
            .await;
        bridge.set_default_provider_key("test-provider").await;
        let state =
            crate::server::create_app_state_with_bridge(GaryxConfig::default(), bridge.clone());
        bridge
            .set_thread_store(state.threads.thread_store.clone())
            .await;
        bridge.set_event_tx(state.ops.events.sender()).await;
        (state, provider)
    }

    async fn seed_thread(state: &Arc<AppState>, thread_id: &str) {
        state
            .threads
            .thread_store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "channel": "api",
                    "account_id": "main",
                    "from_id": "loop",
                    "messages": []
                }),
            )
            .await
            .unwrap();
    }

    async fn wait_for_calls(provider: &RecordingProvider, expected: usize) -> Vec<ProviderCall> {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let calls = provider.calls.lock().unwrap().clone();
                if calls.len() >= expected {
                    return calls;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("provider calls should arrive")
    }

    fn recent_record(
        thread_id: &str,
        run_state: &str,
        active_run_id: Option<&str>,
    ) -> RecentThreadRecord {
        RecentThreadRecord {
            thread_id: thread_id.to_owned(),
            title: thread_id.to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 1,
            last_message_preview: String::new(),
            recent_run_id: None,
            active_run_id: active_run_id.map(ToOwned::to_owned),
            run_state: run_state.to_owned(),
            updated_at: Some("2026-01-01T00:00:00Z".to_owned()),
            last_active_at: "2026-01-01T00:00:00Z".to_owned(),
            recorded_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn pending_restart_wake_serializes_target() {
        let wake = PendingRestartWake {
            id: "wake-1".to_owned(),
            kind: "thread".to_owned(),
            target: "thread::abc".to_owned(),
            message: "continue".to_owned(),
            created_at: "2026-05-02T00:00:00Z".to_owned(),
            targets: Vec::new(),
            target_ordinals: Vec::new(),
            attempt: 0,
            errors: Vec::new(),
        };
        let value = serde_json::to_value(&wake).unwrap();
        assert_eq!(value["kind"], "thread");
        assert_eq!(value["target"], "thread::abc");
        assert!(value.get("targets").is_none());
    }

    #[test]
    fn wake_all_snapshot_includes_running_and_active_threads_with_cap() {
        let mut records = vec![
            recent_record("thread::self", "running", None),
            recent_record("thread::active-only", "completed", Some("run::active")),
            recent_record("thread::idle", "idle", None),
            recent_record("not-a-thread", "running", Some("run::ignored")),
        ];
        for index in 0..20 {
            records.push(recent_record(
                &format!("thread::extra-{index:02}"),
                "running",
                None,
            ));
        }

        let snapshot = restart_wake_all_snapshot_from_records(&records);

        assert_eq!(snapshot.targets.len(), MAX_RESTART_WAKE_ALL_THREADS);
        assert_eq!(snapshot.targets[0], "thread::self");
        assert_eq!(snapshot.targets[1], "thread::active-only");
        assert!(
            !snapshot
                .targets
                .iter()
                .any(|target| target == "thread::idle")
        );
        assert_eq!(snapshot.truncated_count, 6);
    }

    #[test]
    fn pending_wake_scanner_excludes_state_files() {
        assert!(is_fresh_pending_wake_file(Path::new("wake.json")));
        assert!(!is_fresh_pending_wake_file(Path::new(
            "wake.processing.json"
        )));
        assert!(!is_fresh_pending_wake_file(Path::new("wake.failed.json")));
        assert!(!is_fresh_pending_wake_file(Path::new("wake.tmp")));
        assert!(!is_fresh_pending_wake_file(Path::new("wake.extra.json")));
        assert!(is_processing_wake_file(Path::new("wake.processing.json")));
        assert_eq!(
            fresh_path_for_processing_wake(Path::new("/tmp/wake.processing.json"))
                .unwrap()
                .file_name()
                .and_then(|value| value.to_str()),
            Some("wake.json")
        );
    }

    #[test]
    fn stale_processing_recovery_restores_only_old_processing_files() {
        let temp = tempfile::tempdir().unwrap();
        let stale = temp.path().join("wake-old.processing.json");
        let fresh = temp.path().join("wake-new.processing.json");
        fs::write(&stale, b"{}").unwrap();
        fs::write(&fresh, b"{}").unwrap();
        let old_time = filetime::FileTime::from_unix_time(1, 0);
        filetime::set_file_mtime(&stale, old_time).unwrap();

        recover_stale_processing_wakes(temp.path());

        assert!(temp.path().join("wake-old.json").exists());
        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    #[tokio::test]
    async fn wake_all_drain_dispatches_each_target_once() {
        let temp = tempfile::tempdir().unwrap();
        let (state, provider) = test_state(4, false).await;
        seed_thread(&state, "thread::one").await;
        seed_thread(&state, "thread::two").await;
        let wake = PendingRestartWake {
            id: "wake-all".to_owned(),
            kind: WAKE_ALL_KIND.to_owned(),
            target: WAKE_ALL_TARGET.to_owned(),
            message: "continue".to_owned(),
            created_at: "2026-05-02T00:00:00Z".to_owned(),
            targets: vec![
                "thread::one".to_owned(),
                "thread::one".to_owned(),
                "thread::two".to_owned(),
            ],
            target_ordinals: vec![0, 0, 1],
            attempt: 0,
            errors: Vec::new(),
        };
        write_pending_restart_wake_path(&temp.path().join("wake-all.json"), &wake).unwrap();

        drain_pending_restart_wakes_from_dir(state, temp.path().to_path_buf()).await;

        let calls = wait_for_calls(&provider, 2).await;
        let mut thread_ids = calls.iter().map(|call| call.0.clone()).collect::<Vec<_>>();
        thread_ids.sort();
        assert_eq!(thread_ids, vec!["thread::one", "thread::two"]);
        assert!(calls.iter().all(|call| call.1 == "continue"));
        assert!(
            calls
                .iter()
                .all(|call| call.2["restart_wake_all"] == Value::Bool(true))
        );
        assert!(!temp.path().join("wake-all.processing.json").exists());
        assert!(!temp.path().join("wake-all.failed.json").exists());
    }

    #[tokio::test]
    async fn wake_all_requeues_only_unattempted_targets_on_overload() {
        let temp = tempfile::tempdir().unwrap();
        let (state, provider) = test_state(1, true).await;
        seed_thread(&state, "thread::one").await;
        seed_thread(&state, "thread::two").await;
        let wake = PendingRestartWake {
            id: "wake-overload".to_owned(),
            kind: WAKE_ALL_KIND.to_owned(),
            target: WAKE_ALL_TARGET.to_owned(),
            message: "continue".to_owned(),
            created_at: "2026-05-02T00:00:00Z".to_owned(),
            targets: vec!["thread::one".to_owned(), "thread::two".to_owned()],
            target_ordinals: vec![0, 1],
            attempt: 0,
            errors: Vec::new(),
        };
        write_pending_restart_wake_path(&temp.path().join("wake-overload.json"), &wake).unwrap();

        drain_pending_restart_wakes_from_dir(state, temp.path().to_path_buf()).await;

        let calls = wait_for_calls(&provider, 1).await;
        assert_eq!(calls[0].0, "thread::one");
        let requeued_bytes = fs::read(temp.path().join("wake-overload.json")).unwrap();
        let requeued: PendingRestartWake = serde_json::from_slice(&requeued_bytes).unwrap();
        assert_eq!(requeued.targets, vec!["thread::two"]);
        assert_eq!(requeued.target_ordinals, vec![1]);
        assert_eq!(requeued.attempt, 1);
        assert!(!temp.path().join("wake-overload.processing.json").exists());

        provider.release.notify_waiters();
    }

    #[tokio::test]
    async fn drain_skips_wake_file_already_claimed_by_another_drain() {
        let temp = tempfile::tempdir().unwrap();
        let (state, _provider) = test_state(1, false).await;
        let processing = temp.path().join("wake-race.processing.json");
        fs::write(&processing, b"{}").unwrap();

        drain_pending_restart_wake_file(
            state,
            temp.path().to_path_buf(),
            temp.path().join("wake-race.json"),
        )
        .await
        .expect("missing fresh file means another drain claimed it");

        assert!(processing.exists());
        assert!(!temp.path().join("wake-race.failed.json").exists());
    }

    /// Seed a task thread record directly on disk. Deliberately NOT via
    /// TaskService::create_task: creating a task also populates the
    /// process-wide in-memory task index, which would mask the cold-restart
    /// shape these tests need (task on disk, nothing warm in memory).
    async fn seed_cold_task_thread(state: &Arc<AppState>, number: u64, title: &str) -> String {
        let thread_id = format!("thread::wake-task-{number}");
        state
            .threads
            .thread_store
            .set(
                &thread_id,
                json!({
                    "thread_id": thread_id,
                    "channel": "api",
                    "account_id": "main",
                    "from_id": "loop",
                    "messages": [],
                    "task": {
                        "number": number,
                        "title": title,
                        "status": "todo",
                        "creator": {"kind": "agent", "agent_id": "test-agent"},
                        "created_at": "2026-01-01T00:00:00Z",
                        "updated_at": "2026-01-01T00:00:00Z",
                        "updated_by": {"kind": "agent", "agent_id": "test-agent"}
                    }
                }),
            )
            .await
            .unwrap();
        thread_id
    }

    #[tokio::test]
    async fn wake_task_target_resolves_through_projection() {
        let (state, _provider) = test_state(1, false).await;
        let number = 61u64;
        // Writing the record derives the task projection in the same
        // transaction (#TASK-1864): no backfill step exists any more.
        let thread_id = seed_cold_task_thread(&state, number, "Wake target").await;

        let resolved = resolve_task_thread_id(&state, &format!("#TASK-{number}"))
            .await
            .expect("task target resolves");
        assert_eq!(resolved, thread_id);
    }

    #[tokio::test]
    async fn wake_task_target_missing_projection_row_is_a_hard_not_found() {
        // Projections derive in the same transaction as every record write
        // (#TASK-1864): a missing row means the task genuinely does not
        // exist, and the former wake-side forced repair walk is retired.
        let (state, _provider) = test_state(1, false).await;
        let number = 72u64;
        let thread_id = seed_cold_task_thread(&state, number, "Lost row").await;
        assert!(
            state
                .ops
                .garyx_db
                .remove_task_projection(&thread_id)
                .expect("row removes"),
            "projection row should exist before the simulated loss"
        );

        let error = resolve_task_thread_id(&state, &format!("#TASK-{number}"))
            .await
            .expect_err("a missing projection row resolves to not-found");
        assert!(error.contains("not found"), "unexpected error: {error}");
    }
}
