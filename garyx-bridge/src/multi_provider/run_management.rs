use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use garyx_models::provider::{
    AgentRunRequest, FORK_FROM_PROVIDER_TYPE_METADATA_KEY, FORK_FROM_SDK_SESSION_ID_METADATA_KEY,
    FilePayload, ImagePayload, PromptAttachment, ProviderMessage, ProviderMessageRole,
    ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput,
    SDK_SESSION_FORK_METADATA_KEY, SDK_SESSION_ID_METADATA_KEY, StreamEvent,
    attachments_from_metadata, build_user_content_from_parts, stage_file_payloads_for_prompt,
    stage_image_payloads_for_prompt,
};
use garyx_models::thread_logs::{ThreadLogEvent, ThreadLogSink, resolve_thread_log_thread_id};
use garyx_models::{Principal, TaskEventKind, TaskStatus, ThreadTask, is_tool_related_message};
use garyx_router::{
    ThreadHistoryRepository, ThreadStore, mark_thread_task_in_review_if_in_progress,
    thread_metadata_from_value,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep};

use crate::provider_trait::{AgentLoopProvider, BridgeError};
use crate::run_graph::{RunGraphState, execute_agent_run};

use super::MultiProviderBridge;
use super::persistence::{
    PendingUserInput, PendingUserInputStatus, PersistedRun, RunControlRecord, StreamingRunSnapshot,
    TerminalRunControl, ThreadPersistenceCommand,
    save_failed_thread_messages_with_terminal_control, save_streaming_partial,
    save_thread_messages_with_terminal_control,
};
use super::state::ActiveThreadPersistence;
use crate::garyx_native_provider::SESSION_MESSAGES_METADATA_KEY;

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

fn summarize_text(value: &str, limit: usize) -> String {
    let sanitized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }
    let mut clipped = trimmed
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    clipped.push('…');
    clipped
}

fn normalize_provider_thread_title(value: &str) -> Option<String> {
    let title = summarize_text(value, 80);
    (!title.is_empty()).then_some(title)
}

fn api_route_placeholder_label(existing: &Value) -> Option<String> {
    let channel = existing.get("channel").and_then(Value::as_str)?.trim();
    let account_id = existing.get("account_id").and_then(Value::as_str)?.trim();
    let from_id = existing.get("from_id").and_then(Value::as_str)?.trim();
    if channel != "api" || account_id.is_empty() || from_id.is_empty() {
        return None;
    }
    Some(format!("{channel}/{account_id}/{from_id}"))
}

fn should_apply_provider_thread_title(existing: &Value) -> bool {
    if existing
        .get("thread_title_source")
        .and_then(Value::as_str)
        .map(str::trim)
        == Some(PROMPT_THREAD_TITLE_SOURCE)
    {
        return true;
    }

    let Some(label) = existing.get("label").and_then(Value::as_str) else {
        return true;
    };
    let trimmed = label.trim();
    trimmed.is_empty()
        || trimmed == LEGACY_DEFAULT_THREAD_LABEL
        || api_route_placeholder_label(existing).as_deref() == Some(trimmed)
}

async fn persist_provider_thread_title_if_missing(
    store: &Arc<dyn ThreadStore>,
    thread_id: &str,
    title: Option<&str>,
) -> Option<String> {
    let title = title.and_then(normalize_provider_thread_title)?;
    let mut value = store.get(thread_id).await?;
    if !should_apply_provider_thread_title(&value) {
        return None;
    }
    let obj = value.as_object_mut()?;
    obj.insert("label".to_owned(), Value::String(title.clone()));
    obj.insert(
        "provider_thread_title".to_owned(),
        Value::String(title.clone()),
    );
    obj.insert(
        "thread_title_source".to_owned(),
        Value::String(PROVIDER_THREAD_TITLE_SOURCE.to_owned()),
    );
    obj.insert(
        "updated_at".to_owned(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    store.set(thread_id, value).await;
    Some(title)
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

fn build_pending_input_content(
    message: &str,
    images: &[ImagePayload],
    attachments: &[garyx_models::provider::PromptAttachment],
) -> Value {
    build_user_content_from_parts(message, attachments, images)
}

fn is_persistent_control_stream_event(event: &StreamEvent) -> bool {
    matches!(event, StreamEvent::Boundary { .. } | StreamEvent::Done)
}

fn emit_committed_records(
    event_tx: &Option<tokio::sync::broadcast::Sender<String>>,
    thread_id: &str,
    run_id: Option<&str>,
    committed: Vec<(u64, Value)>,
) {
    let event_run_id = run_id.map(str::to_owned);
    for (seq, message) in committed {
        emit_gateway_event(
            event_tx,
            serde_json::json!({
                "type": "committed_message",
                "thread_id": thread_id,
                "run_id": event_run_id.clone(),
                "seq": seq,
                "message": message,
            }),
        );
    }
}

fn control_record_for_stream_event(
    thread_id: &str,
    run_id: &str,
    event: &StreamEvent,
    after_content_count: usize,
) -> Option<RunControlRecord> {
    let mut payload = serde_json::Map::new();
    let kind = match event {
        StreamEvent::Boundary {
            kind: garyx_models::provider::StreamBoundaryKind::AssistantSegment,
            pending_input_id,
        } => {
            if let Some(pending_input_id) = pending_input_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                payload.insert(
                    "pending_input_id".to_owned(),
                    Value::String(pending_input_id.to_owned()),
                );
            }
            "assistant_boundary"
        }
        StreamEvent::Boundary {
            kind: garyx_models::provider::StreamBoundaryKind::UserAck,
            pending_input_id,
        } => {
            if let Some(pending_input_id) = pending_input_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                payload.insert(
                    "pending_input_id".to_owned(),
                    Value::String(pending_input_id.to_owned()),
                );
            }
            "user_ack"
        }
        StreamEvent::Done => "done",
        _ => return None,
    };
    Some(RunControlRecord::new(
        kind,
        thread_id,
        run_id,
        Utc::now().to_rfc3339(),
        payload,
        after_content_count,
    ))
}

fn abort_terminal_control_record(
    thread_id: &str,
    run_id: &str,
    after_content_count: usize,
    error: Option<&str>,
) -> RunControlRecord {
    let mut payload = serde_json::Map::new();
    payload.insert("status".to_owned(), Value::String("interrupted".to_owned()));
    if let Some(error) = error.map(str::trim).filter(|value| !value.is_empty()) {
        payload.insert("error".to_owned(), Value::String(error.to_owned()));
    }
    RunControlRecord::new(
        "run_complete",
        thread_id,
        run_id,
        Utc::now().to_rfc3339(),
        payload,
        after_content_count,
    )
}

fn forward_applied_thread_title_update(
    external_callback: Option<&Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    applied_thread_title: Option<&str>,
) {
    if let Some(title) = applied_thread_title {
        if let Some(callback) = external_callback {
            callback(StreamEvent::ThreadTitleUpdated {
                title: title.to_owned(),
            });
        }
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

fn take_pending_input_for_ack(
    pending_user_inputs: &mut Vec<PendingUserInput>,
    pending_input_id: Option<&str>,
) -> Option<PendingUserInput> {
    let target_id = pending_input_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(target_id) = target_id
        && let Some(index) = pending_user_inputs
            .iter()
            .position(|input| input.id == target_id)
    {
        return Some(pending_user_inputs.remove(index));
    }

    if pending_user_inputs.is_empty() {
        None
    } else {
        Some(pending_user_inputs.remove(0))
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_partial_thread_persistence_worker(
    store: Arc<dyn ThreadStore>,
    history: Arc<ThreadHistoryRepository>,
    thread_id: String,
    user_message: String,
    user_timestamp: String,
    user_images: Vec<ImagePayload>,
    provider_key: String,
    provider_type: ProviderType,
    metadata: HashMap<String, Value>,
    gateway_event_tx: Option<tokio::sync::broadcast::Sender<String>>,
) -> (
    mpsc::UnboundedSender<ThreadPersistenceCommand>,
    JoinHandle<StreamingPersistenceWorkerResult>,
) {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ThreadPersistenceCommand>();
    let task = tokio::spawn(async move {
        let mut snapshot = StreamingRunSnapshot::default();
        let mut pending_user_inputs = Vec::<PendingUserInput>::new();
        // Running count of finalized rows already appended to the committed
        // transcript for this run (F1 real-time append cursor).
        let mut appended_finalized: usize = 0;
        // Run id stamped on the live `committed_message` events (S5). The
        // per-thread stream filters by thread_id; run_id is informational.
        let committed_event_run_id = metadata
            .get("bridge_run_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut transcript_controls = Vec::<RunControlRecord>::new();
        if let Some(run_id) = committed_event_run_id.as_deref() {
            let mut payload = serde_json::Map::new();
            if !provider_key.trim().is_empty() {
                payload.insert(
                    "provider_key".to_owned(),
                    Value::String(provider_key.clone()),
                );
            }
            payload.insert(
                "provider_type".to_owned(),
                serde_json::to_value(&provider_type).unwrap_or(Value::Null),
            );
            transcript_controls.push(RunControlRecord::new(
                "run_start",
                &thread_id,
                run_id,
                user_timestamp.clone(),
                payload,
                0,
            ));
        }
        // Publish each row this flush committed to the jsonl as a seq'd
        // `committed_message` on the gateway bus, AFTER the append flushed
        // (write-then-emit): the in-memory event never references a seq the file
        // does not yet have, so a reconnect replay can never miss it.
        let emit_committed = |committed: Vec<(u64, Value)>| {
            emit_committed_records(
                &gateway_event_tx,
                &thread_id,
                committed_event_run_id.as_deref(),
                committed,
            );
        };

        let (cursor, committed) = save_streaming_partial(
            &store,
            &history,
            PersistedRun {
                thread_id: &thread_id,
                user_message: &user_message,
                user_timestamp: Some(&user_timestamp),
                user_images: &user_images,
                assistant_response: "",
                sdk_session_id: None,
                provider_key: &provider_key,
                provider_type: provider_type.clone(),
                session_messages: &[],
                metadata: &metadata,
            },
            &pending_user_inputs,
            &transcript_controls,
            0,
            appended_finalized,
        )
        .await;
        appended_finalized = cursor;
        emit_committed(committed);

        let mut abort_terminal_ack: Option<tokio::sync::oneshot::Sender<()>> = None;
        while let Some(command) = event_rx.recv().await {
            let mut dirty = false;
            let mut finish = false;
            let mut after_commit_callbacks = Vec::new();
            match command {
                ThreadPersistenceCommand::Stream {
                    event,
                    after_commit,
                } => {
                    if let Some(callback) = after_commit {
                        after_commit_callbacks.push((callback, event.clone()));
                    }
                    match event {
                        StreamEvent::Boundary {
                            kind: garyx_models::provider::StreamBoundaryKind::UserAck,
                            ref pending_input_id,
                        } => {
                            snapshot.apply_stream_event(&event);
                            if let Some(pending_input) = take_pending_input_for_ack(
                                &mut pending_user_inputs,
                                pending_input_id.as_deref(),
                            ) {
                                dirty |= snapshot.acknowledge_pending_input(&pending_input);
                            }
                            if let Some(run_id) = committed_event_run_id.as_deref()
                                && let Some(control) = control_record_for_stream_event(
                                    &thread_id,
                                    run_id,
                                    &event,
                                    1 + snapshot.session_messages.len(),
                                )
                            {
                                transcript_controls.push(control);
                                dirty = true;
                            }
                        }
                        other => {
                            dirty |= snapshot.apply_stream_event(&other);
                            if let Some(run_id) = committed_event_run_id.as_deref()
                                && let Some(control) = control_record_for_stream_event(
                                    &thread_id,
                                    run_id,
                                    &other,
                                    1 + snapshot.session_messages.len(),
                                )
                            {
                                transcript_controls.push(control);
                                dirty = true;
                            }
                        }
                    }
                }
                ThreadPersistenceCommand::QueuePendingInput(pending_input) => {
                    pending_user_inputs.push(pending_input);
                    dirty = true;
                }
                ThreadPersistenceCommand::DropPendingInput { pending_input_id } => {
                    let before = pending_user_inputs.len();
                    pending_user_inputs.retain(|input| input.id != pending_input_id);
                    dirty = pending_user_inputs.len() != before;
                }
                ThreadPersistenceCommand::AbortTerminal { error, ack } => {
                    if let Some(run_id) = committed_event_run_id.as_deref() {
                        transcript_controls.push(abort_terminal_control_record(
                            &thread_id,
                            run_id,
                            1 + snapshot.session_messages.len(),
                            error.as_deref(),
                        ));
                        dirty = true;
                    }
                    abort_terminal_ack = Some(ack);
                    finish = true;
                }
                ThreadPersistenceCommand::Finish => {
                    finish = true;
                }
            }
            while let Ok(pending) = event_rx.try_recv() {
                match pending {
                    ThreadPersistenceCommand::Stream {
                        event,
                        after_commit,
                    } => {
                        if let Some(callback) = after_commit {
                            after_commit_callbacks.push((callback, event.clone()));
                        }
                        match event {
                            StreamEvent::Boundary {
                                kind: garyx_models::provider::StreamBoundaryKind::UserAck,
                                ref pending_input_id,
                            } => {
                                snapshot.apply_stream_event(&event);
                                if let Some(pending_input) = take_pending_input_for_ack(
                                    &mut pending_user_inputs,
                                    pending_input_id.as_deref(),
                                ) {
                                    dirty |= snapshot.acknowledge_pending_input(&pending_input);
                                }
                                if let Some(run_id) = committed_event_run_id.as_deref()
                                    && let Some(control) = control_record_for_stream_event(
                                        &thread_id,
                                        run_id,
                                        &event,
                                        1 + snapshot.session_messages.len(),
                                    )
                                {
                                    transcript_controls.push(control);
                                    dirty = true;
                                }
                            }
                            other => {
                                dirty |= snapshot.apply_stream_event(&other);
                                if let Some(run_id) = committed_event_run_id.as_deref()
                                    && let Some(control) = control_record_for_stream_event(
                                        &thread_id,
                                        run_id,
                                        &other,
                                        1 + snapshot.session_messages.len(),
                                    )
                                {
                                    transcript_controls.push(control);
                                    dirty = true;
                                }
                            }
                        }
                    }
                    ThreadPersistenceCommand::QueuePendingInput(pending_input) => {
                        pending_user_inputs.push(pending_input);
                        dirty = true;
                    }
                    ThreadPersistenceCommand::DropPendingInput { pending_input_id } => {
                        let before = pending_user_inputs.len();
                        pending_user_inputs.retain(|input| input.id != pending_input_id);
                        dirty |= pending_user_inputs.len() != before;
                    }
                    ThreadPersistenceCommand::AbortTerminal { error, ack } => {
                        if let Some(run_id) = committed_event_run_id.as_deref() {
                            transcript_controls.push(abort_terminal_control_record(
                                &thread_id,
                                run_id,
                                1 + snapshot.session_messages.len(),
                                error.as_deref(),
                            ));
                            dirty = true;
                        }
                        abort_terminal_ack = Some(ack);
                        finish = true;
                    }
                    ThreadPersistenceCommand::Finish => {
                        finish = true;
                    }
                }
            }
            if dirty {
                let (cursor, committed) = save_streaming_partial(
                    &store,
                    &history,
                    PersistedRun {
                        thread_id: &thread_id,
                        user_message: &user_message,
                        user_timestamp: Some(&user_timestamp),
                        user_images: &user_images,
                        assistant_response: &snapshot.assistant_response,
                        sdk_session_id: snapshot.sdk_session_id.as_deref(),
                        provider_key: &provider_key,
                        provider_type: provider_type.clone(),
                        session_messages: &snapshot.session_messages,
                        metadata: &metadata,
                    },
                    &pending_user_inputs,
                    &transcript_controls,
                    snapshot.finalized_len(),
                    appended_finalized,
                )
                .await;
                appended_finalized = cursor;
                emit_committed(committed);
            }
            for (callback, event) in after_commit_callbacks {
                callback(event);
            }
            if finish {
                break;
            }
        }

        for pending_input in &mut pending_user_inputs {
            if pending_input.status == PendingUserInputStatus::Queued {
                pending_input.status = PendingUserInputStatus::Abandoned;
            }
        }
        {
            // Final flush at run end. The whole session is finalized now (the run
            // ended, so the trailing assistant is no longer in-flight), so commit
            // the FULL length — this commits + emits the last segment that the
            // periodic streaming flush has not finalized yet, closing the crash window AND
            // delivering it as a seq'd `committed_message` so the client's cursor
            // advances continuously rather than waiting for the next reconnect.
            let (_, committed) = save_streaming_partial(
                &store,
                &history,
                PersistedRun {
                    thread_id: &thread_id,
                    user_message: &user_message,
                    user_timestamp: Some(&user_timestamp),
                    user_images: &user_images,
                    assistant_response: &snapshot.assistant_response,
                    sdk_session_id: snapshot.sdk_session_id.as_deref(),
                    provider_key: &provider_key,
                    provider_type: provider_type.clone(),
                    session_messages: &snapshot.session_messages,
                    metadata: &metadata,
                },
                &pending_user_inputs,
                &transcript_controls,
                snapshot.session_messages.len(),
                appended_finalized,
            )
            .await;
            emit_committed(committed);
        }
        if let Some(ack) = abort_terminal_ack {
            let _ = ack.send(());
        }

        StreamingPersistenceWorkerResult {
            assistant_response: snapshot.assistant_response,
            session_messages: snapshot.session_messages,
            transcript_controls,
        }
    });

    (event_tx, task)
}

fn resolve_sdk_session_id_for_persistence(
    metadata: &HashMap<String, Value>,
    result_sdk_session_id: Option<&str>,
) -> Option<String> {
    result_sdk_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            metadata
                .get(SDK_SESSION_ID_METADATA_KEY)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn non_empty_value_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn persisted_provider_type(session_data: &Value) -> Option<ProviderType> {
    let raw = session_data.get("provider_type")?.clone();
    serde_json::from_value(raw.clone())
        .map_err(
            |e| tracing::debug!(raw = %raw, error = %e, "failed to parse persisted provider_type"),
        )
        .ok()
}

fn provider_types_share_native_session(left: &ProviderType, right: &ProviderType) -> bool {
    left == right
}

fn resolve_persisted_sdk_session_id_for_provider(
    session_data: &Value,
    provider_key: &str,
    provider_type: Option<&ProviderType>,
) -> Option<String> {
    let object = session_data.as_object()?;

    if let Some(expected_provider_type) = provider_type
        && persisted_provider_type(session_data)
            .as_ref()
            .is_some_and(|persisted| {
                provider_types_share_native_session(persisted, expected_provider_type)
            })
        && let Some(sdk_session_id) = non_empty_value_string(object.get("sdk_session_id"))
    {
        return Some(sdk_session_id);
    }

    let trimmed_provider_key = provider_key.trim();
    if trimmed_provider_key.is_empty() {
        return None;
    }

    if let Some(provider_scoped_session_id) = object
        .get("provider_sdk_session_ids")
        .and_then(Value::as_object)
        .and_then(|map| map.get(trimmed_provider_key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(provider_scoped_session_id.to_owned());
    }

    let stored_provider_key = non_empty_value_string(object.get("provider_key"));
    if stored_provider_key.as_deref() != Some(trimmed_provider_key) {
        return None;
    }

    non_empty_value_string(object.get("sdk_session_id"))
}

fn provider_type_from_metadata_value(value: Option<&Value>) -> Option<ProviderType> {
    let raw = value?.clone();
    serde_json::from_value(raw.clone())
        .map_err(|e| tracing::debug!(raw = %raw, error = %e, "failed to parse fork provider_type"))
        .ok()
}

fn resolve_fork_sdk_session_id_for_provider(
    session_data: &Value,
    provider_type: &ProviderType,
) -> Option<String> {
    let metadata = session_data.get("metadata").and_then(Value::as_object)?;
    if metadata
        .get(SDK_SESSION_FORK_METADATA_KEY)
        .and_then(Value::as_bool)
        != Some(true)
    {
        return None;
    }
    let fork_provider_type =
        provider_type_from_metadata_value(metadata.get(FORK_FROM_PROVIDER_TYPE_METADATA_KEY))?;
    if !provider_types_share_native_session(&fork_provider_type, provider_type) {
        return None;
    }
    non_empty_value_string(metadata.get(FORK_FROM_SDK_SESSION_ID_METADATA_KEY))
}

fn attach_provider_sdk_session_metadata(
    options: &mut ProviderRunOptions,
    session_data: &Value,
    provider_key: &str,
    provider_type: &ProviderType,
) {
    // Thread metadata is copied into dispatch metadata before this point. Clear
    // fork mode first so a child thread that has already bound its own provider
    // session resumes normally instead of forking from the parent every turn.
    options.metadata.remove(SDK_SESSION_FORK_METADATA_KEY);

    if let Some(sid) = resolve_persisted_sdk_session_id_for_provider(
        session_data,
        provider_key,
        Some(provider_type),
    ) {
        options
            .metadata
            .insert(SDK_SESSION_ID_METADATA_KEY.to_owned(), Value::String(sid));
        return;
    }

    if let Some(parent_sid) = resolve_fork_sdk_session_id_for_provider(session_data, provider_type)
    {
        options.metadata.insert(
            SDK_SESSION_ID_METADATA_KEY.to_owned(),
            Value::String(parent_sid),
        );
        options
            .metadata
            .insert(SDK_SESSION_FORK_METADATA_KEY.to_owned(), Value::Bool(true));
    }
}

fn persisted_provider_messages_from_thread(session_data: &Value) -> Vec<ProviderMessage> {
    let committed = session_data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut messages = Vec::with_capacity(committed.len());
    for value in &committed {
        if let Some(message) = ProviderMessage::from_value(value) {
            messages.push(message);
        }
    }
    messages
}

fn attach_native_session_messages(
    options: &mut ProviderRunOptions,
    session_data: &Value,
    provider_type: &ProviderType,
) {
    if !matches!(
        provider_type,
        ProviderType::Gpt | ProviderType::ClaudeLlm | ProviderType::GeminiLlm
    ) {
        return;
    }
    let messages = persisted_provider_messages_from_thread(session_data);
    if messages.is_empty() {
        return;
    }
    options.metadata.insert(
        SESSION_MESSAGES_METADATA_KEY.to_owned(),
        serde_json::to_value(messages).unwrap_or(Value::Array(Vec::new())),
    );
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

/// Best assistant text for a task-ready notification body.
///
/// Provider `session_messages` usually carry only assistant/tool output; the
/// triggering user row is synthesized later when the run is persisted. When a
/// human-user boundary is present, collect the final assistant turn across
/// transparent tool traces. Without that boundary, preserve the historical
/// provider-session behavior: use the trailing assistant island after the last
/// tool call, so early narration is not pulled into the notification.
fn last_assistant_segment(messages: &[ProviderMessage]) -> Option<String> {
    if let Some(last_user_index) = messages.iter().rposition(is_human_user_provider_message) {
        return assistant_text_after_provider_user(&messages[last_user_index + 1..]);
    }
    trailing_assistant_text_island(messages)
}

fn assistant_text_after_provider_user(messages: &[ProviderMessage]) -> Option<String> {
    let mut current_group: Vec<String> = Vec::new();
    let mut last_group: Vec<String> = Vec::new();

    for message in messages {
        if is_human_user_provider_message(message) {
            current_group.clear();
            last_group.clear();
            continue;
        }
        if matches!(message.role, ProviderMessageRole::Assistant)
            && !is_tool_related_provider_message(message)
        {
            if let Some(text) = trimmed_provider_text(message) {
                current_group.push(text);
                last_group = current_group.clone();
            }
            continue;
        }
        if is_tool_related_provider_message(message) {
            continue;
        }
        current_group.clear();
    }

    (!last_group.is_empty()).then(|| last_group.join("\n\n"))
}

fn trailing_assistant_text_island(messages: &[ProviderMessage]) -> Option<String> {
    let mut segments: Vec<String> = Vec::new();

    for message in messages.iter().rev() {
        if matches!(message.role, ProviderMessageRole::Assistant)
            && !is_tool_related_provider_message(message)
        {
            if let Some(text) = trimmed_provider_text(message) {
                segments.push(text);
            }
            continue;
        }
        if segments.is_empty() && is_tool_related_provider_message(message) {
            continue;
        }
        if segments.is_empty()
            && matches!(message.role, ProviderMessageRole::Assistant)
            && trimmed_provider_text(message).is_none()
        {
            continue;
        }
        break;
    }

    if segments.is_empty() {
        return None;
    }
    segments.reverse();
    Some(segments.join("\n\n"))
}

fn trimmed_provider_text(message: &ProviderMessage) -> Option<String> {
    message
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn is_human_user_provider_message(message: &ProviderMessage) -> bool {
    matches!(message.role, ProviderMessageRole::User) && !is_tool_related_provider_message(message)
}

fn is_tool_related_provider_message(message: &ProviderMessage) -> bool {
    if matches!(
        message.role,
        ProviderMessageRole::ToolUse | ProviderMessageRole::ToolResult
    ) {
        return true;
    }
    message
        .to_json_value()
        .as_object()
        .is_some_and(|object| is_tool_related_message(message.role_str(), object))
}

fn task_latest_in_review_transition_after(task: &ThreadTask, run_started_at: &str) -> bool {
    if task.status != TaskStatus::InReview {
        return false;
    }
    let Ok(run_started_at) =
        DateTime::parse_from_rfc3339(run_started_at).map(|value| value.with_timezone(&Utc))
    else {
        return false;
    };
    task.events.last().is_some_and(|event| {
        event.at >= run_started_at
            && matches!(
                event.kind,
                TaskEventKind::StatusChanged {
                    from: TaskStatus::InProgress,
                    to: TaskStatus::InReview,
                    ..
                }
            )
    })
}

async fn task_already_ready_for_review_during_run(
    store: &Arc<dyn ThreadStore>,
    thread_id: &str,
    run_started_at: &str,
) -> Result<Option<ThreadTask>, garyx_router::TaskServiceError> {
    let Some(record) = store.get(thread_id).await else {
        return Ok(None);
    };
    let Some(task) = garyx_router::tasks::task_from_record(&record)? else {
        return Ok(None);
    };
    if task_latest_in_review_transition_after(&task, run_started_at) {
        Ok(Some(task))
    } else {
        Ok(None)
    }
}

async fn emit_task_ready_for_review_event(
    inner: &super::state::Inner,
    thread_id: &str,
    run_id: &str,
    task_id: &str,
    notification_text: Option<&str>,
) {
    if let Some(tx) = &*inner.event_tx.read().await {
        let event = serde_json::json!({
            "type": "task_ready_for_review",
            "thread_id": thread_id,
            "run_id": run_id,
            "task_id": task_id,
            "final_message": notification_text
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        });
        let _ = tx.send(event.to_string());
    }
}

async fn mark_task_ready_for_review_after_stopped_run(
    inner: &super::state::Inner,
    thread_id: &str,
    run_id: &str,
    run_started_at: &str,
    gate_response: Option<&str>,
    notification_text: Option<&str>,
    allow_transition: bool,
    thread_logs: Option<Arc<dyn ThreadLogSink>>,
    thread_log_id: Option<&str>,
) {
    let has_gate_response = gate_response
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    if allow_transition && !has_gate_response {
        record_thread_log(
            thread_logs.clone(),
            thread_log_id,
            ThreadLogEvent::info(
                "",
                "task",
                "task run stopped without final response; leaving task in progress",
            )
            .with_run_id(run_id.to_owned())
            .with_field("thread_id", json!(thread_id)),
        )
        .await;
    }
    let Some(store) = inner.thread_store.read().await.clone() else {
        return;
    };

    if allow_transition && has_gate_response {
        match mark_thread_task_in_review_if_in_progress(
            &store,
            thread_id,
            Principal::Agent {
                agent_id: "garyx".to_owned(),
            },
            Some("agent run stopped".to_owned()),
        )
        .await
        {
            Ok(Some(task)) => {
                let task_id = garyx_router::tasks::canonical_task_id(&task);
                emit_task_ready_for_review_event(
                    inner,
                    thread_id,
                    run_id,
                    &task_id,
                    notification_text,
                )
                .await;
                record_thread_log(
                    thread_logs.clone(),
                    thread_log_id,
                    ThreadLogEvent::info("", "task", "task moved to review after run stopped")
                        .with_run_id(run_id.to_owned())
                        .with_field("task_id", json!(task_id)),
                )
                .await;
                return;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    thread_id = %thread_id,
                    run_id = %run_id,
                    error = %error,
                    "failed to move stopped task to review"
                );
                record_thread_log(
                    thread_logs.clone(),
                    thread_log_id,
                    ThreadLogEvent::warn("", "task", "failed to move stopped task to review")
                        .with_run_id(run_id.to_owned())
                        .with_field("error", json!(error.to_string())),
                )
                .await;
                return;
            }
        }
    }

    match task_already_ready_for_review_during_run(&store, thread_id, run_started_at).await {
        Ok(Some(task)) => {
            let task_id = garyx_router::tasks::canonical_task_id(&task);
            emit_task_ready_for_review_event(inner, thread_id, run_id, &task_id, notification_text)
                .await;
            record_thread_log(
                thread_logs.clone(),
                thread_log_id,
                ThreadLogEvent::info(
                    "",
                    "task",
                    "task ready notification emitted after run stopped",
                )
                .with_run_id(run_id.to_owned())
                .with_field("task_id", json!(task_id)),
            )
            .await;
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(
                thread_id = %thread_id,
                run_id = %run_id,
                error = %error,
                "failed to inspect stopped task review transition"
            );
            record_thread_log(
                thread_logs.clone(),
                thread_log_id,
                ThreadLogEvent::warn(
                    "",
                    "task",
                    "failed to inspect stopped task review transition",
                )
                .with_run_id(run_id.to_owned())
                .with_field("error", json!(error.to_string())),
            )
            .await;
        }
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
            attach_native_session_messages(&mut options, &session_data, &resolved_provider_type);
            attach_provider_sdk_session_metadata(
                &mut options,
                &session_data,
                &provider_key_owned,
                &resolved_provider_type,
            );
        }

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
                    let _ = partial_persistence_tx.as_ref().map(|tx| {
                        tx.send(ThreadPersistenceCommand::Stream {
                            event: event.clone(),
                            after_commit: None,
                        })
                    });
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
            let _ = partial_persistence_tx
                .as_ref()
                .map(|tx| tx.send(ThreadPersistenceCommand::Finish));
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

                    let last_segment = last_assistant_segment(
                        persistence_result
                            .as_ref()
                            .map(|value| value.session_messages.as_slice())
                            .filter(|messages| !messages.is_empty())
                            .unwrap_or(&res.session_messages),
                    );
                    mark_task_ready_for_review_after_stopped_run(
                        &inner,
                        &thread_id_owned,
                        &run_id_owned,
                        &run_started_at,
                        Some(&res.response),
                        last_segment.as_deref(),
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
                        &run_started_at,
                        None,
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
            attach_native_session_messages(&mut options, &session_data, &resolved_provider_type);
            attach_provider_sdk_session_metadata(
                &mut options,
                &session_data,
                &provider_key,
                &resolved_provider_type,
            );
        }

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
                    let _ = partial_persistence_tx.as_ref().map(|tx| {
                        tx.send(ThreadPersistenceCommand::Stream {
                            event: event.clone(),
                            after_commit: None,
                        })
                    });
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
        let _ = partial_persistence_tx
            .as_ref()
            .map(|tx| tx.send(ThreadPersistenceCommand::Finish));
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
