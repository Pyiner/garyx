use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use chrono::Utc;
use garyx_models::provider::{
    ImagePayload, ProviderMessage, ProviderMessageRole, ProviderType, StreamEvent,
    attachments_from_metadata, build_user_content_from_parts,
};
use garyx_router::{
    DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, RECENT_COMMITTED_RUN_IDS_LIMIT, ThreadHistoryRepository,
    ThreadStore, history_message_count,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

/// Maximum number of messages to keep per session (matches Python's limit).
const MAX_SESSION_MESSAGES: usize = 100;
const PROVIDER_SDK_SESSION_IDS_KEY: &str = "provider_sdk_session_ids";

fn attach_run_fields(
    object: &mut serde_json::Map<String, Value>,
    metadata: &HashMap<String, Value>,
) {
    for key in [
        "client_run_id",
        "run_id",
        "automation_id",
        "cron_job_id",
        "source",
    ] {
        if let Some(value) = metadata.get(key) {
            object.insert(key.to_owned(), value.clone());
        }
    }
}

fn run_identifiers(metadata: &HashMap<String, Value>) -> Vec<String> {
    let mut identifiers = Vec::new();
    for key in ["bridge_run_id", "run_id", "client_run_id"] {
        let Some(value) = metadata
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if identifiers.iter().any(|existing| existing == value) {
            continue;
        }
        identifiers.push(value.to_owned());
    }
    identifiers
}

fn primary_run_identifier(metadata: &HashMap<String, Value>) -> Option<String> {
    run_identifiers(metadata).into_iter().next()
}

fn message_matches_run(message: &Value, run_identifiers: &[String]) -> bool {
    let Some(object) = message.as_object() else {
        return false;
    };
    let metadata = object.get("metadata").and_then(Value::as_object);

    ["bridge_run_id", "run_id", "client_run_id"]
        .into_iter()
        .filter_map(|key| {
            object
                .get(key)
                .and_then(Value::as_str)
                .or_else(|| {
                    metadata
                        .and_then(|fields| fields.get(key))
                        .and_then(Value::as_str)
                })
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .any(|candidate| run_identifiers.iter().any(|run_id| run_id == candidate))
}

fn is_internal_dispatch(metadata: &HashMap<String, Value>) -> bool {
    metadata
        .get("internal_dispatch")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn apply_internal_message_fields(
    object: &mut serde_json::Map<String, Value>,
    metadata: &HashMap<String, Value>,
) {
    if !is_internal_dispatch(metadata) {
        return;
    }

    object.insert("internal".to_owned(), Value::Bool(true));
    if let Some(kind) = metadata.get("internal_kind") {
        object.insert("internal_kind".to_owned(), kind.clone());
    }
    if let Some(origin) = metadata.get("loop_origin") {
        object.insert("loop_origin".to_owned(), origin.clone());
    }
}

fn build_user_content(
    user_message: &str,
    user_images: &[ImagePayload],
    metadata: &HashMap<String, Value>,
) -> Value {
    let attachments = attachments_from_metadata(metadata);
    build_user_content_from_parts(user_message, &attachments, user_images)
}

fn is_message_tool_name(tool_name: &str) -> bool {
    let trimmed = tool_name.trim();
    !trimmed.is_empty()
        && trimmed
            .rsplit(':')
            .next()
            .is_some_and(|value| value.eq_ignore_ascii_case("message"))
}

fn non_empty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .filter(|text| !text.trim().is_empty())
}

fn value_marks_message_tool(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            non_empty_string(map.get("tool"))
                .or_else(|| non_empty_string(map.get("tool_name")))
                .or_else(|| non_empty_string(map.get("toolName")))
                .or_else(|| non_empty_string(map.get("name")))
                .is_some_and(|name| is_message_tool_name(&name))
                || map.values().any(value_marks_message_tool)
        }
        Value::Array(items) => items.iter().any(value_marks_message_tool),
        _ => false,
    }
}

fn extract_message_tool_text(content: &Value) -> Option<String> {
    const POINTERS: &[&str] = &[
        "/text",
        "/input/text",
        "/input/params/text",
        "/arguments/text",
        "/args/text",
        "/params/text",
        "/result/text",
        "/result/input/text",
        "/result/input/params/text",
        "/result/arguments/text",
        "/result/args/text",
        "/result/params/text",
    ];

    for pointer in POINTERS {
        if let Some(text) = non_empty_string(content.pointer(pointer)) {
            return Some(text);
        }
    }
    None
}

fn is_message_tool_entry(entry: &ProviderMessage) -> bool {
    entry.tool_name.as_deref().is_some_and(is_message_tool_name)
        || value_marks_message_tool(&entry.content)
}

fn build_assistant_object(
    text: &str,
    metadata: &HashMap<String, Value>,
    delivery_mirror: bool,
) -> serde_json::Map<String, Value> {
    let assistant_timestamp = Utc::now().to_rfc3339();
    let mut assistant_object = ProviderMessage::assistant_text(text)
        .with_timestamp(assistant_timestamp)
        .to_json_value()
        .as_object()
        .cloned()
        .unwrap_or_default();

    let mut assistant_metadata = metadata.clone();
    if delivery_mirror {
        assistant_metadata.insert("delivery_mirror".to_owned(), Value::Bool(true));
        assistant_metadata.insert(
            "delivery_source".to_owned(),
            Value::String("message_tool".to_owned()),
        );
    }
    assistant_object.insert(
        "metadata".to_owned(),
        serde_json::to_value(assistant_metadata).unwrap_or(Value::Null),
    );
    attach_run_fields(&mut assistant_object, metadata);
    apply_internal_message_fields(&mut assistant_object, metadata);
    assistant_object
}

fn build_user_object(
    text: &str,
    content: Value,
    timestamp: String,
    metadata: &HashMap<String, Value>,
    extra_metadata: Option<HashMap<String, Value>>,
) -> serde_json::Map<String, Value> {
    let mut merged_metadata = metadata.clone();
    if let Some(extra_metadata) = extra_metadata {
        merged_metadata.extend(extra_metadata);
    }
    let mut user_object = ProviderMessage {
        role: ProviderMessageRole::User,
        content,
        text: if text.trim().is_empty() {
            None
        } else {
            Some(text.to_owned())
        },
        timestamp: Some(timestamp),
        metadata: merged_metadata,
        tool_use_id: None,
        tool_name: None,
        is_error: None,
    }
    .to_json_value()
    .as_object()
    .cloned()
    .unwrap_or_default();
    attach_run_fields(&mut user_object, metadata);
    apply_internal_message_fields(&mut user_object, metadata);
    user_object
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PendingUserInputStatus {
    Queued,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct PendingUserInput {
    pub id: String,
    pub bridge_run_id: String,
    pub text: String,
    pub content: Value,
    pub queued_at: String,
    pub status: PendingUserInputStatus,
}

#[derive(Debug, Clone)]
pub(super) enum ThreadPersistenceCommand {
    Stream(StreamEvent),
    QueuePendingInput(PendingUserInput),
    DropPendingInput { pending_input_id: String },
}

fn pending_inputs_from_value(value: &Value) -> Vec<PendingUserInput> {
    value
        .get("pending_user_inputs")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<PendingUserInput>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn should_clear_abandoned_pending_inputs(run: &PersistedRun<'_>) -> bool {
    !is_internal_dispatch(run.metadata) && !run.user_message.trim().is_empty()
}

fn merge_pending_inputs_for_persistence(
    session_data: &Value,
    current_run_id: Option<&str>,
    pending_user_inputs: &[PendingUserInput],
    clear_abandoned: bool,
) -> Value {
    let mut all_pending_inputs = pending_inputs_from_value(session_data);
    if let Some(current_run_id) = current_run_id {
        all_pending_inputs.retain(|input| input.bridge_run_id != current_run_id);
    }
    if clear_abandoned {
        // Once the user explicitly starts a new turn on the thread, older
        // abandoned follow-ups are no longer actionable and should stop
        // surfacing in history/UI state.
        all_pending_inputs.retain(|input| input.status != PendingUserInputStatus::Abandoned);
    }
    all_pending_inputs.extend(pending_user_inputs.iter().cloned());
    serde_json::to_value(all_pending_inputs).unwrap_or(Value::Array(Vec::new()))
}

#[derive(Debug, Clone, Default)]
pub(super) struct StreamingRunSnapshot {
    pub assistant_response: String,
    pub session_messages: Vec<ProviderMessage>,
    start_new_assistant_segment: bool,
    current_assistant_metadata: Option<HashMap<String, Value>>,
}

impl StreamingRunSnapshot {
    pub fn apply_stream_event(&mut self, event: &StreamEvent) -> bool {
        match event {
            StreamEvent::Delta { text } => {
                if text.is_empty() {
                    return false;
                }
                let (speaker_metadata, clean_text) = parse_agent_team_delta_prefix(text);
                let speaker_changed = speaker_metadata.is_some()
                    && speaker_metadata != self.current_assistant_metadata;
                if speaker_changed && !self.start_new_assistant_segment {
                    self.start_new_assistant_segment = true;
                }
                if self.start_new_assistant_segment && !self.assistant_response.is_empty() {
                    self.assistant_response.push_str("\n\n");
                }
                if let Some(metadata) = speaker_metadata {
                    self.current_assistant_metadata = Some(metadata);
                }
                self.assistant_response.push_str(&clean_text);
                let current_metadata = self.current_assistant_metadata.clone();
                self.append_assistant_delta(&clean_text, current_metadata.as_ref());
                self.start_new_assistant_segment = false;
                true
            }
            StreamEvent::ToolUse { message } => {
                self.session_messages.push(message.clone());
                self.start_new_assistant_segment = true;
                self.current_assistant_metadata = None;
                true
            }
            StreamEvent::ToolResult { message } => {
                self.session_messages.push(message.clone());
                self.start_new_assistant_segment = true;
                self.current_assistant_metadata = None;
                true
            }
            StreamEvent::Boundary { .. } => {
                self.start_new_assistant_segment = true;
                self.current_assistant_metadata = None;
                false
            }
            StreamEvent::Done => false,
        }
    }

    pub fn acknowledge_pending_input(&mut self, pending_input: &PendingUserInput) -> bool {
        let mut metadata = HashMap::new();
        metadata.insert(
            "queued_input_id".to_owned(),
            Value::String(pending_input.id.clone()),
        );
        metadata.insert(
            "queued_at".to_owned(),
            Value::String(pending_input.queued_at.clone()),
        );
        self.session_messages.push(ProviderMessage {
            role: ProviderMessageRole::User,
            content: pending_input.content.clone(),
            text: if pending_input.text.trim().is_empty() {
                None
            } else {
                Some(pending_input.text.clone())
            },
            timestamp: Some(Utc::now().to_rfc3339()),
            metadata,
            tool_use_id: None,
            tool_name: None,
            is_error: None,
        });
        self.start_new_assistant_segment = true;
        self.current_assistant_metadata = None;
        true
    }

    fn append_assistant_delta(&mut self, delta: &str, metadata: Option<&HashMap<String, Value>>) {
        if !self.start_new_assistant_segment {
            if let Some(last_message) = self.session_messages.last_mut() {
                if last_message.role == ProviderMessageRole::Assistant {
                    if let Some(text) = last_message.text.as_mut() {
                        text.push_str(delta);
                    } else {
                        last_message.text = Some(delta.to_owned());
                    }

                    match &mut last_message.content {
                        Value::String(text) => text.push_str(delta),
                        Value::Null => {
                            last_message.content = Value::String(delta.to_owned());
                        }
                        other => {
                            let mut content =
                                other.as_str().map(ToOwned::to_owned).unwrap_or_else(|| {
                                    serde_json::to_string(other).unwrap_or_default()
                                });
                            content.push_str(delta);
                            *other = Value::String(content);
                        }
                    }
                    return;
                }
            }
        }

        let mut message = ProviderMessage::assistant_text(delta);
        if let Some(metadata) = metadata {
            message.metadata = metadata.clone();
        }
        self.session_messages.push(message);
    }
}

fn parse_agent_team_delta_prefix(text: &str) -> (Option<HashMap<String, Value>>, String) {
    let Some(stripped) = text.strip_prefix('[') else {
        return (None, text.to_owned());
    };
    let Some(close_index) = stripped.find(']') else {
        return (None, text.to_owned());
    };
    let agent_id = stripped[..close_index].trim();
    if agent_id.is_empty() {
        return (None, text.to_owned());
    }
    let mut metadata = HashMap::new();
    metadata.insert("agent_id".to_owned(), Value::String(agent_id.to_owned()));
    metadata.insert(
        "agent_display_name".to_owned(),
        Value::String(agent_id.to_owned()),
    );
    (
        Some(metadata),
        stripped[close_index + 1..].trim_start().to_owned(),
    )
}

pub(super) struct PersistedRun<'a> {
    pub thread_id: &'a str,
    pub user_message: &'a str,
    pub user_images: &'a [ImagePayload],
    pub assistant_response: &'a str,
    pub sdk_session_id: Option<&'a str>,
    pub provider_key: &'a str,
    pub provider_type: ProviderType,
    pub session_messages: &'a [ProviderMessage],
    pub metadata: &'a HashMap<String, Value>,
}

enum SdkSessionUpdate<'a> {
    Preserve,
    Set(&'a str),
    Clear,
}

fn ensure_object<'a>(
    object: &'a mut serde_json::Map<String, Value>,
    key: &str,
) -> Option<&'a mut serde_json::Map<String, Value>> {
    if !object.get(key).is_some_and(Value::is_object) {
        object.insert(key.to_owned(), Value::Object(serde_json::Map::new()));
    }
    object.get_mut(key).and_then(Value::as_object_mut)
}

fn update_provider_sdk_session_id(
    object: &mut serde_json::Map<String, Value>,
    provider_key: &str,
    sdk_session_update: &SdkSessionUpdate<'_>,
) {
    let trimmed_provider_key = provider_key.trim();
    if trimmed_provider_key.is_empty() {
        return;
    }

    match sdk_session_update {
        SdkSessionUpdate::Preserve => {}
        SdkSessionUpdate::Set(sid) => {
            if let Some(map) = ensure_object(object, PROVIDER_SDK_SESSION_IDS_KEY) {
                map.insert(
                    trimmed_provider_key.to_owned(),
                    Value::String((*sid).to_owned()),
                );
            }
        }
        SdkSessionUpdate::Clear => {
            let should_remove_parent = object
                .get_mut(PROVIDER_SDK_SESSION_IDS_KEY)
                .and_then(Value::as_object_mut)
                .map(|map| {
                    map.remove(trimmed_provider_key);
                    map.is_empty()
                })
                .unwrap_or(false);
            if should_remove_parent {
                object.remove(PROVIDER_SDK_SESSION_IDS_KEY);
            }
        }
    }
}

fn history_object_mut<'a>(
    object: &'a mut serde_json::Map<String, Value>,
) -> Option<&'a mut serde_json::Map<String, Value>> {
    ensure_object(object, "history")
}

fn recent_committed_run_ids_from_value(value: &Value) -> Vec<String> {
    value
        .get("history")
        .and_then(|history| history.get("recent_committed_run_ids"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn record_recent_committed_run_id(
    existing: &[String],
    current_run_id: Option<&str>,
) -> Vec<String> {
    let mut run_ids = existing.to_vec();
    if let Some(run_id) = current_run_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        run_ids.retain(|existing| existing != run_id);
        run_ids.push(run_id.to_owned());
    }
    if run_ids.len() > RECENT_COMMITTED_RUN_IDS_LIMIT {
        let drop_count = run_ids.len() - RECENT_COMMITTED_RUN_IDS_LIMIT;
        run_ids.drain(0..drop_count);
    }
    run_ids
}

fn build_active_run_snapshot_value(
    run: &PersistedRun<'_>,
    pending_user_inputs: &[PendingUserInput],
) -> Value {
    let provider_key = run
        .provider_key
        .trim()
        .is_empty()
        .then_some(Value::Null)
        .unwrap_or_else(|| Value::String(run.provider_key.to_owned()));
    serde_json::json!({
        "run_id": primary_run_identifier(run.metadata),
        "provider_key": provider_key,
        "provider_type": run.provider_type,
        "assistant_response": (!run.assistant_response.trim().is_empty()).then_some(run.assistant_response.to_owned()),
        "messages": build_run_messages(run),
        "pending_user_inputs": serde_json::to_value(pending_user_inputs).unwrap_or(Value::Array(Vec::new())),
        "updated_at": Utc::now().to_rfc3339(),
    })
}

fn clear_active_run_snapshot(object: &mut serde_json::Map<String, Value>) {
    let should_remove_history = object
        .get_mut("history")
        .and_then(Value::as_object_mut)
        .map(|history| {
            history.remove("active_run_snapshot");
            history.is_empty()
        })
        .unwrap_or(false);
    if should_remove_history {
        object.remove("history");
    }
}

fn update_history_state(
    object: &mut serde_json::Map<String, Value>,
    history: &Arc<ThreadHistoryRepository>,
    thread_id: &str,
    message_count: usize,
    last_message_at: Option<&str>,
    recent_committed_run_ids: &[String],
    active_run_snapshot: Option<Value>,
) {
    let Some(history_obj) = history_object_mut(object) else {
        return;
    };
    history_obj.insert(
        "source".to_owned(),
        Value::String("transcript_v1".to_owned()),
    );
    if let Some(path) = history.transcript_store().transcript_path(thread_id) {
        history_obj.insert(
            "transcript_file".to_owned(),
            Value::String(path.display().to_string()),
        );
    }
    history_obj.insert(
        "message_count".to_owned(),
        Value::Number(serde_json::Number::from(message_count as u64)),
    );
    history_obj.insert(
        "snapshot_limit".to_owned(),
        Value::Number(serde_json::Number::from(
            DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT as u64,
        )),
    );
    history_obj.insert(
        "snapshot_truncated".to_owned(),
        Value::Bool(message_count > DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT),
    );
    match last_message_at {
        Some(value) if !value.trim().is_empty() => {
            history_obj.insert(
                "last_message_at".to_owned(),
                Value::String(value.to_owned()),
            );
        }
        _ => {
            history_obj.remove("last_message_at");
        }
    }
    history_obj.insert(
        "recent_committed_run_ids".to_owned(),
        Value::Array(
            recent_committed_run_ids
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    match active_run_snapshot {
        Some(snapshot) => {
            history_obj.insert("active_run_snapshot".to_owned(), snapshot);
        }
        None => {
            history_obj.remove("active_run_snapshot");
        }
    }
}

fn build_run_messages(run: &PersistedRun<'_>) -> Vec<Value> {
    let mut messages = Vec::new();

    messages.push(Value::Object(build_user_object(
        run.user_message,
        build_user_content(run.user_message, run.user_images, run.metadata),
        Utc::now().to_rfc3339(),
        run.metadata,
        None,
    )));

    // Preserve provider-emitted message order when it is available.
    let has_explicit_assistant_messages = run
        .session_messages
        .iter()
        .any(|entry| entry.role == ProviderMessageRole::Assistant);
    let should_synthesize_delivery_mirror =
        !has_explicit_assistant_messages && run.assistant_response.trim().is_empty();
    let mut pending_message_tool_texts_by_id = HashMap::<String, String>::new();
    let mut pending_message_tool_texts_fifo = VecDeque::<String>::new();

    for entry in run.session_messages {
        let mut object = entry
            .to_json_value()
            .as_object()
            .cloned()
            .unwrap_or_default();
        if object.get("timestamp").and_then(Value::as_str).is_none() {
            object.insert(
                "timestamp".to_owned(),
                Value::String(Utc::now().to_rfc3339()),
            );
        }
        for (key, value) in run.metadata {
            object.entry(key.clone()).or_insert_with(|| value.clone());
        }
        messages.push(Value::Object(object));

        if !should_synthesize_delivery_mirror {
            continue;
        }

        match entry.role {
            ProviderMessageRole::ToolUse if is_message_tool_entry(entry) => {
                if let Some(text) = extract_message_tool_text(&entry.content) {
                    if let Some(tool_use_id) = entry
                        .tool_use_id
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                    {
                        pending_message_tool_texts_by_id.insert(tool_use_id.to_owned(), text);
                    } else {
                        pending_message_tool_texts_fifo.push_back(text);
                    }
                }
            }
            ProviderMessageRole::ToolResult if !entry.is_error.unwrap_or(false) => {
                let mut mirrored_text = entry
                    .tool_use_id
                    .as_deref()
                    .and_then(|tool_use_id| pending_message_tool_texts_by_id.remove(tool_use_id));

                if mirrored_text.is_none() && is_message_tool_entry(entry) {
                    mirrored_text = extract_message_tool_text(&entry.content)
                        .or_else(|| pending_message_tool_texts_fifo.pop_front());
                }

                if let Some(text) = mirrored_text {
                    messages.push(Value::Object(build_assistant_object(
                        &text,
                        run.metadata,
                        true,
                    )));
                }
            }
            _ => {}
        }
    }

    if !has_explicit_assistant_messages && !run.assistant_response.is_empty() {
        messages.push(Value::Object(build_assistant_object(
            run.assistant_response,
            run.metadata,
            false,
        )));
    }

    messages
}

/// Save a partial streaming snapshot for the active run without clearing any
/// previously persisted SDK session id.
pub(super) async fn save_partial_thread_messages(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    run: PersistedRun<'_>,
    pending_user_inputs: &[PendingUserInput],
) {
    let mut session_data = store
        .get(run.thread_id)
        .await
        .unwrap_or_else(|| serde_json::json!({}));
    let current_message_count = history_message_count(&session_data);
    let recent_run_ids = recent_committed_run_ids_from_value(&session_data);
    let merged_pending_inputs = merge_pending_inputs_for_persistence(
        &session_data,
        primary_run_identifier(run.metadata).as_deref(),
        pending_user_inputs,
        should_clear_abandoned_pending_inputs(&run),
    );

    if let Some(obj) = session_data.as_object_mut() {
        if !obj.contains_key("messages") {
            obj.insert("messages".to_owned(), Value::Array(Vec::new()));
        }
        obj.insert("pending_user_inputs".to_owned(), merged_pending_inputs);
        update_provider_sdk_session_id(obj, run.provider_key, &SdkSessionUpdate::Preserve);
        obj.insert(
            "provider_type".to_owned(),
            serde_json::to_value(&run.provider_type).unwrap_or(Value::Null),
        );
        if run.provider_key.trim().is_empty() {
            obj.remove("provider_key");
        } else {
            obj.insert(
                "provider_key".to_owned(),
                Value::String(run.provider_key.to_owned()),
            );
        }
        update_history_state(
            obj,
            history,
            run.thread_id,
            current_message_count,
            None,
            &recent_run_ids,
            Some(build_active_run_snapshot_value(&run, pending_user_inputs)),
        );
        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
    }

    store.set(run.thread_id, session_data).await;
}

/// Save user and provider-emitted messages to the thread store after a run completes.
pub(super) async fn save_thread_messages(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    run: PersistedRun<'_>,
) {
    let sdk_session_update = match run.sdk_session_id {
        Some(sid) => SdkSessionUpdate::Set(sid),
        None => SdkSessionUpdate::Clear,
    };
    save_thread_messages_with_session_update(store, history, run, sdk_session_update).await;
}

/// Save messages produced by a failed run, clearing the active snapshot while
/// preserving any previously committed provider SDK session id for the thread.
pub(super) async fn save_failed_thread_messages(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    run: PersistedRun<'_>,
) {
    save_thread_messages_with_session_update(store, history, run, SdkSessionUpdate::Preserve).await;
}

async fn save_thread_messages_with_session_update(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    run: PersistedRun<'_>,
    sdk_session_update: SdkSessionUpdate<'_>,
) {
    let mut session_data = store
        .get(run.thread_id)
        .await
        .unwrap_or_else(|| serde_json::json!({}));
    let run_messages = build_run_messages(&run);
    let run_ids = run_identifiers(run.metadata);
    let current_run_id = primary_run_identifier(run.metadata);
    let existing_recent_run_ids = recent_committed_run_ids_from_value(&session_data);
    let already_committed = current_run_id.as_deref().is_some_and(|run_id| {
        existing_recent_run_ids
            .iter()
            .any(|existing| existing == run_id)
    });

    let mut snapshot_messages: Vec<Value> = session_data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !run_ids.is_empty() {
        snapshot_messages.retain(|message| !message_matches_run(message, &run_ids));
    }
    snapshot_messages.extend(run_messages.clone());
    if snapshot_messages.len() > MAX_SESSION_MESSAGES {
        let start = snapshot_messages.len() - MAX_SESSION_MESSAGES;
        snapshot_messages = snapshot_messages[start..].to_vec();
    }

    let mut message_count = history_message_count(&session_data).max(snapshot_messages.len());
    let mut last_message_at = snapshot_messages
        .last()
        .and_then(|message| message.get("timestamp"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if !already_committed {
        match history
            .transcript_store()
            .append_committed_messages(run.thread_id, current_run_id.as_deref(), &run_messages)
            .await
        {
            Ok(result) => {
                message_count = result.total_messages;
                last_message_at = result.last_message_at;
                history.enqueue_conversation_index_for_thread(run.thread_id);
            }
            Err(error) => {
                warn!(thread_id = %run.thread_id, error = %error, "failed to append thread transcript");
            }
        }
    }

    let merged_pending_inputs = merge_pending_inputs_for_persistence(
        &session_data,
        current_run_id.as_deref(),
        &[],
        should_clear_abandoned_pending_inputs(&run),
    );
    let recent_run_ids =
        record_recent_committed_run_id(&existing_recent_run_ids, current_run_id.as_deref());

    if let Some(obj) = session_data.as_object_mut() {
        obj.insert("messages".to_owned(), Value::Array(snapshot_messages));
        obj.insert("pending_user_inputs".to_owned(), merged_pending_inputs);
        update_provider_sdk_session_id(obj, run.provider_key, &sdk_session_update);
        obj.insert(
            "provider_type".to_owned(),
            serde_json::to_value(&run.provider_type).unwrap_or(Value::Null),
        );
        match sdk_session_update {
            SdkSessionUpdate::Preserve => {}
            SdkSessionUpdate::Set(sid) => {
                obj.insert("sdk_session_id".to_owned(), Value::String(sid.to_owned()));
            }
            SdkSessionUpdate::Clear => {
                obj.remove("sdk_session_id");
            }
        }
        if run.provider_key.trim().is_empty() {
            obj.remove("provider_key");
        } else {
            obj.insert(
                "provider_key".to_owned(),
                Value::String(run.provider_key.to_owned()),
            );
        }
        clear_active_run_snapshot(obj);
        update_history_state(
            obj,
            history,
            run.thread_id,
            message_count,
            last_message_at.as_deref(),
            &recent_run_ids,
            None,
        );
        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
    }

    store.set(run.thread_id, session_data).await;
}

#[cfg(test)]
mod tests;
