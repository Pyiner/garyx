use garyx_router::ThreadStoreExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use garyx_models::provider::ProviderRunResult;
use garyx_models::provider::{
    AgentDispatchOutcome, AgentRunRequest, FORK_FROM_PROVIDER_TYPE_METADATA_KEY,
    FORK_FROM_SDK_SESSION_ID_METADATA_KEY, FilePayload, ImagePayload, MODEL_METADATA_KEY,
    MODEL_OVERRIDE_METADATA_KEY, MODEL_REASONING_EFFORT_METADATA_KEY,
    MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY, MODEL_SERVICE_TIER_METADATA_KEY,
    MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY, PromptAttachment, ProviderRunOptions, ProviderType,
    QueuedUserInput, SDK_SESSION_FORK_METADATA_KEY, SDK_SESSION_ID_METADATA_KEY, StreamEvent,
    attachments_from_metadata, build_user_content_from_parts, stage_file_payloads_for_prompt,
    stage_image_payloads_for_prompt,
};
use garyx_models::thread_logs::{ThreadLogEvent, ThreadLogSink, resolve_thread_log_thread_id};
use garyx_models::{Principal, final_assistant_text_from_render_records};
#[cfg(test)]
use garyx_router::thread_metadata_from_value;
use garyx_router::{
    ThreadHistoryRepository, ThreadRunLease, ThreadStore, mark_thread_task_in_progress_on_wake,
    mark_thread_task_in_review_if_in_progress,
};
use serde_json::{Map, Value, json};
use tokio::sync::{OwnedMutexGuard, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep};

use crate::provider_common::{metadata_bool, metadata_string};
use crate::provider_trait::{BridgeError, ProviderRuntime, ProviderRuntimeSelection};
use crate::run_graph::{RunGraphState, execute_agent_run};

use super::persistence::{
    PendingUserInput, PendingUserInputStatus, PersistedRun, RunControlRecord, StreamingRunSnapshot,
    TerminalRunControl, ThreadPersistenceCommand, capsule_attached_control_record,
    save_failed_thread_messages_with_terminal_control, save_streaming_partial,
    save_thread_messages_with_terminal_control, strip_runtime_only_metadata,
};
use super::state::{self, ActiveThreadPersistence};
use super::{MultiProviderBridge, RunLifecycleEvent};

mod persistence_worker;
mod session_resolve;
mod task_hooks;
mod thread_title;

use persistence_worker::*;
use session_resolve::*;
use task_hooks::*;
use thread_title::*;

const STREAMING_INPUT_QUEUE_RETRY_INTERVAL: Duration = Duration::from_millis(50);
const STREAMING_INPUT_QUEUE_RETRY_TIMEOUT: Duration = Duration::from_secs(15);
const FOLLOW_UP_INTERRUPT_WAIT_INTERVAL: Duration = Duration::from_millis(25);
const FOLLOW_UP_INTERRUPT_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const LEGACY_DEFAULT_THREAD_LABEL: &str = "Fresh Thread";
const PROMPT_THREAD_TITLE_SOURCE: &str = "garyx_prompt";
const PROVIDER_THREAD_TITLE_SOURCE: &str = "provider";

#[cfg(test)]
pub(super) fn emit_committed_records_for_test(
    event_tx: &Option<tokio::sync::broadcast::Sender<String>>,
    thread_id: &str,
    run_id: Option<&str>,
    committed: Vec<(u64, Value)>,
) {
    persistence_worker::emit_committed_records(event_tx, thread_id, run_id, committed);
}

fn normalize_workspace_dir(workspace_dir: Option<String>) -> Option<String> {
    workspace_dir.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            let canonical = std::fs::canonicalize(trimmed)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| trimmed.to_owned());
            Some(canonical)
        }
    })
}

fn requested_provider_from_metadata(metadata: &HashMap<String, Value>) -> Option<ProviderType> {
    metadata
        .get("requested_provider_type")
        .and_then(Value::as_str)
        .and_then(ProviderType::from_slug)
}

fn requested_agent_id_from_metadata(metadata: &HashMap<String, Value>) -> Option<String> {
    metadata
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn queued_dispatch_metadata(
    metadata: &HashMap<String, Value>,
    requested_run_id: &str,
) -> HashMap<String, Value> {
    let mut durable = metadata.clone();
    if !requested_run_id.trim().is_empty() {
        durable.insert(
            "origin_run_id".to_owned(),
            Value::String(requested_run_id.to_owned()),
        );
    }
    durable
}

fn pending_input_metadata_for_persistence(
    mut metadata: HashMap<String, Value>,
) -> HashMap<String, Value> {
    strip_runtime_only_metadata(&mut metadata);
    metadata
}

fn thread_value_string(thread_data: &Value, key: &str) -> Option<String> {
    thread_data
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get(key))
        .or_else(|| thread_data.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn has_thread_value(thread_data: &Value, key: &str) -> bool {
    thread_value_string(thread_data, key).is_some()
}

fn summarize_value(value: &Value, limit: usize) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => summarize_text(text, limit),
        other => summarize_text(&serde_json::to_string(other).unwrap_or_default(), limit),
    }
}

#[cfg(test)]
fn scrub_inline_run_runtime_metadata(metadata: &mut HashMap<String, Value>) {
    // Inline runs inherit caller metadata for runtime context, but
    // thread-bound identity/provider fields must be re-derived from the target
    // thread itself.
    for key in [
        "agent_id",
        "agent_display_name",
        "model",
        "model_reasoning_effort",
        "system_prompt",
        "requested_provider_type",
        SDK_SESSION_ID_METADATA_KEY,
        SDK_SESSION_FORK_METADATA_KEY,
        "bridge_run_id",
    ] {
        metadata.remove(key);
    }
}

fn summarize_provider_message(message: &garyx_models::provider::ProviderMessage) -> String {
    if let Some(text) = message.text.as_deref() {
        let summary = summarize_text(text, 160);
        if !summary.is_empty() {
            return summary;
        }
    }
    summarize_value(&message.content, 160)
}

fn emit_gateway_event(event_tx: &Option<tokio::sync::broadcast::Sender<String>>, payload: Value) {
    if let Some(tx) = event_tx {
        let _ = tx.send(payload.to_string());
    }
}

struct StreamingPersistenceWorkerResult {
    assistant_response: String,
    session_messages: Vec<garyx_models::provider::ProviderMessage>,
    transcript_controls: Vec<RunControlRecord>,
}

pub struct QueuedStreamingInput {
    pub pending_input_id: String,
    pub run_id: String,
}

enum DispatchExecutionMode {
    Legacy,
    Durable {
        active_run_plan: DurableActiveRunPlan,
        pending_input_id: String,
    },
}

enum DurableActiveRunPlan {
    NoActiveRun,
    QueueTo { run_id: String },
    Replace { run_id: String },
}

struct RunLifecycleTerminalGuard {
    tx: tokio::sync::broadcast::Sender<RunLifecycleEvent>,
    thread_id: String,
    run_id: String,
}

impl RunLifecycleTerminalGuard {
    fn started(
        tx: tokio::sync::broadcast::Sender<RunLifecycleEvent>,
        thread_id: String,
        run_id: String,
    ) -> Self {
        let _ = tx.send(RunLifecycleEvent::Started {
            thread_id: thread_id.clone(),
            run_id: run_id.clone(),
        });
        Self {
            tx,
            thread_id,
            run_id,
        }
    }
}

impl Drop for RunLifecycleTerminalGuard {
    fn drop(&mut self) {
        let _ = self.tx.send(RunLifecycleEvent::Terminal {
            thread_id: self.thread_id.clone(),
            run_id: self.run_id.clone(),
        });
    }
}

/// Side-effect-free, per-thread-serialized provider handoff plan.
///
/// The owned dispatch guard keeps another same-thread dispatch from changing
/// the plan between the durable database admission commit and execution.
/// Provider completion may still race a queued plan; that becomes an
/// ambiguous one-shot result instead of falling back to a fresh run.
pub struct DurableDispatchPlan {
    request: AgentRunRequest,
    run_lease: Option<ThreadRunLease>,
    thread_dispatch_guard: OwnedMutexGuard<()>,
    active_run_plan: DurableActiveRunPlan,
    pending_input_id: String,
}

/// Side-effect-free direct stream-input plan held under the same per-thread
/// dispatch guard as chat-start planning.
pub struct DurableStreamInputPlan {
    thread_id: String,
    message: String,
    images: Option<Vec<ImagePayload>>,
    files: Option<Vec<FilePayload>>,
    attachments: Option<Vec<PromptAttachment>>,
    client_intent_id: Option<String>,
    thread_dispatch_guard: OwnedMutexGuard<()>,
    planned_active_run_id: Option<String>,
    pending_input_id: String,
}

impl DurableStreamInputPlan {
    pub fn effective_run_id(&self) -> Option<&str> {
        self.planned_active_run_id.as_deref()
    }

    pub fn pending_input_id(&self) -> Option<&str> {
        self.planned_active_run_id
            .as_ref()
            .map(|_| self.pending_input_id.as_str())
    }
}

impl DurableDispatchPlan {
    pub fn requested_run_id(&self) -> &str {
        &self.request.run_id
    }

    pub fn effective_run_id(&self) -> &str {
        match &self.active_run_plan {
            DurableActiveRunPlan::QueueTo { run_id } => run_id,
            DurableActiveRunPlan::NoActiveRun | DurableActiveRunPlan::Replace { .. } => {
                &self.request.run_id
            }
        }
    }

    pub fn pending_input_id(&self) -> Option<&str> {
        matches!(self.active_run_plan, DurableActiveRunPlan::QueueTo { .. })
            .then_some(self.pending_input_id.as_str())
    }

    pub fn outcome(&self) -> AgentDispatchOutcome {
        match &self.active_run_plan {
            DurableActiveRunPlan::QueueTo { run_id } => AgentDispatchOutcome::QueuedToActiveRun {
                effective_run_id: run_id.clone(),
                pending_input_id: self.pending_input_id.clone(),
            },
            DurableActiveRunPlan::NoActiveRun | DurableActiveRunPlan::Replace { .. } => {
                AgentDispatchOutcome::Started
            }
        }
    }
}

async fn active_run_id_for_thread(inner: &super::state::Inner, thread_id: &str) -> Option<String> {
    inner
        .run_index
        .read()
        .await
        .run_sessions
        .iter()
        .find(|(_, candidate_thread_id)| candidate_thread_id.as_str() == thread_id)
        .map(|(run_id, _)| run_id.clone())
}

async fn planned_active_run_id_for_thread(
    inner: &super::state::Inner,
    thread_id: &str,
) -> Option<String> {
    if let Some(run_id) = inner
        .active_thread_persistence
        .lock()
        .await
        .get(thread_id)
        .map(|handle| handle.run_id.clone())
    {
        return Some(run_id);
    }
    active_run_id_for_thread(inner, thread_id).await
}

async fn active_run_supports_streaming_input(
    bridge: &MultiProviderBridge,
    thread_id: &str,
    run_id: &str,
) -> bool {
    let provider_key = bridge
        .inner
        .run_index
        .read()
        .await
        .active_runs
        .get(run_id)
        .cloned();
    let provider_key = match provider_key {
        Some(provider_key) => Some(provider_key),
        None => bridge
            .inner
            .thread_affinity
            .read()
            .await
            .get(thread_id)
            .cloned(),
    };
    let Some(provider_key) = provider_key else {
        return false;
    };
    bridge
        .get_provider(&provider_key)
        .await
        .is_some_and(|provider| provider.supports_streaming_input())
}

async fn durable_active_run_plan(
    bridge: &MultiProviderBridge,
    thread_id: &str,
) -> DurableActiveRunPlan {
    let Some(run_id) = planned_active_run_id_for_thread(&bridge.inner, thread_id).await else {
        return DurableActiveRunPlan::NoActiveRun;
    };
    if active_run_supports_streaming_input(bridge, thread_id, &run_id).await {
        DurableActiveRunPlan::QueueTo { run_id }
    } else {
        DurableActiveRunPlan::Replace { run_id }
    }
}

async fn has_active_streaming_run_for_thread(inner: &super::state::Inner, thread_id: &str) -> bool {
    if active_run_id_for_thread(inner, thread_id).await.is_some() {
        return true;
    }

    inner
        .active_thread_persistence
        .lock()
        .await
        .contains_key(thread_id)
}

async fn queue_streaming_input_with_retry(
    inner: &super::state::Inner,
    provider: Arc<dyn ProviderRuntime>,
    thread_id: &str,
    input: QueuedUserInput,
) -> bool {
    if provider.add_streaming_input(thread_id, input.clone()).await {
        return true;
    }

    if !provider.supports_streaming_input()
        || !has_active_streaming_run_for_thread(inner, thread_id).await
    {
        return false;
    }

    let deadline = Instant::now() + STREAMING_INPUT_QUEUE_RETRY_TIMEOUT;
    while Instant::now() < deadline {
        sleep(STREAMING_INPUT_QUEUE_RETRY_INTERVAL).await;

        if provider.add_streaming_input(thread_id, input.clone()).await {
            tracing::debug!(
                thread_id = %thread_id,
                "queued streaming input after waiting for provider readiness"
            );
            return true;
        }

        if !has_active_streaming_run_for_thread(inner, thread_id).await {
            return false;
        }
    }

    tracing::debug!(
        thread_id = %thread_id,
        "timed out waiting for active provider run to accept queued input"
    );
    false
}

async fn render_streaming_user_message_for_provider(
    _inner: &super::state::Inner,
    _thread_id: &str,
    message: &str,
) -> String {
    message.to_owned()
}

fn non_empty_trimmed_owned(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

async fn wait_for_thread_to_become_idle(
    inner: &super::state::Inner,
    thread_id: &str,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !has_active_streaming_run_for_thread(inner, thread_id).await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(FOLLOW_UP_INTERRUPT_WAIT_INTERVAL).await;
    }
}

async fn restore_thread_affinity_from_store(
    bridge: &MultiProviderBridge,
    thread_id: &str,
) -> Option<String> {
    if bridge
        .inner
        .thread_affinity
        .read()
        .await
        .contains_key(thread_id)
    {
        return bridge
            .inner
            .thread_affinity
            .read()
            .await
            .get(thread_id)
            .cloned();
    }

    let store = bridge.inner.thread_store.read().await.clone()?;
    let session_data = store.get_logged(thread_id).await?;
    if let Some(provider_key) = session_data
        .get("provider_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        && bridge.get_provider(&provider_key).await.is_some()
    {
        bridge
            .inner
            .thread_affinity
            .write()
            .await
            .insert(thread_id.to_owned(), provider_key.clone());
        return Some(provider_key);
    }
    if let Some(provider_type) = persisted_provider_type(&session_data)
        && let Some(provider_key) = bridge.select_best_provider(Some(provider_type), true).await
    {
        bridge
            .inner
            .thread_affinity
            .write()
            .await
            .insert(thread_id.to_owned(), provider_key.clone());
        return Some(provider_key);
    }
    None
}

async fn record_thread_log(
    sink: Option<Arc<dyn ThreadLogSink>>,
    thread_id: Option<&str>,
    event: ThreadLogEvent,
) {
    let Some(thread_id) = thread_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let Some(sink) = sink else {
        return;
    };
    sink.record_event(ThreadLogEvent {
        thread_id: thread_id.to_owned(),
        ..event
    })
    .await;
}

async fn resolve_effective_workspace_dir(
    inner: &super::state::Inner,
    thread_id: &str,
    workspace_dir: Option<String>,
) -> Result<Option<String>, BridgeError> {
    let mut bindings = inner.thread_workspace_bindings.write().await;
    let requested = normalize_workspace_dir(workspace_dir);
    match (bindings.get(thread_id).cloned(), requested) {
        (Some(bound), Some(requested)) => {
            if requested != bound {
                return Err(BridgeError::SessionError(format!(
                    "thread {thread_id} is already bound to workspace {bound}",
                )));
            }
            Ok(Some(bound))
        }
        (Some(bound), None) => Ok(Some(bound)),
        (None, Some(requested)) => {
            bindings.insert(thread_id.to_owned(), requested.clone());
            Ok(Some(requested))
        }
        (None, None) => Ok(None),
    }
}

/// Thread-log labels used by the shared streaming response callback. The
/// production dispatch path and the inline sub-agent path emit the same
/// events with different label prefixes.
struct StreamCallbackLogLabels {
    first_token: &'static str,
    tool_use: &'static str,
    tool_result: &'static str,
}

const RUN_STREAM_LOG_LABELS: StreamCallbackLogLabels = StreamCallbackLogLabels {
    first_token: "first token received",
    tool_use: "tool use emitted",
    tool_result: "tool result emitted",
};

#[cfg(test)]
const SUBAGENT_STREAM_LOG_LABELS: StreamCallbackLogLabels = StreamCallbackLogLabels {
    first_token: "sub-agent first token received",
    tool_use: "sub-agent tool use emitted",
    tool_result: "sub-agent tool result emitted",
};

/// Builds the streaming response callback shared by the production dispatch
/// path (`start_admitted_run`) and the inline sub-agent path
/// (`run_inline_streaming`): forwards events into the partial-persistence
/// worker, records first-token/tool thread logs, and relays non-control
/// events to the external callback once persistence has been handed off.
fn build_streaming_response_callback(
    sink: Option<Arc<dyn ThreadLogSink>>,
    thread_log_id: Option<String>,
    run_id: String,
    external_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    partial_persistence_tx: Option<mpsc::UnboundedSender<ThreadPersistenceCommand>>,
    labels: &'static StreamCallbackLogLabels,
) -> Arc<dyn Fn(StreamEvent) + Send + Sync> {
    let first_token_logged = Arc::new(AtomicBool::new(false));
    Arc::new(move |event: StreamEvent| {
        let sink = sink.clone();
        let thread_log_id = thread_log_id.clone();
        let run_id = run_id.clone();
        let external_callback = external_callback.clone();
        let first_token_logged = first_token_logged.clone();
        let event_for_log = event.clone();
        let persistent_control = is_persistent_control_stream_event(&event);
        let callback_after_commit = if persistent_control {
            if let Some(tx) = partial_persistence_tx.as_ref() {
                let callback = external_callback.clone();
                let sent = tx
                    .send(ThreadPersistenceCommand::Stream {
                        event: event.clone(),
                        after_commit: callback.clone(),
                    })
                    .is_ok();
                sent.then_some(callback).flatten()
            } else {
                None
            }
        } else {
            if let Some(tx) = partial_persistence_tx.as_ref() {
                let _ = tx.send(ThreadPersistenceCommand::Stream {
                    event: event.clone(),
                    after_commit: None,
                });
            }
            None
        };
        tokio::spawn(async move {
            match event_for_log {
                StreamEvent::Delta { text } => {
                    if !text.is_empty() && !first_token_logged.swap(true, Ordering::Relaxed) {
                        record_thread_log(
                            sink,
                            thread_log_id.as_deref(),
                            ThreadLogEvent::info("", "run", labels.first_token).with_run_id(run_id),
                        )
                        .await;
                    }
                }
                StreamEvent::ToolUse { message } => {
                    record_thread_log(
                        sink,
                        thread_log_id.as_deref(),
                        ThreadLogEvent::info("", "tool", labels.tool_use)
                            .with_run_id(run_id)
                            .with_field("tool_name", json!(message.tool_name))
                            .with_field("tool_use_id", json!(message.tool_use_id))
                            .with_field("message", json!(summarize_provider_message(&message))),
                    )
                    .await;
                }
                StreamEvent::ToolResult { message } => {
                    record_thread_log(
                        sink,
                        thread_log_id.as_deref(),
                        ThreadLogEvent::info("", "tool", labels.tool_result)
                            .with_run_id(run_id)
                            .with_field("tool_name", json!(message.tool_name))
                            .with_field("tool_use_id", json!(message.tool_use_id))
                            .with_field("is_error", json!(message.is_error))
                            .with_field("message", json!(summarize_provider_message(&message))),
                    )
                    .await;
                }
                StreamEvent::SessionBound { .. }
                | StreamEvent::Boundary { .. }
                | StreamEvent::ThreadTitleUpdated { .. }
                | StreamEvent::Done => {}
            }
        });

        if matches!(event, StreamEvent::ThreadTitleUpdated { .. }) {
            return;
        }

        if callback_after_commit.is_none()
            && let Some(callback) = external_callback
        {
            callback(event);
        }
    })
}

/// Finalizes the partial-persistence worker for a finished provider run:
/// detaches this run's active-persistence handle, drops the shared sender so
/// the worker stops waiting for commands, sends `Finish`, awaits the worker
/// result, and clears the handle again afterwards. Shared by the production
/// dispatch path and the inline sub-agent path.
async fn finalize_partial_persistence(
    active_thread_persistence: &tokio::sync::Mutex<HashMap<String, ActiveThreadPersistence>>,
    thread_id: &str,
    run_id: &str,
    partial_persistence_tx: Option<mpsc::UnboundedSender<ThreadPersistenceCommand>>,
    partial_persistence_task: Option<JoinHandle<StreamingPersistenceWorkerResult>>,
    failure_log: &'static str,
) -> Option<StreamingPersistenceWorkerResult> {
    let removed_persistence = {
        let mut persistence = active_thread_persistence.lock().await;
        let should_remove = persistence
            .get(thread_id)
            .map(|handle| handle.run_id == run_id)
            .unwrap_or(false);
        if should_remove {
            persistence.remove(thread_id)
        } else {
            None
        }
    };
    drop(removed_persistence);
    if let Some(tx) = partial_persistence_tx.as_ref() {
        let _ = tx.send(ThreadPersistenceCommand::Finish);
    }
    drop(partial_persistence_tx);
    let persistence_result = if let Some(task) = partial_persistence_task {
        match task.await {
            Ok(result) => Some(result),
            Err(error) => {
                tracing::warn!(run_id = %run_id, error = %error, "{}", failure_log);
                None
            }
        }
    } else {
        None
    };
    {
        let mut persistence = active_thread_persistence.lock().await;
        let should_remove = persistence
            .get(thread_id)
            .map(|handle| handle.run_id == run_id)
            .unwrap_or(false);
        if should_remove {
            persistence.remove(thread_id);
        }
    }
    persistence_result
}

/// Persists the terminal state of a successful provider call (including the
/// Codex soft-failure `Ok` with `success == false`): applies a provider
/// thread title if the thread has none, consumes the soft-failure rate-limit
/// context, writes user/assistant/tool messages with the terminal control,
/// and returns the applied thread title plus the committed records for the
/// caller to emit at its own event-bus read point.
#[allow(clippy::too_many_arguments)]
async fn persist_terminal_success(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    provider: &dyn ProviderRuntime,
    thread_id: &str,
    user_message: &str,
    provider_key: &str,
    run_started_at: &str,
    graph_state: &RunGraphState,
    res: &ProviderRunResult,
    persistence_result: Option<&StreamingPersistenceWorkerResult>,
) -> (Option<String>, Vec<(u64, Value)>) {
    let user_images = graph_state.run_options.images.clone().unwrap_or_default();
    let persisted_assistant_response = persistence_result
        .map(|value| value.assistant_response.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(&res.response);
    let persisted_session_messages = persistence_result
        .map(|value| value.session_messages.as_slice())
        .filter(|value| !value.is_empty())
        .unwrap_or(&res.session_messages);
    let persisted_transcript_controls = persistence_result
        .map(|value| value.transcript_controls.as_slice())
        .unwrap_or(&[]);
    let sdk_session_id = resolve_sdk_session_id_for_persistence(
        &graph_state.run_options.metadata,
        res.sdk_session_id.as_deref(),
    );
    let applied_thread_title =
        persist_provider_thread_title_if_missing(store, thread_id, res.thread_title.as_deref())
            .await;
    // Codex surfaces a usage-quota exhaustion as a soft failure (`Ok` result
    // with `success == false`), so the rate-limit context is consumed here
    // rather than on the hard-error path.
    let rate_limit = if res.success {
        None
    } else {
        provider.take_rate_limit(thread_id).await
    };
    let terminal_committed = save_thread_messages_with_terminal_control(
        store,
        history,
        PersistedRun {
            thread_id,
            user_message,
            user_timestamp: Some(run_started_at),
            user_images: &user_images,
            assistant_response: persisted_assistant_response,
            sdk_session_id: sdk_session_id.as_deref(),
            provider_key,
            provider_type: provider.provider_type(),
            session_messages: persisted_session_messages,
            metadata: &graph_state.run_options.metadata,
        },
        persisted_transcript_controls,
        Some(TerminalRunControl {
            duration_ms: Some(graph_state.metrics.duration_ms()),
            success: Some(res.success),
            error: res.error.clone(),
            thread_title: applied_thread_title.clone(),
            rate_limit,
        }),
    )
    .await;
    (applied_thread_title, terminal_committed)
}

/// Persists the terminal state of a hard-failed provider call: consumes the
/// rate-limit context, writes whatever partial output the persistence worker
/// captured with a failed terminal control, and returns the committed
/// records for the caller to emit at its own event-bus read point.
#[allow(clippy::too_many_arguments)]
async fn persist_terminal_failure(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    provider: &dyn ProviderRuntime,
    thread_id: &str,
    user_message: &str,
    provider_key: &str,
    run_started_at: &str,
    graph_state: &RunGraphState,
    error: &BridgeError,
    persistence_result: Option<&StreamingPersistenceWorkerResult>,
) -> Vec<(u64, Value)> {
    let user_images = graph_state.run_options.images.clone().unwrap_or_default();
    let failed_assistant_response = persistence_result
        .map(|value| value.assistant_response.as_str())
        .unwrap_or_default();
    let failed_session_messages = persistence_result
        .map(|value| value.session_messages.as_slice())
        .unwrap_or(&[]);
    let persisted_transcript_controls = persistence_result
        .map(|value| value.transcript_controls.as_slice())
        .unwrap_or(&[]);
    let rate_limit = provider.take_rate_limit(thread_id).await;
    let terminal_committed = save_failed_thread_messages_with_terminal_control(
        store,
        history,
        PersistedRun {
            thread_id,
            user_message,
            user_timestamp: Some(run_started_at),
            user_images: &user_images,
            assistant_response: failed_assistant_response,
            sdk_session_id: None,
            provider_key,
            provider_type: provider.provider_type(),
            session_messages: failed_session_messages,
            metadata: &graph_state.run_options.metadata,
        },
        persisted_transcript_controls,
        Some(TerminalRunControl {
            duration_ms: Some(graph_state.metrics.duration_ms()),
            success: Some(false),
            error: Some(error.to_string()),
            thread_title: None,
            rate_limit,
        }),
    )
    .await;
    terminal_committed
}

impl MultiProviderBridge {
    async fn resolve_thread_execution_target(
        &self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        metadata: &mut HashMap<String, Value>,
        requested_provider: Option<ProviderType>,
    ) -> Result<(String, Arc<dyn ProviderRuntime>, Option<ProviderType>), BridgeError> {
        let _ = restore_thread_affinity_from_store(self, thread_id).await;
        // Every dispatch funnels through here, so the thread's bound agent
        // configuration is backfilled once at this chokepoint instead of at
        // each entry point. Entry points that resolve an explicit agent
        // override (for example chat one-off targets) have already written
        // their values, which win.
        self.backfill_bound_agent_runtime_metadata(thread_id, metadata)
            .await;
        let requested_provider =
            requested_provider.or_else(|| requested_provider_from_metadata(metadata));

        let exact_agent_provider_key =
            if let Some(agent_id) = requested_agent_id_from_metadata(metadata) {
                self.provider_key_for_agent_id(&agent_id).await?
            } else {
                None
            };
        let provider_key = match exact_agent_provider_key {
            Some(provider_key) => Some(provider_key),
            None => {
                self.resolve_provider_for_request(
                    thread_id,
                    channel,
                    account_id,
                    requested_provider.clone(),
                )
                .await
            }
        }
        .ok_or_else(|| {
            BridgeError::ProviderNotFound(format!(
                "no provider for channel={channel} account={account_id}{}",
                requested_provider
                    .as_ref()
                    .map(|value| format!(" type={value:?}"))
                    .unwrap_or_default(),
            ))
        })?;
        let provider = self
            .get_provider(&provider_key)
            .await
            .ok_or_else(|| BridgeError::ProviderNotFound(provider_key.clone()))?;

        Ok((provider_key, provider, requested_provider))
    }

    async fn acquire_thread_dispatch_guard(&self, thread_id: &str) -> OwnedMutexGuard<()> {
        let guard = {
            let mut guards = self.inner.thread_dispatch_guards.lock().await;
            guards
                .entry(thread_id.to_owned())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        guard.lock_owned().await
    }

    /// Plan a correlated dispatch without crossing the provider side-effect
    /// boundary. The returned value owns the bridge's same-thread ordering
    /// guard until it is executed or dropped.
    pub async fn prepare_durable_dispatch(
        &self,
        run: garyx_router::AdmittedRun,
        pending_input_id: String,
    ) -> Result<DurableDispatchPlan, String> {
        let (request, run_lease) = run.into_dispatch_parts();
        if let Some(lease) = run_lease.as_ref() {
            lease.ensure_valid().map_err(|error| error.to_string())?;
        }
        let thread_dispatch_guard = self.acquire_thread_dispatch_guard(&request.thread_id).await;
        let active_run_plan = durable_active_run_plan(self, &request.thread_id).await;
        Ok(DurableDispatchPlan {
            request,
            run_lease,
            thread_dispatch_guard,
            active_run_plan,
            pending_input_id,
        })
    }

    /// Cross the provider handoff boundary exactly once for a previously
    /// prepared durable plan.
    pub async fn execute_durable_dispatch(
        &self,
        plan: DurableDispatchPlan,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<AgentDispatchOutcome, String> {
        let DurableDispatchPlan {
            request,
            run_lease,
            thread_dispatch_guard,
            active_run_plan,
            pending_input_id,
        } = plan;
        self.start_admitted_run_with_guard(
            request,
            run_lease,
            response_callback,
            thread_dispatch_guard,
            DispatchExecutionMode::Durable {
                active_run_plan,
                pending_input_id,
            },
        )
        .await
        .map_err(|error| error.to_string())
    }

    /// Plan direct follow-up input without staging a pending record or calling
    /// the provider. A missing queue-capable active run is a stable,
    /// side-effect-free plan so callers can fall back to a replacement run.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_durable_stream_input(
        &self,
        thread_id: String,
        message: String,
        images: Option<Vec<ImagePayload>>,
        files: Option<Vec<FilePayload>>,
        attachments: Option<Vec<PromptAttachment>>,
        client_intent_id: Option<String>,
        pending_input_id: String,
    ) -> DurableStreamInputPlan {
        let thread_dispatch_guard = self.acquire_thread_dispatch_guard(&thread_id).await;
        let planned_active_run_id =
            match planned_active_run_id_for_thread(&self.inner, &thread_id).await {
                Some(run_id)
                    if active_run_supports_streaming_input(self, &thread_id, &run_id).await =>
                {
                    Some(run_id)
                }
                _ => None,
            };
        DurableStreamInputPlan {
            thread_id,
            message,
            images,
            files,
            attachments,
            client_intent_id,
            thread_dispatch_guard,
            planned_active_run_id,
            pending_input_id,
        }
    }

    /// Execute a direct stream-input plan exactly once. Any untyped provider
    /// failure after planning an active run is ambiguous and never retried.
    pub async fn execute_durable_stream_input(
        &self,
        plan: DurableStreamInputPlan,
    ) -> Result<Option<QueuedStreamingInput>, String> {
        let DurableStreamInputPlan {
            thread_id,
            message,
            images,
            files,
            attachments,
            client_intent_id,
            thread_dispatch_guard,
            planned_active_run_id,
            pending_input_id,
        } = plan;
        let Some(expected_run_id) = planned_active_run_id else {
            drop(thread_dispatch_guard);
            return Ok(None);
        };
        let queued = self
            .add_streaming_input_with_metadata_exact(
                &thread_id,
                &message,
                images,
                files,
                attachments,
                client_intent_id,
                HashMap::new(),
                pending_input_id.clone(),
            )
            .await;
        drop(thread_dispatch_guard);
        match queued {
            Some(queued)
                if queued.run_id == expected_run_id
                    && queued.pending_input_id == pending_input_id =>
            {
                Ok(Some(queued))
            }
            Some(queued) => Err(format!(
                "durable stream-input handoff changed plan: expected run {expected_run_id} / pending {pending_input_id}, got run {} / pending {}",
                queued.run_id, queued.pending_input_id
            )),
            None => Err("durable stream-input provider handoff outcome is ambiguous".to_owned()),
        }
    }

    /// Start an agent run and forward optional image payloads to providers.
    pub(crate) async fn start_admitted_run(
        &self,
        request: AgentRunRequest,
        run_lease: Option<ThreadRunLease>,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<AgentDispatchOutcome, BridgeError> {
        let thread_dispatch_guard = self.acquire_thread_dispatch_guard(&request.thread_id).await;
        self.start_admitted_run_with_guard(
            request,
            run_lease,
            response_callback,
            thread_dispatch_guard,
            DispatchExecutionMode::Legacy,
        )
        .await
    }

    async fn start_admitted_run_with_guard(
        &self,
        request: AgentRunRequest,
        mut run_lease: Option<ThreadRunLease>,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
        thread_dispatch_guard: OwnedMutexGuard<()>,
        dispatch_mode: DispatchExecutionMode,
    ) -> Result<AgentDispatchOutcome, BridgeError> {
        if let Some(lease) = run_lease.as_ref() {
            lease
                .ensure_valid()
                .map_err(|error| BridgeError::SessionError(error.to_string()))?;
        }
        let AgentRunRequest {
            thread_id,
            message,
            run_id,
            channel,
            account_id,
            mut metadata,
            images,
            workspace_dir,
            requested_provider,
        } = request;
        let thread_log_id = resolve_thread_log_thread_id(&thread_id, &metadata);
        let thread_logs = self.thread_log_sink();
        let effective_workspace_dir =
            resolve_effective_workspace_dir(&self.inner, &thread_id, workspace_dir).await?;

        let gateway_event_tx = self.inner.event_tx.read().await.clone();
        let (provider_key, provider, _) = self
            .resolve_thread_execution_target(
                &thread_id,
                &channel,
                &account_id,
                &mut metadata,
                requested_provider,
            )
            .await?;
        record_thread_log(
            thread_logs.clone(),
            thread_log_id.as_deref(),
            ThreadLogEvent::info("", "run", "provider resolved")
                .with_run_id(&run_id)
                .with_field("provider_key", json!(provider_key.clone()))
                .with_field("channel", json!(channel))
                .with_field("account_id", json!(account_id))
                .with_field("thread_id", json!(thread_id)),
        )
        .await;

        // If there is already an active streaming session for this thread,
        // queue the message as streaming input instead of spawning a new run.
        // If the provider cannot accept follow-up input mid-run, interrupt the
        // in-flight run and wait for cleanup before starting the replacement
        // run. Same-thread follow-ups must never run concurrently.
        let prompt_attachments = attachments_from_metadata(&metadata);
        let queued_attachments = if prompt_attachments.is_empty() {
            None
        } else {
            Some(prompt_attachments.clone())
        };
        if let Some(lease) = run_lease.as_ref() {
            lease
                .ensure_valid()
                .map_err(|error| BridgeError::SessionError(error.to_string()))?;
        }
        let queued = match &dispatch_mode {
            DispatchExecutionMode::Legacy => {
                self.add_streaming_input_with_metadata(
                    &thread_id,
                    &message,
                    images.clone(),
                    None,
                    queued_attachments.clone(),
                    metadata_string(&metadata, "client_intent_id"),
                    queued_dispatch_metadata(&metadata, &run_id),
                )
                .await
            }
            DispatchExecutionMode::Durable {
                active_run_plan: DurableActiveRunPlan::QueueTo { .. },
                pending_input_id,
            } => {
                self.add_streaming_input_with_metadata_exact(
                    &thread_id,
                    &message,
                    images.clone(),
                    None,
                    queued_attachments.clone(),
                    metadata_string(&metadata, "client_intent_id"),
                    queued_dispatch_metadata(&metadata, &run_id),
                    pending_input_id.clone(),
                )
                .await
            }
            DispatchExecutionMode::Durable {
                active_run_plan:
                    DurableActiveRunPlan::NoActiveRun | DurableActiveRunPlan::Replace { .. },
                ..
            } => None,
        };
        if let Some(queued) = queued {
            if let DispatchExecutionMode::Durable {
                active_run_plan:
                    DurableActiveRunPlan::QueueTo {
                        run_id: expected_run_id,
                    },
                ..
            } = &dispatch_mode
                && queued.run_id != *expected_run_id
            {
                return Err(BridgeError::SessionError(format!(
                    "durable queued handoff changed active run from {expected_run_id} to {}",
                    queued.run_id
                )));
            }
            if let Some(lease) = run_lease.as_ref() {
                lease
                    .ensure_valid()
                    .map_err(|error| BridgeError::SessionError(error.to_string()))?;
            }
            tracing::info!(
                thread_id = %thread_id,
                run_id = %run_id,
                effective_run_id = %queued.run_id,
                "queued message to existing streaming session"
            );
            record_thread_log(
                thread_logs.clone(),
                thread_log_id.as_deref(),
                ThreadLogEvent::info("", "dispatch", "queued message to active streaming session")
                    .with_run_id(&run_id)
                    .with_field("provider_key", json!(provider_key.clone()))
                    .with_field("effective_run_id", json!(queued.run_id.clone()))
                    .with_field("message", json!(summarize_text(&message, 160))),
            )
            .await;
            return Ok(AgentDispatchOutcome::QueuedToActiveRun {
                effective_run_id: queued.run_id,
                pending_input_id: queued.pending_input_id,
            });
        }
        if matches!(
            &dispatch_mode,
            DispatchExecutionMode::Durable {
                active_run_plan: DurableActiveRunPlan::QueueTo { .. },
                ..
            }
        ) {
            return Err(BridgeError::SessionError(
                "durable queued provider handoff outcome is ambiguous".to_owned(),
            ));
        }
        let active_run_to_replace = match &dispatch_mode {
            DispatchExecutionMode::Legacy => {
                planned_active_run_id_for_thread(&self.inner, &thread_id).await
            }
            DispatchExecutionMode::Durable {
                active_run_plan:
                    DurableActiveRunPlan::Replace {
                        run_id: expected_run_id,
                    },
                ..
            } => match planned_active_run_id_for_thread(&self.inner, &thread_id).await {
                Some(active_run_id) if active_run_id == *expected_run_id => Some(active_run_id),
                Some(active_run_id) => {
                    return Err(BridgeError::SessionError(format!(
                        "durable replacement plan changed active run from {expected_run_id} to {active_run_id}",
                    )));
                }
                None => None,
            },
            DispatchExecutionMode::Durable { .. } => None,
        };
        if active_run_to_replace.is_some() {
            let mut interrupted = provider.interrupt_streaming_session(&thread_id).await;
            let mut aborted_runs = Vec::new();
            if !interrupted {
                let (aborted_any, run_ids) = self.abort_thread_runs(&thread_id).await;
                interrupted = aborted_any;
                aborted_runs = run_ids;
            }
            if !interrupted {
                return Err(BridgeError::SessionError(format!(
                    "thread {thread_id} already has an active run and the provider cannot accept follow-up input",
                )));
            }
            if !wait_for_thread_to_become_idle(
                &self.inner,
                &thread_id,
                FOLLOW_UP_INTERRUPT_WAIT_TIMEOUT,
            )
            .await
            {
                return Err(BridgeError::Timeout);
            }
            record_thread_log(
                thread_logs.clone(),
                thread_log_id.as_deref(),
                ThreadLogEvent::warn(
                    "",
                    "dispatch",
                    "interrupted active run to accept follow-up input",
                )
                .with_run_id(run_id.clone())
                .with_field("provider_key", json!(provider_key.clone()))
                .with_field("replaced_run_id", json!(active_run_to_replace))
                .with_field("aborted_runs", json!(aborted_runs))
                .with_field("message", json!(summarize_text(&message, 160))),
            )
            .await;
        }

        let run_permit = self
            .inner
            .run_limiter
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                BridgeError::Overloaded(format!(
                    "run concurrency limit reached ({})",
                    self.inner.max_concurrent_runs
                ))
            })?;
        if let Some(lease) = run_lease.as_mut() {
            lease
                .promote_to_active()
                .map_err(|error| BridgeError::SessionError(error.to_string()))?;
        }
        record_thread_log(
            thread_logs.clone(),
            thread_log_id.as_deref(),
            ThreadLogEvent::info("", "dispatch", "run accepted")
                .with_run_id(&run_id)
                .with_field("provider_key", json!(provider_key.clone()))
                .with_field("message", json!(summarize_text(&message, 160)))
                .with_field(
                    "image_count",
                    json!(images.as_ref().map(|value| value.len()).unwrap_or(0)),
                ),
        )
        .await;
        mark_task_in_progress_on_work_run_wake(
            &self.inner,
            &thread_id,
            &run_id,
            &metadata,
            thread_logs.clone(),
            thread_log_id.as_deref(),
        )
        .await;
        let run_id_owned = run_id.to_owned();
        let thread_id_owned = thread_id.to_owned();
        let provider_key_owned = provider_key;

        let mut options = ProviderRunOptions {
            thread_id: thread_id.to_owned(),
            message: message.to_owned(),
            workspace_dir: effective_workspace_dir,
            images: None,
            metadata,
        };
        options
            .metadata
            .insert("bridge_run_id".to_owned(), Value::String(run_id.to_owned()));
        if let Some(payloads) = images {
            options.images = if payloads.is_empty() {
                None
            } else {
                Some(payloads)
            };
        }

        // Load the persisted sdk_session_id for the active provider only.
        // This avoids leaking a different provider's session/thread id across
        // provider switches on the same Garyx thread.
        if let Some(store) = &*self.inner.thread_store.read().await
            && let Some(session_data) = store.get_logged(&thread_id).await
        {
            let resolved_provider_type = provider.provider_type();
            attach_provider_sdk_session_metadata(
                &mut options,
                &session_data,
                &provider_key_owned,
                &resolved_provider_type,
            );
        }

        let runtime_selection = provider.resolve_runtime_selection(&options);
        persist_thread_runtime_snapshot(
            self.inner.thread_store.read().await.clone(),
            &thread_id,
            &runtime_selection,
        )
        .await;

        let run_started_at = chrono::Utc::now().to_rfc3339();
        let partial_user_images = options.images.clone().unwrap_or_default();
        let partial_metadata = options.metadata.clone();
        let partial_provider_key = provider_key_owned.clone();
        let partial_provider_type = provider.provider_type();
        let partial_gateway_event_tx = self.inner.event_tx.read().await.clone();
        let partial_thread_store = self.inner.thread_store.read().await.clone();
        let partial_thread_history = self.inner.thread_history.read().await.clone();
        drop(thread_dispatch_guard);

        let inner = self.inner.clone();
        let user_message = message.to_owned();
        let thread_log_id_owned = thread_log_id.clone();
        let thread_logs_for_task = thread_logs.clone();
        let final_external_callback = response_callback.clone();
        let final_gateway_event_tx = gateway_event_tx.clone();

        // Publish the active indexes, persistence handle, and task ownership
        // as one cancellation-free section. Every await happens before the
        // persistence worker is spawned or any runtime index is mutated;
        // once all guards are held there is no suspension point until the
        // spawned task owns the promoted lease. Cancelling the dispatch
        // future before this point therefore drops the lease without leaking
        // an index or persistence worker, while DELETE can spin on the visible
        // active lease until this handoff completes.
        let mut run_index = self.inner.run_index.write().await;
        let mut thread_affinity = self.inner.thread_affinity.write().await;
        let mut active_thread_persistence = self.inner.active_thread_persistence.lock().await;
        let mut active_tasks = self.inner.active_tasks.lock().await;
        if let Some(lease) = run_lease.as_ref() {
            lease
                .ensure_valid()
                .map_err(|error| BridgeError::SessionError(error.to_string()))?;
        }
        let (partial_persistence_tx, partial_persistence_task) = partial_thread_store
            .zip(partial_thread_history)
            .map(|(store, history)| {
                spawn_partial_thread_persistence_worker(
                    store,
                    history,
                    thread_id.to_owned(),
                    message.to_owned(),
                    run_started_at.clone(),
                    partial_user_images,
                    partial_provider_key,
                    partial_provider_type,
                    partial_metadata,
                    partial_gateway_event_tx,
                )
            })
            .unzip();
        let response_callback = Some(build_streaming_response_callback(
            thread_logs.clone(),
            thread_log_id.clone(),
            run_id.to_owned(),
            response_callback.clone(),
            partial_persistence_tx.clone(),
            &RUN_STREAM_LOG_LABELS,
        ));

        run_index
            .active_runs
            .insert(run_id.to_owned(), provider_key_owned.clone());
        run_index
            .run_sessions
            .insert(run_id.to_owned(), thread_id.to_owned());
        thread_affinity.insert(thread_id.to_owned(), provider_key_owned.clone());
        if let Some(tx) = partial_persistence_tx.clone() {
            active_thread_persistence.insert(
                thread_id.to_owned(),
                ActiveThreadPersistence {
                    run_id: run_id.to_owned(),
                    tx,
                },
            );
        }

        let run_lifecycle_guard = RunLifecycleTerminalGuard::started(
            self.inner.run_lifecycle_tx.clone(),
            thread_id.to_owned(),
            run_id.to_owned(),
        );
        let task: JoinHandle<()> = tokio::spawn(async move {
            let _run_lifecycle_guard = run_lifecycle_guard;
            let _run_lease = run_lease;
            let _permit = run_permit;
            let mut graph_state = RunGraphState::new(
                run_id_owned.clone(),
                thread_id_owned.clone(),
                provider_key_owned.clone(),
                options,
            );
            record_thread_log(
                thread_logs_for_task.clone(),
                thread_log_id_owned.as_deref(),
                ThreadLogEvent::info("", "run", "agent run started")
                    .with_run_id(run_id_owned.clone())
                    .with_field("provider_key", json!(provider_key_owned.clone()))
                    .with_field("thread_id", json!(thread_id_owned.clone())),
            )
            .await;

            let result =
                execute_agent_run(provider.as_ref(), &mut graph_state, response_callback).await;
            let persistence_result = finalize_partial_persistence(
                &inner.active_thread_persistence,
                &thread_id_owned,
                &run_id_owned,
                partial_persistence_tx,
                partial_persistence_task,
                "partial thread persistence task failed",
            )
            .await;

            match &result {
                Ok(res) => {
                    if let Some(actual_model) = res.actual_model.as_ref() {
                        tracing::info!(
                            run_id = %run_id_owned,
                            success = res.success,
                            actual_model = %actual_model,
                            duration_ms = graph_state.metrics.duration_ms(),
                            cost_usd = graph_state.metrics.cost_usd,
                            "agent run completed via run graph",
                        );
                    } else {
                        tracing::info!(
                            run_id = %run_id_owned,
                            success = res.success,
                            duration_ms = graph_state.metrics.duration_ms(),
                            cost_usd = graph_state.metrics.cost_usd,
                            "agent run completed via run graph",
                        );
                    }
                    if !res.success {
                        tracing::warn!(
                            run_id = %run_id_owned,
                            error = %res.error.as_deref().unwrap_or("unknown error"),
                            "agent run failed via run graph",
                        );
                    }

                    // Persist user + assistant + tool messages to thread store.
                    let mut applied_thread_title: Option<String> = None;
                    if let (Some(store), Some(history)) = (
                        &*inner.thread_store.read().await,
                        &*inner.thread_history.read().await,
                    ) {
                        let (applied, terminal_committed) = persist_terminal_success(
                            store,
                            history,
                            provider.as_ref(),
                            &thread_id_owned,
                            &user_message,
                            &provider_key_owned,
                            &run_started_at,
                            &graph_state,
                            res,
                            persistence_result.as_ref(),
                        )
                        .await;
                        applied_thread_title = applied;
                        emit_committed_records(
                            &final_gateway_event_tx,
                            &thread_id_owned,
                            Some(&run_id_owned),
                            terminal_committed,
                        );
                        record_thread_log(
                            thread_logs_for_task.clone(),
                            thread_log_id_owned.as_deref(),
                            ThreadLogEvent::info("", "persistence", "thread messages persisted")
                                .with_run_id(run_id_owned.clone())
                                .with_field("thread_id", json!(thread_id_owned.clone()))
                                .with_field("response", json!(summarize_text(&res.response, 160))),
                        )
                        .await;
                    }
                    forward_applied_thread_title_update(
                        final_external_callback.as_ref(),
                        applied_thread_title.as_deref(),
                    );
                    let mut completed_event =
                        ThreadLogEvent::info("", "run", "agent run completed")
                            .with_run_id(run_id_owned.clone())
                            .with_field("success", json!(res.success))
                            .with_field("duration_ms", json!(graph_state.metrics.duration_ms()))
                            .with_field("cost_usd", json!(graph_state.metrics.cost_usd))
                            .with_field("input_tokens", json!(graph_state.metrics.input_tokens))
                            .with_field("output_tokens", json!(graph_state.metrics.output_tokens))
                            .with_field("response", json!(summarize_text(&res.response, 160)));
                    if let Some(actual_model) = res.actual_model.as_ref() {
                        completed_event =
                            completed_event.with_field("actual_model", json!(actual_model));
                    }
                    if let Some(error) = res
                        .error
                        .as_deref()
                        .map(str::trim)
                        .filter(|error| !error.is_empty())
                    {
                        completed_event = completed_event.with_field("error", json!(error));
                    }
                    record_thread_log(
                        thread_logs_for_task.clone(),
                        thread_log_id_owned.as_deref(),
                        completed_event,
                    )
                    .await;

                    let task_handoff = if res.success {
                        final_task_handoff_for_stopped_run(
                            &inner,
                            &thread_id_owned,
                            &run_id_owned,
                            &res.response,
                        )
                        .await
                    } else {
                        non_empty_trimmed_owned(&res.response)
                    };
                    mark_task_ready_for_review_after_stopped_run(
                        &inner,
                        &thread_id_owned,
                        &run_id_owned,
                        task_handoff.as_deref(),
                        res.success,
                        thread_logs_for_task.clone(),
                        thread_log_id_owned.as_deref(),
                    )
                    .await;

                    if !res.success {
                        record_thread_log(
                            thread_logs_for_task.clone(),
                            thread_log_id_owned.as_deref(),
                            ThreadLogEvent::info(
                                "",
                                "task",
                                "unsuccessful task run left in progress for retry",
                            )
                            .with_run_id(run_id_owned.clone())
                            .with_field("thread_id", json!(thread_id_owned.clone()))
                            .with_field("response", json!(summarize_text(&res.response, 160))),
                        )
                        .await;
                    }
                }
                Err(e) => {
                    tracing::error!(
                        run_id = %run_id_owned,
                        error = %e,
                        phase = ?graph_state.phase,
                        "agent run failed via run graph",
                    );

                    let failed_assistant_response = persistence_result
                        .as_ref()
                        .map(|value| value.assistant_response.as_str())
                        .unwrap_or_default();
                    if let (Some(store), Some(history)) = (
                        &*inner.thread_store.read().await,
                        &*inner.thread_history.read().await,
                    ) {
                        let terminal_committed = persist_terminal_failure(
                            store,
                            history,
                            provider.as_ref(),
                            &thread_id_owned,
                            &user_message,
                            &provider_key_owned,
                            &run_started_at,
                            &graph_state,
                            e,
                            persistence_result.as_ref(),
                        )
                        .await;
                        emit_committed_records(
                            &final_gateway_event_tx,
                            &thread_id_owned,
                            Some(&run_id_owned),
                            terminal_committed,
                        );
                        record_thread_log(
                            thread_logs_for_task.clone(),
                            thread_log_id_owned.as_deref(),
                            ThreadLogEvent::info(
                                "",
                                "persistence",
                                "failed run messages finalized",
                            )
                            .with_run_id(run_id_owned.clone())
                            .with_field("thread_id", json!(thread_id_owned.clone()))
                            .with_field(
                                "response",
                                json!(summarize_text(failed_assistant_response, 160)),
                            ),
                        )
                        .await;
                    }

                    record_thread_log(
                        thread_logs_for_task.clone(),
                        thread_log_id_owned.as_deref(),
                        ThreadLogEvent::error("", "run", "agent run failed")
                            .with_run_id(run_id_owned.clone())
                            .with_field("phase", json!(format!("{:?}", graph_state.phase)))
                            .with_field("error", json!(e.to_string())),
                    )
                    .await;

                    mark_task_ready_for_review_after_stopped_run(
                        &inner,
                        &thread_id_owned,
                        &run_id_owned,
                        None,
                        false,
                        thread_logs_for_task.clone(),
                        thread_log_id_owned.as_deref(),
                    )
                    .await;

                    record_thread_log(
                        thread_logs_for_task.clone(),
                        thread_log_id_owned.as_deref(),
                        ThreadLogEvent::info(
                            "",
                            "task",
                            "failed task run left in progress for retry",
                        )
                        .with_run_id(run_id_owned.clone())
                        .with_field("thread_id", json!(thread_id_owned.clone()))
                        .with_field(
                            "response",
                            json!(summarize_text(failed_assistant_response, 160)),
                        ),
                    )
                    .await;
                }
            }

            // Cleanup.
            let mut run_index = inner.run_index.write().await;
            run_index.active_runs.remove(&run_id_owned);
            run_index.run_sessions.remove(&run_id_owned);
            drop(run_index);
            let mut persistence = inner.active_thread_persistence.lock().await;
            let should_remove = persistence
                .get(&thread_id_owned)
                .map(|handle| handle.run_id == run_id_owned.as_str())
                .unwrap_or(false);
            if should_remove {
                persistence.remove(&thread_id_owned);
            }
            inner.active_tasks.lock().await.remove(&run_id_owned);
        });

        active_tasks.insert(run_id.to_owned(), task);
        drop(active_tasks);
        drop(active_thread_persistence);
        drop(thread_affinity);
        drop(run_index);
        Ok(AgentDispatchOutcome::Started)
    }

    /// Raw helper retained only for this crate's unit tests. Production
    /// callers must cross the sealed `AdmittedRun` dispatcher boundary.
    #[cfg(test)]
    pub async fn start_agent_run(
        &self,
        request: AgentRunRequest,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<AgentDispatchOutcome, BridgeError> {
        self.start_admitted_run(request, None, response_callback)
            .await
    }

    /// Run a thread-bound turn inline through the bridge's normal
    /// thread execution path.
    ///
    /// Unlike `start_agent_run`, this helper awaits completion and persists the
    /// thread's transcript/provider state before returning the
    /// `ProviderRunResult`.
    #[cfg(test)]
    pub(crate) async fn run_inline_streaming(
        &self,
        thread_id: &str,
        message: &str,
        mut metadata: HashMap<String, Value>,
        images: Option<Vec<ImagePayload>>,
        workspace_dir: Option<String>,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<ProviderRunResult, BridgeError> {
        let thread_dispatch_guard = {
            let mut guards = self.inner.thread_dispatch_guards.lock().await;
            guards
                .entry(thread_id.to_owned())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let thread_dispatch_guard = thread_dispatch_guard.lock().await;

        if has_active_streaming_run_for_thread(&self.inner, thread_id).await {
            return Err(BridgeError::Overloaded(format!(
                "thread {thread_id} already has an active run"
            )));
        }

        scrub_inline_run_runtime_metadata(&mut metadata);
        if let Some(store) = self.inner.thread_store.read().await.clone()
            && let Some(thread_data) = store.get_logged(thread_id).await
        {
            for (key, value) in thread_metadata_from_value(&thread_data) {
                metadata.entry(key).or_insert(value);
            }
        }
        let thread_log_id = resolve_thread_log_thread_id(thread_id, &metadata);
        let thread_logs = self.thread_log_sink();
        let effective_workspace_dir =
            resolve_effective_workspace_dir(&self.inner, thread_id, workspace_dir).await?;
        let (provider_key, provider, _) = self
            .resolve_thread_execution_target(thread_id, "api", "inline", &mut metadata, None)
            .await?;
        record_thread_log(
            thread_logs.clone(),
            thread_log_id.as_deref(),
            ThreadLogEvent::info("", "run", "sub-agent provider resolved")
                .with_field("provider_key", json!(provider_key.clone()))
                .with_field("thread_id", json!(thread_id)),
        )
        .await;

        let run_permit = self
            .inner
            .run_limiter
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                BridgeError::Overloaded(format!(
                    "run concurrency limit reached ({})",
                    self.inner.max_concurrent_runs
                ))
            })?;

        let run_id = format!("subagent-run-{}", uuid::Uuid::new_v4());
        {
            let mut run_index = self.inner.run_index.write().await;
            run_index
                .active_runs
                .insert(run_id.clone(), provider_key.clone());
            run_index
                .run_sessions
                .insert(run_id.clone(), thread_id.to_owned());
        }
        self.inner
            .thread_affinity
            .write()
            .await
            .insert(thread_id.to_owned(), provider_key.clone());

        let mut options = ProviderRunOptions {
            thread_id: thread_id.to_owned(),
            message: message.to_owned(),
            workspace_dir: effective_workspace_dir,
            images: None,
            metadata,
        };
        options
            .metadata
            .insert("bridge_run_id".to_owned(), Value::String(run_id.clone()));
        if let Some(payloads) = images {
            options.images = if payloads.is_empty() {
                None
            } else {
                Some(payloads)
            };
        }

        if let Some(store) = &*self.inner.thread_store.read().await
            && let Some(session_data) = store.get_logged(thread_id).await
        {
            let resolved_provider_type = provider.provider_type();
            attach_provider_sdk_session_metadata(
                &mut options,
                &session_data,
                &provider_key,
                &resolved_provider_type,
            );
        }

        let runtime_selection = provider.resolve_runtime_selection(&options);
        persist_thread_runtime_snapshot(
            self.inner.thread_store.read().await.clone(),
            thread_id,
            &runtime_selection,
        )
        .await;

        let run_started_at = chrono::Utc::now().to_rfc3339();
        let partial_user_images = options.images.clone().unwrap_or_default();
        let partial_metadata = options.metadata.clone();
        let partial_provider_key = provider_key.clone();
        let partial_provider_type = provider.provider_type();
        let partial_gateway_event_tx = self.inner.event_tx.read().await.clone();
        let partial_thread_store = self.inner.thread_store.read().await.clone();
        let partial_thread_history = self.inner.thread_history.read().await.clone();
        let (partial_persistence_tx, partial_persistence_task) = partial_thread_store
            .zip(partial_thread_history)
            .map(|(store, history)| {
                spawn_partial_thread_persistence_worker(
                    store,
                    history,
                    thread_id.to_owned(),
                    message.to_owned(),
                    run_started_at.clone(),
                    partial_user_images,
                    partial_provider_key,
                    partial_provider_type,
                    partial_metadata,
                    partial_gateway_event_tx,
                )
            })
            .unzip();
        if let Some(tx) = partial_persistence_tx.clone() {
            self.inner.active_thread_persistence.lock().await.insert(
                thread_id.to_owned(),
                ActiveThreadPersistence {
                    run_id: run_id.clone(),
                    tx,
                },
            );
        }

        drop(thread_dispatch_guard);
        let _run_permit = run_permit;

        let external_callback = response_callback.clone();
        let final_external_callback = external_callback.clone();
        let response_callback = Some(build_streaming_response_callback(
            thread_logs.clone(),
            thread_log_id.clone(),
            run_id.clone(),
            external_callback,
            partial_persistence_tx.clone(),
            &SUBAGENT_STREAM_LOG_LABELS,
        ));

        let mut graph_state = RunGraphState::new(
            run_id.clone(),
            thread_id.to_owned(),
            provider_key.clone(),
            options,
        );
        let result =
            execute_agent_run(provider.as_ref(), &mut graph_state, response_callback).await;

        let persistence_result = finalize_partial_persistence(
            &self.inner.active_thread_persistence,
            thread_id,
            &run_id,
            partial_persistence_tx,
            partial_persistence_task,
            "sub-agent partial thread persistence task failed",
        )
        .await;

        let outcome = match &result {
            Ok(res) => {
                let mut applied_thread_title: Option<String> = None;
                if let (Some(store), Some(history)) = (
                    &*self.inner.thread_store.read().await,
                    &*self.inner.thread_history.read().await,
                ) {
                    let (applied, terminal_committed) = persist_terminal_success(
                        store,
                        history,
                        provider.as_ref(),
                        thread_id,
                        message,
                        &provider_key,
                        &run_started_at,
                        &graph_state,
                        res,
                        persistence_result.as_ref(),
                    )
                    .await;
                    applied_thread_title = applied;
                    let terminal_event_tx = self.inner.event_tx.read().await.clone();
                    emit_committed_records(
                        &terminal_event_tx,
                        thread_id,
                        Some(&run_id),
                        terminal_committed,
                    );
                }
                forward_applied_thread_title_update(
                    final_external_callback.as_ref(),
                    applied_thread_title.as_deref(),
                );
                Ok(res.clone())
            }
            Err(error) => {
                if let (Some(store), Some(history)) = (
                    &*self.inner.thread_store.read().await,
                    &*self.inner.thread_history.read().await,
                ) {
                    let terminal_committed = persist_terminal_failure(
                        store,
                        history,
                        provider.as_ref(),
                        thread_id,
                        message,
                        &provider_key,
                        &run_started_at,
                        &graph_state,
                        error,
                        persistence_result.as_ref(),
                    )
                    .await;
                    let terminal_event_tx = self.inner.event_tx.read().await.clone();
                    emit_committed_records(
                        &terminal_event_tx,
                        thread_id,
                        Some(&run_id),
                        terminal_committed,
                    );
                }

                Err(error.clone())
            }
        };

        let mut run_index = self.inner.run_index.write().await;
        run_index.active_runs.remove(&run_id);
        run_index.run_sessions.remove(&run_id);
        drop(run_index);

        outcome
    }

    /// Queue a message to an existing streaming thread, delegating to the
    /// provider that has affinity for this session.
    pub async fn add_streaming_input(
        &self,
        thread_id: &str,
        message: &str,
        images: Option<Vec<ImagePayload>>,
        files: Option<Vec<FilePayload>>,
        attachments: Option<Vec<PromptAttachment>>,
        client_intent_id: Option<String>,
    ) -> Option<QueuedStreamingInput> {
        self.add_streaming_input_with_metadata(
            thread_id,
            message,
            images,
            files,
            attachments,
            client_intent_id,
            HashMap::new(),
        )
        .await
    }

    /// [`Self::add_streaming_input`] carrying attribution metadata from the
    /// originating dispatch (e.g. `source`/`automation_id` when a scheduled
    /// turn is queued into an already-active run); merged into the
    /// acknowledged user record.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_streaming_input_with_metadata(
        &self,
        thread_id: &str,
        message: &str,
        images: Option<Vec<ImagePayload>>,
        files: Option<Vec<FilePayload>>,
        attachments: Option<Vec<PromptAttachment>>,
        client_intent_id: Option<String>,
        origin_metadata: HashMap<String, Value>,
    ) -> Option<QueuedStreamingInput> {
        self.add_streaming_input_with_metadata_mode(
            thread_id,
            message,
            images,
            files,
            attachments,
            client_intent_id,
            origin_metadata,
            None,
            true,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn add_streaming_input_with_metadata_exact(
        &self,
        thread_id: &str,
        message: &str,
        images: Option<Vec<ImagePayload>>,
        files: Option<Vec<FilePayload>>,
        attachments: Option<Vec<PromptAttachment>>,
        client_intent_id: Option<String>,
        origin_metadata: HashMap<String, Value>,
        pending_input_id: String,
    ) -> Option<QueuedStreamingInput> {
        self.add_streaming_input_with_metadata_mode(
            thread_id,
            message,
            images,
            files,
            attachments,
            client_intent_id,
            origin_metadata,
            Some(pending_input_id),
            false,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn add_streaming_input_with_metadata_mode(
        &self,
        thread_id: &str,
        message: &str,
        images: Option<Vec<ImagePayload>>,
        files: Option<Vec<FilePayload>>,
        attachments: Option<Vec<PromptAttachment>>,
        client_intent_id: Option<String>,
        origin_metadata: HashMap<String, Value>,
        pending_input_id: Option<String>,
        allow_retry: bool,
    ) -> Option<QueuedStreamingInput> {
        let provider_key = self
            .inner
            .thread_affinity
            .read()
            .await
            .get(thread_id)
            .cloned();
        if let Some(key) = provider_key
            && let Some(provider) = self.get_provider(&key).await
        {
            let active_run_id = active_run_id_for_thread(&self.inner, thread_id).await;
            let persistence_handle = self
                .inner
                .active_thread_persistence
                .lock()
                .await
                .get(thread_id)
                .cloned();
            let run_id = persistence_handle
                .as_ref()
                .map(|handle| handle.run_id.clone())
                .or(active_run_id)
                .unwrap_or_default();
            let persistence_tx = persistence_handle.as_ref().map(|handle| handle.tx.clone());

            let image_payloads = images.clone().unwrap_or_default();
            let file_payloads = files.clone().unwrap_or_default();
            let mut staged_attachments = attachments.clone().unwrap_or_default();
            staged_attachments.extend(stage_image_payloads_for_prompt(
                "garyx-bridge",
                &image_payloads,
            ));
            staged_attachments.extend(stage_file_payloads_for_prompt(
                "garyx-bridge",
                &file_payloads,
            ));
            let provider_message =
                render_streaming_user_message_for_provider(&self.inner, thread_id, message).await;
            // Pending inputs live in the thread record while the provider has
            // not ACKed them. Apply the same runtime-only filter used by the
            // direct transcript path at this persistence boundary as well as
            // at the dispatch projection above, so every caller is safe.
            let pending_input = PendingUserInput {
                id: pending_input_id
                    .unwrap_or_else(|| format!("queued_input:{}", uuid::Uuid::new_v4())),
                bridge_run_id: run_id.clone(),
                text: message.to_owned(),
                content: build_pending_input_content(message, &image_payloads, &staged_attachments),
                queued_at: chrono::Utc::now().to_rfc3339(),
                origin_id: client_intent_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                metadata: pending_input_metadata_for_persistence(origin_metadata),
                status: PendingUserInputStatus::Queued,
            };
            if let Some(tx) = persistence_tx.as_ref() {
                let _ = tx.send(ThreadPersistenceCommand::QueuePendingInput(
                    pending_input.clone(),
                ));
            }

            let provider_input = QueuedUserInput {
                pending_input_id: Some(pending_input.id.clone()),
                message: provider_message,
                images: if staged_attachments.is_empty() {
                    image_payloads
                } else {
                    Vec::new()
                },
                attachments: staged_attachments,
            };
            let accepted = if allow_retry {
                queue_streaming_input_with_retry(
                    &self.inner,
                    provider.clone(),
                    thread_id,
                    provider_input,
                )
                .await
            } else {
                provider
                    .add_streaming_input(thread_id, provider_input)
                    .await
            };
            if accepted {
                return Some(QueuedStreamingInput {
                    pending_input_id: pending_input.id,
                    run_id,
                });
            }

            if allow_retry && let Some(tx) = persistence_tx {
                let _ = tx.send(ThreadPersistenceCommand::DropPendingInput {
                    pending_input_id: pending_input.id,
                });
            }
            return None;
        }
        None
    }

    /// Interrupt a streaming thread gracefully via the provider.
    pub async fn interrupt_streaming_session(&self, thread_id: &str) -> bool {
        let provider_key = self
            .inner
            .thread_affinity
            .read()
            .await
            .get(thread_id)
            .cloned();
        if let Some(key) = provider_key
            && let Some(provider) = self.get_provider(&key).await
        {
            return provider.interrupt_streaming_session(thread_id).await;
        }
        false
    }

    /// Abort a running agent request.
    pub async fn abort_run(&self, run_id: &str) -> bool {
        let (provider_key, thread_id_for_run) = {
            let run_index = self.inner.run_index.read().await;
            (
                run_index.active_runs.get(run_id).cloned(),
                run_index.run_sessions.get(run_id).cloned(),
            )
        };

        // A promoted lease is published to `run_index` before the spawned
        // JoinHandle is inserted into `active_tasks`. DELETE may arrive in
        // that narrow window. Keep the index intact and let the coordinator
        // retry instead of treating an early provider abort as proof that a
        // task which has not started yet can no longer start.
        if provider_key.is_some() && !self.inner.active_tasks.lock().await.contains_key(run_id) {
            tokio::task::yield_now().await;
            return false;
        }

        let provider = match provider_key.as_ref() {
            Some(provider_key) => self
                .inner
                .topology
                .read()
                .await
                .provider_pool
                .get(provider_key)
                .cloned(),
            None => None,
        };

        let persistence_handle = if let Some(thread_id) = thread_id_for_run.as_deref() {
            let mut persistence = self.inner.active_thread_persistence.lock().await;
            let should_remove = persistence
                .get(thread_id)
                .map(|handle| handle.run_id == run_id)
                .unwrap_or(false);
            if should_remove {
                persistence.remove(thread_id)
            } else {
                None
            }
        } else {
            None
        };
        let mut abort_terminal_ack = None;
        if let Some(handle) = persistence_handle.as_ref() {
            let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
            if handle
                .tx
                .send(ThreadPersistenceCommand::AbortTerminal {
                    error: Some("aborted".to_owned()),
                    ack: ack_tx,
                })
                .is_ok()
            {
                abort_terminal_ack = Some(ack_rx);
            }
        }

        // Stdio transports that need to flush a protocol-native cancel frame
        // get a bounded provider abort while the owning task is still alive.
        let provider_aborted_before_task = if let Some(provider) = provider.as_ref()
            && provider.abort_before_task_cancel()
        {
            provider.abort(run_id).await
        } else {
            false
        };

        // Cancel the tokio task.
        let cancelled_task = {
            let mut tasks = self.inner.active_tasks.lock().await;
            if let Some(task) = tasks.remove(run_id) {
                task.abort();
                Some(task)
            } else {
                None
            }
        };
        let task_cancelled = cancelled_task.is_some();
        if let Some(task) = cancelled_task {
            // `abort` is only a request. Await the JoinHandle so destructive
            // thread mutation cannot race task-local cleanup or provider use.
            let _ = task.await;
        }

        // Also try provider-level abort for providers that do not need the
        // pre-drop ordering above.
        let provider_aborted = if provider_aborted_before_task {
            true
        } else if let Some(provider) = provider {
            provider.abort(run_id).await
        } else {
            false
        };

        // Cleanup tracking state.
        {
            let mut run_index = self.inner.run_index.write().await;
            run_index.active_runs.remove(run_id);
            run_index.run_sessions.remove(run_id);
        }

        drop(persistence_handle);
        if let Some(ack) = abort_terminal_ack {
            let _ = ack.await;
        }

        task_cancelled || provider_aborted
    }

    /// Abort all runs for a given thread.
    pub async fn abort_thread_runs(&self, thread_id: &str) -> (bool, Vec<String>) {
        let run_ids: Vec<String> = {
            self.inner
                .run_index
                .read()
                .await
                .run_sessions
                .iter()
                .filter(|(_, sk)| sk.as_str() == thread_id)
                .map(|(rid, _)| rid.clone())
                .collect()
        };

        let mut aborted = Vec::new();
        for run_id in &run_ids {
            if self.abort_run(run_id).await {
                aborted.push(run_id.clone());
            }
        }

        (!aborted.is_empty(), aborted)
    }

    /// Abort every active task for a thread and wait until the run index no
    /// longer exposes an active run. A promoted lease may briefly precede the
    /// run-index insertion; in that case retry instead of missing the task.
    pub async fn abort_thread_runs_and_wait(&self, thread_id: &str) {
        loop {
            let _ = self.abort_thread_runs(thread_id).await;
            let still_indexed = self
                .inner
                .run_index
                .read()
                .await
                .run_sessions
                .values()
                .any(|value| value == thread_id);
            let promoted_without_index = if still_indexed {
                false
            } else {
                self.inner
                    .thread_store
                    .read()
                    .await
                    .as_ref()
                    .is_some_and(|store| store.run_coordinator().has_active_lease(thread_id))
            };
            if !still_indexed && !promoted_without_index {
                return;
            }
            tokio::task::yield_now().await;
        }
    }

    /// Abort every active run across all threads. Called on graceful shutdown
    /// so a clean restart writes close controls and leaves no orphaned
    /// `running` projection behind; the startup reconcile only needs to back up
    /// true hard crashes (SIGKILL / power loss).
    pub async fn abort_all_active_runs(&self) -> Vec<String> {
        let run_ids = self.get_active_runs().await;
        let mut aborted = Vec::new();
        for run_id in &run_ids {
            if self.abort_run(run_id).await {
                aborted.push(run_id.clone());
            }
        }
        aborted
    }

    /// Get list of active run IDs.
    pub async fn get_active_runs(&self) -> Vec<String> {
        self.inner
            .run_index
            .read()
            .await
            .active_runs
            .keys()
            .cloned()
            .collect()
    }

    /// Check if a run is still active.
    pub async fn is_run_active(&self, run_id: &str) -> bool {
        self.inner
            .run_index
            .read()
            .await
            .active_runs
            .contains_key(run_id)
    }
}

#[cfg(test)]
mod tests;
