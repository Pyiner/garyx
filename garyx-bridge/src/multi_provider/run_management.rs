use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use garyx_models::Principal;
use garyx_models::provider::{
    AgentRunRequest, FilePayload, ImagePayload, PromptAttachment, PromptAttachmentKind,
    ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput, StreamEvent,
    attachments_from_metadata, build_user_content_from_parts, stage_file_payloads_for_prompt,
    stage_image_payloads_for_prompt,
};
use garyx_models::thread_logs::{ThreadLogEvent, ThreadLogSink, resolve_thread_log_thread_id};
use garyx_router::{
    ThreadHistoryRepository, ThreadStore, build_runtime_context_metadata, loop_enabled_from_value,
    loop_iteration_count_from_value, mark_thread_task_in_review_if_in_progress,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep};

use crate::gary_prompt::append_task_suffix_to_user_message;
use crate::provider_trait::{AgentLoopProvider, BridgeError};
use crate::run_graph::{RunGraphState, execute_agent_run};

use super::MultiProviderBridge;
use super::persistence::{
    PendingUserInput, PendingUserInputStatus, PersistedRun, StreamingRunSnapshot,
    ThreadPersistenceCommand, save_failed_thread_messages, save_partial_thread_messages,
    save_thread_messages,
};
use super::state::ActiveThreadPersistence;

const STREAMING_INPUT_QUEUE_RETRY_INTERVAL: Duration = Duration::from_millis(50);
const STREAMING_INPUT_QUEUE_RETRY_TIMEOUT: Duration = Duration::from_secs(15);
const FOLLOW_UP_INTERRUPT_WAIT_INTERVAL: Duration = Duration::from_millis(25);
const FOLLOW_UP_INTERRUPT_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

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

fn provider_type_label(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeCode => "claude_code",
        ProviderType::CodexAppServer => "codex_app_server",
        ProviderType::GeminiCli => "gemini_cli",
        ProviderType::AgentTeam => "agent_team",
    }
}

fn requested_provider_from_metadata(metadata: &HashMap<String, Value>) -> Option<ProviderType> {
    metadata
        .get("requested_provider_type")
        .and_then(Value::as_str)
        .and_then(|value| match value {
            "claude_code" => Some(ProviderType::ClaudeCode),
            "codex_app_server" => Some(ProviderType::CodexAppServer),
            "gemini_cli" => Some(ProviderType::GeminiCli),
            "agent_team" => Some(ProviderType::AgentTeam),
            _ => None,
        })
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
        "system_prompt",
        "requested_provider_type",
        "sdk_session_id",
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

fn build_user_message_event(
    thread_id: &str,
    run_id: &str,
    message: &str,
    images: Option<&[ImagePayload]>,
    attachments: Option<&[PromptAttachment]>,
) -> Value {
    let inline_image_count = images.map_or(0, <[ImagePayload]>::len);
    let attachment_images = attachments.map_or(0, |items| {
        items
            .iter()
            .filter(|item| item.kind == PromptAttachmentKind::Image)
            .count()
    });
    let attachment_files = attachments.map_or(0, |items| {
        items
            .iter()
            .filter(|item| item.kind == PromptAttachmentKind::File)
            .count()
    });
    let image_count = inline_image_count + attachment_images;
    let mut parts = Vec::new();
    if image_count > 0 {
        parts.push(format!(
            "{image_count} image{}",
            if image_count == 1 { "" } else { "s" }
        ));
    }
    if attachment_files > 0 {
        parts.push(format!(
            "{attachment_files} file{}",
            if attachment_files == 1 { "" } else { "s" }
        ));
    }
    let text = if message.trim().is_empty() && !parts.is_empty() {
        format!("[{}]", parts.join(", "))
    } else {
        message.to_owned()
    };
    json!({
        "type": "user_message",
        "thread_id": thread_id,
        "run_id": run_id,
        "text": text,
        "image_count": image_count,
    })
}

fn build_pending_input_content(
    message: &str,
    images: &[ImagePayload],
    attachments: &[garyx_models::provider::PromptAttachment],
) -> Value {
    build_user_content_from_parts(message, attachments, images)
}

fn build_stream_event_payload(thread_id: &str, run_id: &str, event: &StreamEvent) -> Option<Value> {
    match event {
        StreamEvent::Delta { text } => {
            if text.is_empty() {
                None
            } else {
                Some(json!({
                    "type": "assistant_delta",
                    "thread_id": thread_id,
                    "run_id": run_id,
                    "delta": text,
                }))
            }
        }
        StreamEvent::ToolUse { message } => Some(json!({
            "type": "tool_use",
            "thread_id": thread_id,
            "run_id": run_id,
            "message": message,
        })),
        StreamEvent::ToolResult { message } => Some(json!({
            "type": "tool_result",
            "thread_id": thread_id,
            "run_id": run_id,
            "message": message,
        })),
        StreamEvent::Boundary {
            kind,
            pending_input_id,
        } => Some(json!({
            "type": match kind {
                garyx_models::provider::StreamBoundaryKind::AssistantSegment => "assistant_boundary",
                garyx_models::provider::StreamBoundaryKind::UserAck => "user_ack",
            },
            "thread_id": thread_id,
            "run_id": run_id,
            "pending_input_id": pending_input_id,
        })),
        StreamEvent::Done => Some(json!({
            "type": "done",
            "thread_id": thread_id,
            "run_id": run_id,
        })),
    }
}

struct StreamingPersistenceWorkerResult {
    assistant_response: String,
    session_messages: Vec<garyx_models::provider::ProviderMessage>,
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
    if let Some(target_id) = target_id {
        if let Some(index) = pending_user_inputs
            .iter()
            .position(|input| input.id == target_id)
        {
            return Some(pending_user_inputs.remove(index));
        }
    }

    if pending_user_inputs.is_empty() {
        None
    } else {
        Some(pending_user_inputs.remove(0))
    }
}

fn spawn_partial_thread_persistence_worker(
    store: Arc<dyn ThreadStore>,
    history: Arc<ThreadHistoryRepository>,
    thread_id: String,
    user_message: String,
    user_images: Vec<ImagePayload>,
    provider_key: String,
    provider_type: ProviderType,
    metadata: HashMap<String, Value>,
) -> (
    mpsc::UnboundedSender<ThreadPersistenceCommand>,
    JoinHandle<StreamingPersistenceWorkerResult>,
) {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ThreadPersistenceCommand>();
    let task = tokio::spawn(async move {
        let mut snapshot = StreamingRunSnapshot::default();
        let mut pending_user_inputs = Vec::<PendingUserInput>::new();

        save_partial_thread_messages(
            &store,
            &history,
            PersistedRun {
                thread_id: &thread_id,
                user_message: &user_message,
                user_images: &user_images,
                assistant_response: "",
                sdk_session_id: None,
                provider_key: &provider_key,
                provider_type: provider_type.clone(),
                session_messages: &[],
                metadata: &metadata,
            },
            &pending_user_inputs,
        )
        .await;

        while let Some(command) = event_rx.recv().await {
            let mut dirty = false;
            match command {
                ThreadPersistenceCommand::Stream(event) => match event {
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
                    }
                    other => {
                        dirty |= snapshot.apply_stream_event(&other);
                    }
                },
                ThreadPersistenceCommand::QueuePendingInput(pending_input) => {
                    pending_user_inputs.push(pending_input);
                    dirty = true;
                }
                ThreadPersistenceCommand::DropPendingInput { pending_input_id } => {
                    let before = pending_user_inputs.len();
                    pending_user_inputs.retain(|input| input.id != pending_input_id);
                    dirty = pending_user_inputs.len() != before;
                }
            }
            while let Ok(pending) = event_rx.try_recv() {
                match pending {
                    ThreadPersistenceCommand::Stream(event) => match event {
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
                        }
                        other => {
                            dirty |= snapshot.apply_stream_event(&other);
                        }
                    },
                    ThreadPersistenceCommand::QueuePendingInput(pending_input) => {
                        pending_user_inputs.push(pending_input);
                        dirty = true;
                    }
                    ThreadPersistenceCommand::DropPendingInput { pending_input_id } => {
                        let before = pending_user_inputs.len();
                        pending_user_inputs.retain(|input| input.id != pending_input_id);
                        dirty |= pending_user_inputs.len() != before;
                    }
                }
            }
            if !dirty {
                continue;
            }

            save_partial_thread_messages(
                &store,
                &history,
                PersistedRun {
                    thread_id: &thread_id,
                    user_message: &user_message,
                    user_images: &user_images,
                    assistant_response: &snapshot.assistant_response,
                    sdk_session_id: None,
                    provider_key: &provider_key,
                    provider_type: provider_type.clone(),
                    session_messages: &snapshot.session_messages,
                    metadata: &metadata,
                },
                &pending_user_inputs,
            )
            .await;
        }

        let mut final_dirty = false;
        for pending_input in &mut pending_user_inputs {
            if pending_input.status == PendingUserInputStatus::Queued {
                pending_input.status = PendingUserInputStatus::Abandoned;
                final_dirty = true;
            }
        }
        if final_dirty {
            save_partial_thread_messages(
                &store,
                &history,
                PersistedRun {
                    thread_id: &thread_id,
                    user_message: &user_message,
                    user_images: &user_images,
                    assistant_response: &snapshot.assistant_response,
                    sdk_session_id: None,
                    provider_key: &provider_key,
                    provider_type: provider_type.clone(),
                    session_messages: &snapshot.session_messages,
                    metadata: &metadata,
                },
                &pending_user_inputs,
            )
            .await;
        }

        StreamingPersistenceWorkerResult {
            assistant_response: snapshot.assistant_response,
            session_messages: snapshot.session_messages,
        }
    });

    (event_tx, task)
}

fn has_tool_activity(messages: &[garyx_models::provider::ProviderMessage]) -> bool {
    messages
        .iter()
        .any(|message| matches!(message.role_str(), "tool_use" | "tool_result"))
}

fn should_auto_disable_loop(
    _metadata: &HashMap<String, Value>,
    result: &garyx_models::provider::ProviderRunResult,
) -> bool {
    result.success
        && !has_tool_activity(&result.session_messages)
        && !result.response.trim().is_empty()
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
                .get("sdk_session_id")
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

fn message_actor_label(object: &serde_json::Map<String, Value>) -> Option<String> {
    let metadata = object.get("metadata").and_then(Value::as_object);
    let role = object
        .get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();

    let agent_display_name = metadata
        .and_then(|fields| fields.get("agent_display_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let agent_id = metadata
        .and_then(|fields| fields.get("agent_id"))
        .and_then(Value::as_str)
        .or_else(|| object.get("agent_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let from_id = metadata
        .and_then(|fields| fields.get("from_id"))
        .and_then(Value::as_str)
        .or_else(|| object.get("from_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let internal_dispatch = metadata
        .and_then(|fields| fields.get("internal_dispatch"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    match role {
        "assistant" => agent_id.or(agent_display_name),
        "user" if internal_dispatch => agent_id.or(agent_display_name).or(from_id),
        "user" => Some("user".to_owned()),
        _ => agent_id.or(agent_display_name).or(from_id),
    }
}

/// Project a group thread's persisted `messages[]` into the JSON shape
/// `AgentTeamProvider::parse_group_transcript` consumes:
/// `[ { "agent_id": String, "text": String, "at": String }, ... ]`.
///
/// We include only entries that carry at least an `agent_id` or non-empty
/// `text` — matching the filter inside the provider — so a malformed or
/// empty message doesn't produce an empty `<group_activity>` envelope when
/// the provider later slices the snapshot for per-child catch-up.
///
/// The `text` field is resolved with the following precedence:
/// 1. The message's explicit `text` field (how the persistence layer
///    records assistant replies and user turns).
/// 2. `content` when it is a bare string (legacy persisted shape).
/// 3. Empty string otherwise (structured content such as image blocks or
///    tool_use payloads — these don't belong in the textual transcript
///    anyway).
///
/// `agent_id` is read from `metadata.agent_id` (the provider-side
/// attribution used by attach_run_fields and the team planner) with a
/// fallback to a top-level `agent_id` field in case a future persistence
/// tweak hoists it. `at` is taken from the message's `timestamp`.
///
/// We deliberately snapshot *all* messages, not just assistant replies:
/// user turns in a team thread carry routing-relevant context (e.g. prior
/// @mentions) that child agents catch up on through the envelope.
fn build_group_transcript_snapshot(thread_data: &Value) -> Value {
    let Some(messages) = thread_data.get("messages").and_then(Value::as_array) else {
        return Value::Array(Vec::new());
    };
    let mut entries = Vec::with_capacity(messages.len());
    for message in messages {
        let Some(object) = message.as_object() else {
            continue;
        };
        let agent_id = message_actor_label(object).unwrap_or_default();
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| object.get("content").and_then(Value::as_str))
            .unwrap_or("");
        if agent_id.is_empty() && text.is_empty() {
            continue;
        }
        let at = object
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("");
        entries.push(json!({
            "agent_id": agent_id,
            "text": text,
            "at": at,
        }));
    }
    Value::Array(entries)
}

fn persisted_provider_type(session_data: &Value) -> Option<ProviderType> {
    let raw = session_data.get("provider_type")?.clone();
    serde_json::from_value(raw.clone())
        .map_err(
            |e| tracing::debug!(raw = %raw, error = %e, "failed to parse persisted provider_type"),
        )
        .ok()
}

fn resolve_persisted_sdk_session_id_for_provider(
    session_data: &Value,
    provider_key: &str,
    provider_type: Option<&ProviderType>,
) -> Option<String> {
    let object = session_data.as_object()?;

    if let Some(expected_provider_type) = provider_type {
        if persisted_provider_type(session_data).as_ref() == Some(expected_provider_type) {
            if let Some(sdk_session_id) = non_empty_value_string(object.get("sdk_session_id")) {
                return Some(sdk_session_id);
            }
        }
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
    inner: &super::state::Inner,
    thread_id: &str,
    message: &str,
) -> String {
    let Some(store) = inner.thread_store.read().await.clone() else {
        return message.to_owned();
    };
    let Some(record) = store.get(thread_id).await else {
        return message.to_owned();
    };
    if record.get("task").is_none() {
        return message.to_owned();
    }

    let channel = record
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let account_id = record
        .get("account_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let from_id = record
        .get("from_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let workspace_dir = record.get("workspace_dir").and_then(Value::as_str);
    let runtime_context = build_runtime_context_metadata(
        thread_id,
        Some(&record),
        &HashMap::new(),
        channel,
        account_id,
        from_id,
        workspace_dir,
    );
    let metadata = HashMap::from([("runtime_context".to_owned(), runtime_context)]);
    append_task_suffix_to_user_message(message, &metadata)
}

async fn mark_task_ready_for_review_after_stopped_run(
    inner: &super::state::Inner,
    thread_id: &str,
    run_id: &str,
    final_message: Option<&str>,
    thread_logs: Option<Arc<dyn ThreadLogSink>>,
    thread_log_id: Option<&str>,
) {
    let Some(store) = inner.thread_store.read().await.clone() else {
        return;
    };
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
            if let Some(tx) = &*inner.event_tx.read().await {
                let event = serde_json::json!({
                    "type": "task_ready_for_review",
                    "thread_id": thread_id,
                    "run_id": run_id,
                    "task_id": task_id,
                    "final_message": final_message.unwrap_or_default(),
                });
                let _ = tx.send(event.to_string());
            }
            record_thread_log(
                thread_logs,
                thread_log_id,
                ThreadLogEvent::info("", "task", "task moved to review after run stopped")
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
                "failed to move stopped task to review"
            );
            record_thread_log(
                thread_logs,
                thread_log_id,
                ThreadLogEvent::warn("", "task", "failed to move stopped task to review")
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
    {
        if bridge.get_provider(&provider_key).await.is_some() {
            bridge
                .inner
                .thread_affinity
                .write()
                .await
                .insert(thread_id.to_owned(), provider_key.clone());
            return Some(provider_key);
        }
    }
    if let Some(provider_type) = persisted_provider_type(&session_data) {
        if let Some(provider_key) = bridge.select_best_provider(Some(provider_type), true).await {
            bridge
                .inner
                .thread_affinity
                .write()
                .await
                .insert(thread_id.to_owned(), provider_key.clone());
            return Some(provider_key);
        }
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
    async fn thread_agent_context(
        &self,
        thread_id: &str,
    ) -> (Option<String>, Option<ProviderType>, Option<Value>) {
        let Some(store) = self.inner.thread_store.read().await.clone() else {
            return (None, None, None);
        };
        let Some(thread_data) = store.get(thread_id).await else {
            return (None, None, None);
        };
        let agent_id = thread_data
            .get("agent_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let provider_type = thread_data
            .get("provider_type")
            .cloned()
            .and_then(|value| serde_json::from_value::<ProviderType>(value).ok());
        (agent_id, provider_type, Some(thread_data))
    }

    async fn enrich_agent_metadata(
        &self,
        thread_id: &str,
        metadata: &mut HashMap<String, Value>,
    ) -> Option<ProviderType> {
        let (thread_agent_id, thread_provider_type, thread_data) =
            self.thread_agent_context(thread_id).await;
        let mut agent_id = metadata
            .get("agent_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        if agent_id.is_none() {
            agent_id = thread_agent_id;
        }

        let Some(agent_id) = agent_id else {
            return requested_provider_from_metadata(metadata).or(thread_provider_type);
        };

        metadata
            .entry("agent_id".to_owned())
            .or_insert_with(|| Value::String(agent_id.clone()));

        // Team-bound thread short-circuit. If the thread's `agent_id` refers
        // to an `AgentTeamProfile`, route through the AgentTeamProvider by
        // forcing `requested_provider_type = "agent_team"` and injecting the
        // two metadata fields `AgentTeamProvider` consumes:
        //
        //   - `agent_team_id`: identifies which team profile to load.
        //   - `group_transcript_snapshot`: projection of the group thread's
        //     existing `messages[]` into `[{agent_id, text, at}]`, used by
        //     the provider for per-child catch-up slicing. The current user
        //     turn is *not* yet persisted in `messages[]` at this call site
        //     (the partial persistence worker only starts writing after
        //     enrich_agent_metadata returns), so the snapshot is correctly
        //     "transcript without live turn" as the provider expects.
        //
        // The AgentTeam provider expects a group transcript snapshot that
        // excludes the live turn currently being enriched.
        if let Some(team) = self.team_profile(&agent_id).await {
            metadata.insert(
                "agent_team_id".to_owned(),
                Value::String(team.team_id.clone()),
            );
            metadata
                .entry("agent_display_name".to_owned())
                .or_insert_with(|| Value::String(team.display_name.clone()));

            let snapshot = thread_data
                .as_ref()
                .map(build_group_transcript_snapshot)
                .unwrap_or_else(|| Value::Array(Vec::new()));
            metadata.insert("group_transcript_snapshot".to_owned(), snapshot);

            // Force AgentTeamProvider selection regardless of any previously
            // requested provider: child providers are dispatched *through*
            // the team provider, never directly, when the thread is bound
            // to a team.
            metadata.insert(
                "requested_provider_type".to_owned(),
                Value::String(provider_type_label(&ProviderType::AgentTeam).to_owned()),
            );
            return Some(ProviderType::AgentTeam);
        }

        let Some(profile) = self.agent_profile(&agent_id).await else {
            return requested_provider_from_metadata(metadata).or(thread_provider_type);
        };

        metadata
            .entry("agent_display_name".to_owned())
            .or_insert_with(|| Value::String(profile.display_name.clone()));
        if !metadata.contains_key("model") && !profile.model.trim().is_empty() {
            metadata.insert("model".to_owned(), Value::String(profile.model.clone()));
        }
        if !metadata.contains_key("system_prompt") && !profile.system_prompt.trim().is_empty() {
            metadata.insert(
                "system_prompt".to_owned(),
                Value::String(profile.system_prompt.clone()),
            );
        }
        if !metadata.contains_key("requested_provider_type") {
            let preferred_provider_type = thread_provider_type
                .clone()
                .unwrap_or_else(|| profile.provider_type.clone());
            metadata.insert(
                "requested_provider_type".to_owned(),
                Value::String(provider_type_label(&preferred_provider_type).to_owned()),
            );
        }
        requested_provider_from_metadata(metadata)
            .or(thread_provider_type)
            .or(Some(profile.provider_type.clone()))
    }

    async fn resolve_thread_execution_target(
        &self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        metadata: &mut HashMap<String, Value>,
        requested_provider: Option<ProviderType>,
    ) -> Result<(String, Arc<dyn AgentLoopProvider>, Option<ProviderType>), BridgeError> {
        let _ = restore_thread_affinity_from_store(self, thread_id).await;
        let inferred_requested_provider = self.enrich_agent_metadata(thread_id, metadata).await;
        let requested_provider = requested_provider
            .or(inferred_requested_provider)
            .or_else(|| requested_provider_from_metadata(metadata));

        if requested_provider == Some(ProviderType::AgentTeam)
            && !metadata.contains_key("agent_team_id")
        {
            return Err(missing_agent_team_binding_error(thread_id, metadata));
        }

        let provider_key = self
            .resolve_provider_for_request(
                thread_id,
                channel,
                account_id,
                requested_provider.clone(),
            )
            .await
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
        if self
            .add_streaming_input(&thread_id, &message, images.clone(), None, None)
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
            let prompt_attachments = attachments_from_metadata(&metadata);
            emit_gateway_event(
                &gateway_event_tx,
                build_user_message_event(
                    &thread_id,
                    &run_id,
                    &message,
                    images.as_deref(),
                    Some(&prompt_attachments),
                ),
            );
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
        let prompt_attachments = attachments_from_metadata(&metadata);
        emit_gateway_event(
            &gateway_event_tx,
            build_user_message_event(
                &thread_id,
                &run_id,
                &message,
                images.as_deref(),
                Some(&prompt_attachments),
            ),
        );

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
        if let Some(store) = &*self.inner.thread_store.read().await {
            if let Some(session_data) = store.get(&thread_id).await {
                let resolved_provider_type = provider.provider_type();
                if let Some(sid) = resolve_persisted_sdk_session_id_for_provider(
                    &session_data,
                    &provider_key_owned,
                    Some(&resolved_provider_type),
                ) {
                    options
                        .metadata
                        .insert("sdk_session_id".to_owned(), Value::String(sid.to_owned()));
                }
            }
        }

        let partial_user_images = options.images.clone().unwrap_or_default();
        let partial_metadata = options.metadata.clone();
        let partial_provider_key = provider_key_owned.clone();
        let partial_provider_type = provider.provider_type();
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
                    partial_user_images,
                    partial_provider_key,
                    partial_provider_type,
                    partial_metadata,
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
        let response_callback = {
            let external_callback = response_callback.clone();
            let sink = thread_logs.clone();
            let thread_log_id = thread_log_id.clone();
            let run_id = run_id.to_owned();
            let thread_id = thread_id.to_owned();
            let first_token_logged = Arc::new(AtomicBool::new(false));
            let gateway_event_tx = gateway_event_tx.clone();
            let partial_persistence_tx = partial_persistence_tx.clone();
            Some(Arc::new(move |event: StreamEvent| {
                let sink = sink.clone();
                let thread_log_id = thread_log_id.clone();
                let run_id = run_id.clone();
                let event_run_id = run_id.clone();
                let thread_id = thread_id.clone();
                let external_callback = external_callback.clone();
                let first_token_logged = first_token_logged.clone();
                let event_for_log = event.clone();
                let event_for_emit = event.clone();
                let _ = partial_persistence_tx
                    .as_ref()
                    .map(|tx| tx.send(ThreadPersistenceCommand::Stream(event.clone())));
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
                        StreamEvent::Boundary { .. } | StreamEvent::Done => {}
                    }
                });
                if let Some(payload) =
                    build_stream_event_payload(&thread_id, &event_run_id, &event_for_emit)
                {
                    emit_gateway_event(&gateway_event_tx, payload);
                }
                if let Some(callback) = external_callback {
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

            // Emit run_start event.
            if let Some(tx) = &*inner.event_tx.read().await {
                let event = serde_json::json!({
                    "type": "run_start",
                    "run_id": run_id_owned,
                    "thread_id": thread_id_owned,
                });
                let _ = tx.send(event.to_string());
            }

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
                        let sdk_session_id = resolve_sdk_session_id_for_persistence(
                            &graph_state.run_options.metadata,
                            res.sdk_session_id.as_deref(),
                        );
                        save_thread_messages(
                            store,
                            history,
                            PersistedRun {
                                thread_id: &thread_id_owned,
                                user_message: &user_message,
                                user_images: &user_images,
                                assistant_response: persisted_assistant_response,
                                sdk_session_id: sdk_session_id.as_deref(),
                                provider_key: &provider_key_owned,
                                provider_type: provider.provider_type(),
                                session_messages: persisted_session_messages,
                                metadata: &graph_state.run_options.metadata,
                            },
                        )
                        .await;
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

                    let mut scheduled_loop_continue = false;

                    // Emit run_complete event.
                    if let Some(tx) = &*inner.event_tx.read().await {
                        let event = serde_json::json!({
                            "type": "run_complete",
                            "run_id": run_id_owned,
                            "thread_id": thread_id_owned,
                            "duration_ms": graph_state.metrics.duration_ms(),
                        });
                        let _ = tx.send(event.to_string());
                    }

                    // Auto-continue loop: if loop_enabled, schedule a
                    // continuation run after a short delay.
                    if res.success {
                        if let Some(store) = &*inner.thread_store.read().await {
                            if let Some(mut session_data) = store.get(&thread_id_owned).await {
                                let loop_enabled = loop_enabled_from_value(&session_data);
                                let iteration_count =
                                    loop_iteration_count_from_value(&session_data);

                                if loop_enabled {
                                    const MAX_LOOP_ITERATIONS: u64 = 50;
                                    if should_auto_disable_loop(
                                        &graph_state.run_options.metadata,
                                        res,
                                    ) {
                                        tracing::warn!(
                                            thread_id = %thread_id_owned,
                                            iteration_count,
                                            "latest run completed without tool activity; disabling loop"
                                        );
                                        if let Some(obj) = session_data.as_object_mut() {
                                            obj.insert(
                                                "loop_enabled".to_owned(),
                                                Value::Bool(false),
                                            );
                                            obj.insert("loop_iteration_count".to_owned(), json!(0));
                                            store.set(&thread_id_owned, session_data).await;
                                        }
                                        record_thread_log(
                                            thread_logs_for_task.clone(),
                                            thread_log_id_owned.as_deref(),
                                            ThreadLogEvent::warn(
                                                "",
                                                "loop",
                                                "loop auto-disabled after tool-free completion",
                                            )
                                            .with_run_id(run_id_owned.clone())
                                            .with_field("iteration", json!(iteration_count)),
                                        )
                                        .await;
                                    } else if iteration_count >= MAX_LOOP_ITERATIONS {
                                        tracing::warn!(
                                            thread_id = %thread_id_owned,
                                            iteration_count,
                                            "loop iteration limit reached, disabling loop"
                                        );
                                        if let Some(obj) = session_data.as_object_mut() {
                                            obj.insert(
                                                "loop_enabled".to_owned(),
                                                Value::Bool(false),
                                            );
                                            obj.insert("loop_iteration_count".to_owned(), json!(0));
                                            store.set(&thread_id_owned, session_data).await;
                                        }
                                    } else {
                                        // Increment iteration count.
                                        if let Some(obj) = session_data.as_object_mut() {
                                            obj.insert(
                                                "loop_iteration_count".to_owned(),
                                                json!(iteration_count + 1),
                                            );
                                            store.set(&thread_id_owned, session_data).await;
                                        }

                                        // Emit loop_continue event for the
                                        // gateway to pick up.
                                        if let Some(tx) = &*inner.event_tx.read().await {
                                            let event = serde_json::json!({
                                                "type": "loop_continue",
                                                "thread_id": thread_id_owned,
                                                "iteration": iteration_count + 1,
                                            });
                                            let _ = tx.send(event.to_string());
                                        }
                                        scheduled_loop_continue = true;

                                        record_thread_log(
                                            thread_logs_for_task.clone(),
                                            thread_log_id_owned.as_deref(),
                                            ThreadLogEvent::info(
                                                "",
                                                "loop",
                                                "loop auto-continue scheduled",
                                            )
                                            .with_run_id(run_id_owned.clone())
                                            .with_field("iteration", json!(iteration_count + 1)),
                                        )
                                        .await;
                                    }
                                }
                            }
                        }
                    }

                    if !scheduled_loop_continue {
                        mark_task_ready_for_review_after_stopped_run(
                            &inner,
                            &thread_id_owned,
                            &run_id_owned,
                            Some(&res.response),
                            thread_logs_for_task.clone(),
                            thread_log_id_owned.as_deref(),
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

                    if let (Some(store), Some(history)) = (
                        &*inner.thread_store.read().await,
                        &*inner.thread_history.read().await,
                    ) {
                        let user_images =
                            graph_state.run_options.images.clone().unwrap_or_default();
                        save_failed_thread_messages(
                            store,
                            history,
                            PersistedRun {
                                thread_id: &thread_id_owned,
                                user_message: &user_message,
                                user_images: &user_images,
                                assistant_response: failed_assistant_response,
                                sdk_session_id: None,
                                provider_key: &provider_key_owned,
                                provider_type: provider.provider_type(),
                                session_messages: failed_session_messages,
                                metadata: &graph_state.run_options.metadata,
                            },
                        )
                        .await;
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

                    // Emit run_error event.
                    if let Some(tx) = &*inner.event_tx.read().await {
                        let event = serde_json::json!({
                            "type": "run_error",
                            "run_id": run_id_owned,
                            "thread_id": thread_id_owned,
                            "error": e.to_string(),
                        });
                        let _ = tx.send(event.to_string());
                    }

                    // Error guard: disable loop on failure.
                    if let Some(store) = &*inner.thread_store.read().await {
                        if let Some(mut session_data) = store.get(&thread_id_owned).await {
                            if loop_enabled_from_value(&session_data) {
                                tracing::warn!(
                                    thread_id = %thread_id_owned,
                                    "loop run failed, disabling loop mode"
                                );
                                if let Some(obj) = session_data.as_object_mut() {
                                    obj.insert("loop_enabled".to_owned(), Value::Bool(false));
                                    obj.insert("loop_iteration_count".to_owned(), json!(0));
                                    store.set(&thread_id_owned, session_data).await;
                                }
                            }
                        }
                    }

                    mark_task_ready_for_review_after_stopped_run(
                        &inner,
                        &thread_id_owned,
                        &run_id_owned,
                        Some(failed_assistant_response),
                        thread_logs_for_task.clone(),
                        thread_log_id_owned.as_deref(),
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

        if let Some(store) = &*self.inner.thread_store.read().await {
            if let Some(session_data) = store.get(thread_id).await {
                let resolved_provider_type = provider.provider_type();
                if let Some(sid) = resolve_persisted_sdk_session_id_for_provider(
                    &session_data,
                    &provider_key,
                    Some(&resolved_provider_type),
                ) {
                    options
                        .metadata
                        .insert("sdk_session_id".to_owned(), Value::String(sid));
                }
            }
        }

        let partial_user_images = options.images.clone().unwrap_or_default();
        let partial_metadata = options.metadata.clone();
        let partial_provider_key = provider_key.clone();
        let partial_provider_type = provider.provider_type();
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
                    partial_user_images,
                    partial_provider_key,
                    partial_provider_type,
                    partial_metadata,
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

        let external_callback = response_callback.clone();
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
                let _ = partial_persistence_tx
                    .as_ref()
                    .map(|tx| tx.send(ThreadPersistenceCommand::Stream(event.clone())));
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
                        StreamEvent::Boundary { .. } | StreamEvent::Done => {}
                    }
                });
                if let Some(callback) = external_callback.as_ref() {
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
                    let sdk_session_id = resolve_sdk_session_id_for_persistence(
                        &graph_state.run_options.metadata,
                        res.sdk_session_id.as_deref(),
                    );
                    save_thread_messages(
                        store,
                        history,
                        PersistedRun {
                            thread_id,
                            user_message: message,
                            user_images: &user_images,
                            assistant_response: persisted_assistant_response,
                            sdk_session_id: sdk_session_id.as_deref(),
                            provider_key: &provider_key,
                            provider_type: provider.provider_type(),
                            session_messages: persisted_session_messages,
                            metadata: &graph_state.run_options.metadata,
                        },
                    )
                    .await;
                }
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
                    save_failed_thread_messages(
                        store,
                        history,
                        PersistedRun {
                            thread_id,
                            user_message: message,
                            user_images: &user_images,
                            assistant_response: failed_assistant_response,
                            sdk_session_id: None,
                            provider_key: &provider_key,
                            provider_type: provider.provider_type(),
                            session_messages: failed_session_messages,
                            metadata: &graph_state.run_options.metadata,
                        },
                    )
                    .await;
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
        if let Some(key) = provider_key {
            if let Some(provider) = self.get_provider(&key).await {
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
                    render_streaming_user_message_for_provider(&self.inner, thread_id, message)
                        .await;
                let pending_input = PendingUserInput {
                    id: format!("queued_input:{}", uuid::Uuid::new_v4()),
                    bridge_run_id: run_id.clone(),
                    text: message.to_owned(),
                    content: build_pending_input_content(
                        message,
                        &image_payloads,
                        &staged_attachments,
                    ),
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
        if let Some(key) = provider_key {
            if let Some(provider) = self.get_provider(&key).await {
                return provider.interrupt_streaming_session(thread_id).await;
            }
        }
        false
    }

    /// Abort a running agent request.
    pub async fn abort_run(&self, run_id: &str) -> bool {
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
        let provider_key = self
            .inner
            .run_index
            .read()
            .await
            .active_runs
            .get(run_id)
            .cloned();
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
        let mut run_index = self.inner.run_index.write().await;
        run_index.active_runs.remove(run_id);
        run_index.run_sessions.remove(run_id);

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
