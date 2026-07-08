use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use garyx_models::provider::{
    AgentRunRequest, FORK_FROM_PROVIDER_TYPE_METADATA_KEY, FORK_FROM_SDK_SESSION_ID_METADATA_KEY,
    FilePayload, ImagePayload, MODEL_METADATA_KEY, MODEL_OVERRIDE_METADATA_KEY,
    MODEL_REASONING_EFFORT_METADATA_KEY, MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY,
    MODEL_SERVICE_TIER_METADATA_KEY, MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY, PromptAttachment,
    ProviderMessage, ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput,
    SDK_SESSION_FORK_METADATA_KEY, SDK_SESSION_ID_METADATA_KEY, StreamEvent,
    attachments_from_metadata, build_user_content_from_parts, stage_file_payloads_for_prompt,
    stage_image_payloads_for_prompt,
};
use garyx_models::thread_logs::{ThreadLogEvent, ThreadLogSink, resolve_thread_log_thread_id};
use garyx_models::{Principal, final_assistant_text_from_render_records};
use garyx_router::{
    ThreadHistoryRepository, ThreadStore, mark_thread_task_in_progress_on_wake,
    mark_thread_task_in_review_if_in_progress, thread_metadata_from_value,
};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep};

use crate::provider_trait::{AgentLoopProvider, BridgeError, ProviderRuntimeSelection};
use crate::run_graph::{RunGraphState, execute_agent_run};

use super::MultiProviderBridge;
use super::persistence::{
    MAX_SESSION_MESSAGES, PendingUserInput, PendingUserInputStatus, PersistedRun,
    RunControlRecord, StreamingRunSnapshot, TerminalRunControl, ThreadPersistenceCommand,
    capsule_attached_control_record, save_failed_thread_messages_with_terminal_control,
    save_streaming_partial, save_thread_messages_with_terminal_control,
};
use super::state::{self, ActiveThreadPersistence};
use crate::garyx_native_provider::SESSION_MESSAGES_METADATA_KEY;

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

fn metadata_string(metadata: &HashMap<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
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

fn scrub_subagent_runtime_metadata(metadata: &mut HashMap<String, Value>) {
    // AgentTeam child runs inherit caller metadata for runtime context, but
    // thread-bound identity/provider fields must be re-derived from the child
    // thread itself so the leaf provider sees the sub-agent's own persona and
    // session state rather than the parent group's.
    for key in [
        "agent_id",
        "agent_display_name",
        "model",
        "model_reasoning_effort",
        "system_prompt",
        "requested_provider_type",
        SDK_SESSION_ID_METADATA_KEY,
        SDK_SESSION_FORK_METADATA_KEY,
        "agent_team_id",
        "group_transcript_snapshot",
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
    provider: Arc<dyn AgentLoopProvider>,
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

fn metadata_bool(metadata: &HashMap<String, Value>, key: &str) -> bool {
    metadata.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn metadata_string_is_present(metadata: &HashMap<String, Value>, key: &str) -> bool {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
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
    let session_data = store.get(thread_id).await?;
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

fn missing_agent_team_binding_error(
    thread_id: &str,
    metadata: &HashMap<String, Value>,
) -> BridgeError {
    let team_id = metadata
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown-team");
    BridgeError::SessionError(format!(
        "thread {thread_id} is bound to missing agent team {team_id}",
    ))
}

impl MultiProviderBridge {
    async fn resolve_thread_execution_target(
        &self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        metadata: &mut HashMap<String, Value>,
        requested_provider: Option<ProviderType>,
    ) -> Result<(String, Arc<dyn AgentLoopProvider>, Option<ProviderType>), BridgeError> {
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

        if requested_provider == Some(ProviderType::AgentTeam)
            && !metadata.contains_key("agent_team_id")
        {
            return Err(missing_agent_team_binding_error(thread_id, metadata));
        }

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

    /// Start an agent run and forward optional image payloads to providers.
    pub async fn start_agent_run(
        &self,
        request: AgentRunRequest,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<(), BridgeError> {
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
        let thread_dispatch_guard = {
            let mut guards = self.inner.thread_dispatch_guards.lock().await;
            guards
                .entry(thread_id.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let thread_dispatch_guard = thread_dispatch_guard.lock().await;
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
        if self
            .add_streaming_input(
                &thread_id,
                &message,
                images.clone(),
                None,
                queued_attachments,
                metadata_string(&metadata, "client_intent_id"),
            )
            .await
            .is_some()
        {
            tracing::info!(
                thread_id = %thread_id,
                run_id = %run_id,
                "queued message to existing streaming session"
            );
            record_thread_log(
                thread_logs.clone(),
                thread_log_id.as_deref(),
                ThreadLogEvent::info("", "dispatch", "queued message to active streaming session")
                    .with_run_id(&run_id)
                    .with_field("provider_key", json!(provider_key.clone()))
                    .with_field("message", json!(summarize_text(&message, 160))),
            )
            .await;
            return Ok(());
        }
        if has_active_streaming_run_for_thread(&self.inner, &thread_id).await {
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
        // Track active run.
        {
            let mut run_index = self.inner.run_index.write().await;
            run_index
                .active_runs
                .insert(run_id.to_owned(), provider_key.clone());
            run_index
                .run_sessions
                .insert(run_id.to_owned(), thread_id.to_owned());
        }
        self.inner
            .thread_affinity
            .write()
            .await
            .insert(thread_id.to_owned(), provider_key.clone());

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
            && let Some(session_data) = store.get(&thread_id).await
        {
            let resolved_provider_type = provider.provider_type();
            let history = self.inner.thread_history.read().await.clone();
            attach_native_session_messages(
                &mut options,
                history.as_ref(),
                &thread_id,
                &resolved_provider_type,
            )
            .await;
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
                    run_id: run_id.to_owned(),
                    tx,
                },
            );
        }
        drop(thread_dispatch_guard);

        let inner = self.inner.clone();
        let user_message = message.to_owned();
        let thread_log_id_owned = thread_log_id.clone();
        let thread_logs_for_task = thread_logs.clone();
        let final_external_callback = response_callback.clone();
        let final_gateway_event_tx = gateway_event_tx.clone();
        let response_callback = {
            let external_callback = response_callback.clone();
            let sink = thread_logs.clone();
            let thread_log_id = thread_log_id.clone();
            let run_id = run_id.to_owned();
            let first_token_logged = Arc::new(AtomicBool::new(false));
            let partial_persistence_tx = partial_persistence_tx.clone();
            Some(Arc::new(move |event: StreamEvent| {
                let sink = sink.clone();
                let thread_log_id = thread_log_id.clone();
                let run_id = run_id.clone();
                let external_callback = external_callback.clone();
                let first_token_logged = first_token_logged.clone();
                let event_for_log = event.clone();
                let event_for_emit = event.clone();
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
                            if !text.is_empty() && !first_token_logged.swap(true, Ordering::Relaxed)
                            {
                                record_thread_log(
                                    sink,
                                    thread_log_id.as_deref(),
                                    ThreadLogEvent::info("", "run", "first token received")
                                        .with_run_id(run_id),
                                )
                                .await;
                            }
                        }
                        StreamEvent::ToolUse { message } => {
                            record_thread_log(
                                sink,
                                thread_log_id.as_deref(),
                                ThreadLogEvent::info("", "tool", "tool use emitted")
                                    .with_run_id(run_id)
                                    .with_field("tool_name", json!(message.tool_name))
                                    .with_field("tool_use_id", json!(message.tool_use_id))
                                    .with_field(
                                        "message",
                                        json!(summarize_provider_message(&message)),
                                    ),
                            )
                            .await;
                        }
                        StreamEvent::ToolResult { message } => {
                            record_thread_log(
                                sink,
                                thread_log_id.as_deref(),
                                ThreadLogEvent::info("", "tool", "tool result emitted")
                                    .with_run_id(run_id)
                                    .with_field("tool_name", json!(message.tool_name))
                                    .with_field("tool_use_id", json!(message.tool_use_id))
                                    .with_field("is_error", json!(message.is_error))
                                    .with_field(
                                        "message",
                                        json!(summarize_provider_message(&message)),
                                    ),
                            )
                            .await;
                        }
                        StreamEvent::SessionBound { .. }
                        | StreamEvent::Boundary { .. }
                        | StreamEvent::ThreadTitleUpdated { .. }
                        | StreamEvent::Done => {}
                    }
                });

                if matches!(event_for_emit, StreamEvent::ThreadTitleUpdated { .. }) {
                    return;
                }

                if callback_after_commit.is_none()
                    && let Some(callback) = external_callback
                {
                    callback(event);
                }
            }) as Arc<dyn Fn(StreamEvent) + Send + Sync>)
        };

        let task: JoinHandle<()> = tokio::spawn(async move {
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
            // Drop the shared persistence sender before awaiting the worker.
            // Otherwise the worker keeps waiting for more commands and the run
            // never reaches final persistence/cleanup.
            let removed_persistence = {
                let mut persistence = inner.active_thread_persistence.lock().await;
                let should_remove = persistence
                    .get(&thread_id_owned)
                    .map(|handle| handle.run_id == run_id_owned.as_str())
                    .unwrap_or(false);
                if should_remove {
                    persistence.remove(&thread_id_owned)
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
                        tracing::warn!(
                            run_id = %run_id_owned,
                            error = %error,
                            "partial thread persistence task failed"
                        );
                        None
                    }
                }
            } else {
                None
            };
            {
                let mut persistence = inner.active_thread_persistence.lock().await;
                let should_remove = persistence
                    .get(&thread_id_owned)
                    .map(|handle| handle.run_id == run_id_owned.as_str())
                    .unwrap_or(false);
                if should_remove {
                    persistence.remove(&thread_id_owned);
                }
            }

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

                    // Record health metrics.
                    if res.success {
                        MultiProviderBridge::record_health_success(
                            &inner,
                            &provider_key_owned,
                            graph_state.metrics.duration_ms() as f64,
                        )
                        .await;
                    } else {
                        MultiProviderBridge::record_health_failure(
                            &inner,
                            &provider_key_owned,
                            res.error.as_deref().unwrap_or("unknown error"),
                        )
                        .await;
                    }

                    // Persist user + assistant + tool messages to thread store.
                    let mut applied_thread_title: Option<String> = None;
                    if let (Some(store), Some(history)) = (
                        &*inner.thread_store.read().await,
                        &*inner.thread_history.read().await,
                    ) {
                        let user_images =
                            graph_state.run_options.images.clone().unwrap_or_default();
                        let persisted_assistant_response = persistence_result
                            .as_ref()
                            .map(|value| value.assistant_response.as_str())
                            .filter(|value| !value.is_empty())
                            .unwrap_or(&res.response);
                        let persisted_session_messages = persistence_result
                            .as_ref()
                            .map(|value| value.session_messages.as_slice())
                            .filter(|value| !value.is_empty())
                            .unwrap_or(&res.session_messages);
                        let persisted_transcript_controls = persistence_result
                            .as_ref()
                            .map(|value| value.transcript_controls.as_slice())
                            .unwrap_or(&[]);
                        let sdk_session_id = resolve_sdk_session_id_for_persistence(
                            &graph_state.run_options.metadata,
                            res.sdk_session_id.as_deref(),
                        );
                        applied_thread_title = persist_provider_thread_title_if_missing(
                            store,
                            &thread_id_owned,
                            res.thread_title.as_deref(),
                        )
                        .await;
                        // Codex surfaces a usage-quota exhaustion as a soft
                        // failure (`Ok` result with `success == false`), so the
                        // rate-limit context is consumed here rather than on the
                        // hard-error path below.
                        let rate_limit = if res.success {
                            None
                        } else {
                            provider.take_rate_limit(&thread_id_owned).await
                        };
                        let terminal_committed = save_thread_messages_with_terminal_control(
                            store,
                            history,
                            PersistedRun {
                                thread_id: &thread_id_owned,
                                user_message: &user_message,
                                user_timestamp: Some(&run_started_at),
                                user_images: &user_images,
                                assistant_response: persisted_assistant_response,
                                sdk_session_id: sdk_session_id.as_deref(),
                                provider_key: &provider_key_owned,
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

                    // Record health failure.
                    MultiProviderBridge::record_health_failure(
                        &inner,
                        &provider_key_owned,
                        &e.to_string(),
                    )
                    .await;

                    let failed_assistant_response = persistence_result
                        .as_ref()
                        .map(|value| value.assistant_response.as_str())
                        .unwrap_or_default();
                    let failed_session_messages = persistence_result
                        .as_ref()
                        .map(|value| value.session_messages.as_slice())
                        .unwrap_or(&[]);
                    let persisted_transcript_controls = persistence_result
                        .as_ref()
                        .map(|value| value.transcript_controls.as_slice())
                        .unwrap_or(&[]);

                    if let (Some(store), Some(history)) = (
                        &*inner.thread_store.read().await,
                        &*inner.thread_history.read().await,
                    ) {
                        let user_images =
                            graph_state.run_options.images.clone().unwrap_or_default();
                        let rate_limit = provider.take_rate_limit(&thread_id_owned).await;
                        let terminal_committed = save_failed_thread_messages_with_terminal_control(
                            store,
                            history,
                            PersistedRun {
                                thread_id: &thread_id_owned,
                                user_message: &user_message,
                                user_timestamp: Some(&run_started_at),
                                user_images: &user_images,
                                assistant_response: failed_assistant_response,
                                sdk_session_id: None,
                                provider_key: &provider_key_owned,
                                provider_type: provider.provider_type(),
                                session_messages: failed_session_messages,
                                metadata: &graph_state.run_options.metadata,
                            },
                            persisted_transcript_controls,
                            Some(TerminalRunControl {
                                duration_ms: Some(graph_state.metrics.duration_ms()),
                                success: Some(false),
                                error: Some(e.to_string()),
                                thread_title: None,
                                rate_limit,
                            }),
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

        self.inner
            .active_tasks
            .lock()
            .await
            .insert(run_id.to_owned(), task);
        Ok(())
    }

    /// Run a thread-bound sub-agent turn inline through the bridge's normal
    /// thread execution path.
    ///
    /// Used by the AgentTeam meta-provider for child-thread dispatch. Unlike
    /// `start_agent_run`, this helper awaits completion and persists the child
    /// thread's transcript/provider state before returning the
    /// `ProviderRunResult`.
    pub async fn run_subagent_streaming(
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

        scrub_subagent_runtime_metadata(&mut metadata);
        if let Some(store) = self.inner.thread_store.read().await.clone()
            && let Some(thread_data) = store.get(thread_id).await
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
            .resolve_thread_execution_target(
                thread_id,
                "agent_team",
                "subagent",
                &mut metadata,
                None,
            )
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
            && let Some(session_data) = store.get(thread_id).await
        {
            let resolved_provider_type = provider.provider_type();
            let history = self.inner.thread_history.read().await.clone();
            attach_native_session_messages(
                &mut options,
                history.as_ref(),
                thread_id,
                &resolved_provider_type,
            )
            .await;
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
        let first_token_logged = Arc::new(AtomicBool::new(false));
        let response_callback = {
            let sink = thread_logs.clone();
            let thread_log_id = thread_log_id.clone();
            let run_id = run_id.clone();
            let first_token_logged = first_token_logged.clone();
            let partial_persistence_tx = partial_persistence_tx.clone();
            Some(Arc::new(move |event: StreamEvent| {
                let sink = sink.clone();
                let thread_log_id = thread_log_id.clone();
                let run_id = run_id.clone();
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
                            if !text.is_empty() && !first_token_logged.swap(true, Ordering::Relaxed)
                            {
                                record_thread_log(
                                    sink,
                                    thread_log_id.as_deref(),
                                    ThreadLogEvent::info(
                                        "",
                                        "run",
                                        "sub-agent first token received",
                                    )
                                    .with_run_id(run_id),
                                )
                                .await;
                            }
                        }
                        StreamEvent::ToolUse { message } => {
                            record_thread_log(
                                sink,
                                thread_log_id.as_deref(),
                                ThreadLogEvent::info("", "tool", "sub-agent tool use emitted")
                                    .with_run_id(run_id)
                                    .with_field("tool_name", json!(message.tool_name))
                                    .with_field("tool_use_id", json!(message.tool_use_id))
                                    .with_field(
                                        "message",
                                        json!(summarize_provider_message(&message)),
                                    ),
                            )
                            .await;
                        }
                        StreamEvent::ToolResult { message } => {
                            record_thread_log(
                                sink,
                                thread_log_id.as_deref(),
                                ThreadLogEvent::info("", "tool", "sub-agent tool result emitted")
                                    .with_run_id(run_id)
                                    .with_field("tool_name", json!(message.tool_name))
                                    .with_field("tool_use_id", json!(message.tool_use_id))
                                    .with_field("is_error", json!(message.is_error))
                                    .with_field(
                                        "message",
                                        json!(summarize_provider_message(&message)),
                                    ),
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
                    && let Some(callback) = external_callback.as_ref()
                {
                    callback(event);
                }
            }) as Arc<dyn Fn(StreamEvent) + Send + Sync>)
        };

        let mut graph_state = RunGraphState::new(
            run_id.clone(),
            thread_id.to_owned(),
            provider_key.clone(),
            options,
        );
        let result =
            execute_agent_run(provider.as_ref(), &mut graph_state, response_callback).await;

        let removed_persistence = {
            let mut persistence = self.inner.active_thread_persistence.lock().await;
            let should_remove = persistence
                .get(thread_id)
                .map(|handle| handle.run_id == run_id.as_str())
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
                    tracing::warn!(
                        run_id = %run_id,
                        error = %error,
                        "sub-agent partial thread persistence task failed"
                    );
                    None
                }
            }
        } else {
            None
        };
        {
            let mut persistence = self.inner.active_thread_persistence.lock().await;
            let should_remove = persistence
                .get(thread_id)
                .map(|handle| handle.run_id == run_id.as_str())
                .unwrap_or(false);
            if should_remove {
                persistence.remove(thread_id);
            }
        }

        let outcome = match &result {
            Ok(res) => {
                if res.success {
                    MultiProviderBridge::record_health_success(
                        &self.inner,
                        &provider_key,
                        graph_state.metrics.duration_ms() as f64,
                    )
                    .await;
                } else {
                    MultiProviderBridge::record_health_failure(
                        &self.inner,
                        &provider_key,
                        res.error.as_deref().unwrap_or("unknown error"),
                    )
                    .await;
                }

                let mut applied_thread_title: Option<String> = None;
                if let (Some(store), Some(history)) = (
                    &*self.inner.thread_store.read().await,
                    &*self.inner.thread_history.read().await,
                ) {
                    let user_images = graph_state.run_options.images.clone().unwrap_or_default();
                    let persisted_assistant_response = persistence_result
                        .as_ref()
                        .map(|value| value.assistant_response.as_str())
                        .filter(|value| !value.is_empty())
                        .unwrap_or(&res.response);
                    let persisted_session_messages = persistence_result
                        .as_ref()
                        .map(|value| value.session_messages.as_slice())
                        .filter(|value| !value.is_empty())
                        .unwrap_or(&res.session_messages);
                    let persisted_transcript_controls = persistence_result
                        .as_ref()
                        .map(|value| value.transcript_controls.as_slice())
                        .unwrap_or(&[]);
                    let sdk_session_id = resolve_sdk_session_id_for_persistence(
                        &graph_state.run_options.metadata,
                        res.sdk_session_id.as_deref(),
                    );
                    applied_thread_title = persist_provider_thread_title_if_missing(
                        store,
                        thread_id,
                        res.thread_title.as_deref(),
                    )
                    .await;
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
                            user_message: message,
                            user_timestamp: Some(&run_started_at),
                            user_images: &user_images,
                            assistant_response: persisted_assistant_response,
                            sdk_session_id: sdk_session_id.as_deref(),
                            provider_key: &provider_key,
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
                MultiProviderBridge::record_health_failure(
                    &self.inner,
                    &provider_key,
                    &error.to_string(),
                )
                .await;

                if let (Some(store), Some(history)) = (
                    &*self.inner.thread_store.read().await,
                    &*self.inner.thread_history.read().await,
                ) {
                    let user_images = graph_state.run_options.images.clone().unwrap_or_default();
                    let failed_assistant_response = persistence_result
                        .as_ref()
                        .map(|value| value.assistant_response.as_str())
                        .unwrap_or_default();
                    let failed_session_messages = persistence_result
                        .as_ref()
                        .map(|value| value.session_messages.as_slice())
                        .unwrap_or(&[]);
                    let persisted_transcript_controls = persistence_result
                        .as_ref()
                        .map(|value| value.transcript_controls.as_slice())
                        .unwrap_or(&[]);
                    let rate_limit = provider.take_rate_limit(thread_id).await;
                    let terminal_committed = save_failed_thread_messages_with_terminal_control(
                        store,
                        history,
                        PersistedRun {
                            thread_id,
                            user_message: message,
                            user_timestamp: Some(&run_started_at),
                            user_images: &user_images,
                            assistant_response: failed_assistant_response,
                            sdk_session_id: None,
                            provider_key: &provider_key,
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
            let pending_input = PendingUserInput {
                id: format!("queued_input:{}", uuid::Uuid::new_v4()),
                bridge_run_id: run_id.clone(),
                text: message.to_owned(),
                content: build_pending_input_content(message, &image_payloads, &staged_attachments),
                queued_at: chrono::Utc::now().to_rfc3339(),
                origin_id: client_intent_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
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
            if queue_streaming_input_with_retry(
                &self.inner,
                provider.clone(),
                thread_id,
                provider_input,
            )
            .await
            {
                return Some(QueuedStreamingInput {
                    pending_input_id: pending_input.id,
                    run_id,
                });
            }

            if let Some(tx) = persistence_tx {
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

        // Cancel the tokio task.
        let task_cancelled = {
            let mut tasks = self.inner.active_tasks.lock().await;
            if let Some(task) = tasks.remove(run_id) {
                task.abort();
                true
            } else {
                false
            }
        };

        // Also try provider-level abort.
        let provider = match provider_key {
            Some(provider_key) => self
                .inner
                .topology
                .read()
                .await
                .provider_pool
                .get(&provider_key)
                .cloned(),
            None => None,
        };
        let provider_aborted = if let Some(provider) = provider {
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
