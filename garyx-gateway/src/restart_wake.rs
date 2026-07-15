use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use garyx_models::local_paths::{default_session_data_dir, garyx_database_path_for_data_dir};
use garyx_router::is_thread_key;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::garyx_db::{GaryxDbError, GaryxDbService, ReadOnlyGaryxDb};
use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};
use crate::server::AppState;

const WAKE_ALL_KIND: &str = "all";
const WAKE_ALL_TARGET: &str = "all";
pub const RESTART_WAKE_ALL_SNAPSHOT_PATH: &str = "/api/restart-wake/snapshot";
/// Message injected when a restart wakes a thread. Wrapped in the
/// `garyx_restarted` tag so clients render it as a restart-notice card
/// (mirrors the task-notification card); an agent receiving it should simply
/// continue its interrupted work.
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestartWakeAllSnapshot {
    pub targets: Vec<String>,
    pub truncated_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RestartWakeDispatchError {
    RetryableOverload(String),
    Other(String),
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

pub fn queue_pending_restart_wake_all(
    message: &str,
    snapshot: RestartWakeAllSnapshot,
) -> Result<QueuedRestartWakeAll, Box<dyn std::error::Error>> {
    validate_restart_wake_all_snapshot(&snapshot)?;
    queue_pending_restart_wake_all_targets(message, snapshot)
}

fn validate_restart_wake_all_snapshot(
    snapshot: &RestartWakeAllSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    if snapshot.targets.len() > MAX_RESTART_WAKE_ALL_THREADS {
        return Err(format!(
            "restart wake-all snapshot exceeds target cap: {} > {MAX_RESTART_WAKE_ALL_THREADS}",
            snapshot.targets.len()
        )
        .into());
    }
    let mut seen = HashSet::new();
    for target in &snapshot.targets {
        if !is_thread_key(target) {
            return Err(format!(
                "restart wake-all snapshot contains non-canonical thread id: {target}"
            )
            .into());
        }
        if !seen.insert(target) {
            return Err(format!(
                "restart wake-all snapshot contains duplicate thread id: {target}"
            )
            .into());
        }
    }
    Ok(())
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
    if wake.kind != WAKE_ALL_KIND {
        // Single-target restart wakes are retired; a stray legacy file moves
        // to `.failed.json` via the caller's error path.
        return Err(format!("unknown restart wake kind: {}", wake.kind));
    }
    drain_pending_restart_wake_all_file(state, dir, path, processing_path, wake).await
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
) -> Result<RestartWakeAllSnapshot, GaryxDbError> {
    let page = garyx_db.list_active_recent_thread_ids(MAX_RESTART_WAKE_ALL_THREADS)?;
    Ok(RestartWakeAllSnapshot {
        truncated_count: page.total.saturating_sub(page.thread_ids.len()),
        targets: page.thread_ids,
    })
}

pub fn restart_wake_all_snapshot_from_data_dir(
    data_dir: impl AsRef<Path>,
) -> Result<RestartWakeAllSnapshot, GaryxDbError> {
    let database_path = garyx_database_path_for_data_dir(data_dir.as_ref());
    if !database_path.exists() {
        return Ok(RestartWakeAllSnapshot::default());
    }
    let mut garyx_db = ReadOnlyGaryxDb::open(database_path)?;
    let page = garyx_db.list_active_recent_thread_ids(MAX_RESTART_WAKE_ALL_THREADS)?;
    Ok(RestartWakeAllSnapshot {
        truncated_count: page.total.saturating_sub(page.thread_ids.len()),
        targets: page.thread_ids,
    })
}

pub(crate) async fn capture_restart_wake_all_snapshot(
    state: &Arc<AppState>,
) -> Result<RestartWakeAllSnapshot, GaryxDbError> {
    state
        .ops
        .garyx_db
        .run_blocking(restart_wake_all_snapshot_from_db)
        .await
}

pub async fn restart_wake_all_snapshot_endpoint(State(state): State<Arc<AppState>>) -> Response {
    match capture_restart_wake_all_snapshot(&state).await {
        Ok(snapshot) => (StatusCode::OK, Json(snapshot)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "InternalError",
                "message": error.to_string(),
            })),
        )
            .into_response(),
    }
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
    extra_metadata.insert("restart_wake_all".to_owned(), Value::Bool(true));
    extra_metadata.insert(
        "restart_wake_attempt".to_owned(),
        Value::Number(serde_json::Number::from(wake.attempt)),
    );
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

fn pending_restart_wake_dir() -> PathBuf {
    default_session_data_dir().join("restart-wake")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::Request;
    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;
    use garyx_bridge::MultiProviderBridge;
    use garyx_bridge::provider_trait::{BridgeError, ProviderRuntime, StreamCallback};
    use garyx_models::config::GaryxConfig;
    use garyx_models::provider::{
        ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent,
    };
    use serde_json::json;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use crate::garyx_db::RecentThreadDraft;

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

    fn seed_recent_record(
        garyx_db: &GaryxDbService,
        thread_id: &str,
        run_state: &str,
        active_run_id: Option<&str>,
        last_active_at: &str,
    ) {
        garyx_db
            .upsert_recent_thread(RecentThreadDraft {
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
                last_active_at: last_active_at.to_owned(),
            })
            .expect("seed recent thread");
    }

    #[test]
    fn wake_all_snapshot_includes_running_and_active_threads_with_cap() {
        let garyx_db = GaryxDbService::memory().expect("database");
        seed_recent_record(
            &garyx_db,
            "thread::self",
            "running",
            None,
            "2026-01-01T23:59:00Z",
        );
        seed_recent_record(
            &garyx_db,
            "thread::active-only",
            "completed",
            Some("run::active"),
            "2026-01-01T23:58:00Z",
        );
        seed_recent_record(
            &garyx_db,
            "thread::idle",
            "idle",
            None,
            "2026-01-01T23:57:00Z",
        );
        seed_recent_record(
            &garyx_db,
            "not-a-thread",
            "running",
            Some("run::ignored"),
            "2026-01-01T23:56:00Z",
        );
        for index in 0..20 {
            seed_recent_record(
                &garyx_db,
                &format!("thread::extra-{index:02}"),
                "running",
                None,
                &format!("2026-01-01T10:{index:02}:00Z"),
            );
        }

        let snapshot = restart_wake_all_snapshot_from_db(&garyx_db).expect("snapshot");

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
    fn wake_all_snapshot_reads_the_configured_data_dir() {
        let temp = tempfile::tempdir().expect("temp dir");
        let configured_data_dir = temp.path().join("custom-session-data");
        let garyx_db = GaryxDbService::open(garyx_database_path_for_data_dir(&configured_data_dir))
            .expect("custom database");
        seed_recent_record(
            &garyx_db,
            "thread::custom-data-running",
            "running",
            None,
            "2026-07-14T00:00:00Z",
        );

        let snapshot = restart_wake_all_snapshot_from_data_dir(&configured_data_dir)
            .expect("read custom database");

        assert_eq!(snapshot.targets, vec!["thread::custom-data-running"]);
        assert_eq!(snapshot.truncated_count, 0);
    }

    #[tokio::test]
    async fn wake_all_snapshot_http_endpoint_reads_the_sql_projection() {
        let (state, _provider) = test_state(1, false).await;
        seed_recent_record(
            &state.ops.garyx_db,
            "thread::http-running",
            "running",
            None,
            "2026-07-14T00:00:00Z",
        );
        seed_recent_record(
            &state.ops.garyx_db,
            "thread::http-idle",
            "idle",
            None,
            "2026-07-13T00:00:00Z",
        );
        let router = crate::build_router(state);
        let request = Request::builder()
            .uri(RESTART_WAKE_ALL_SNAPSHOT_PATH)
            .extension(ConnectInfo(
                "127.0.0.1:31337".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.expect("snapshot response");

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("response body");
        let snapshot: RestartWakeAllSnapshot =
            serde_json::from_slice(&bytes).expect("snapshot json");
        assert_eq!(snapshot.targets, vec!["thread::http-running"]);
        assert_eq!(snapshot.truncated_count, 0);
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

    #[tokio::test]
    async fn drain_moves_unknown_wake_kind_to_failed() {
        // Single-target restart wakes are retired: a stray legacy file must
        // not dispatch and must land in `.failed.json`.
        let temp = tempfile::tempdir().unwrap();
        let (state, provider) = test_state(1, false).await;
        let wake = PendingRestartWake {
            id: "wake-legacy".to_owned(),
            kind: "thread".to_owned(),
            target: "thread::abc".to_owned(),
            message: "continue".to_owned(),
            created_at: "2026-05-02T00:00:00Z".to_owned(),
            targets: Vec::new(),
            target_ordinals: Vec::new(),
            attempt: 0,
            errors: Vec::new(),
        };
        write_pending_restart_wake_path(&temp.path().join("wake-legacy.json"), &wake).unwrap();

        drain_pending_restart_wakes_from_dir(state, temp.path().to_path_buf()).await;

        assert!(provider.calls.lock().unwrap().is_empty());
        assert!(!temp.path().join("wake-legacy.json").exists());
        assert!(!temp.path().join("wake-legacy.processing.json").exists());
        assert!(temp.path().join("wake-legacy.failed.json").exists());
    }
}
