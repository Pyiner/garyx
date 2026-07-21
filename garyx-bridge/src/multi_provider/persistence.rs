use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use chrono::Utc;
use garyx_models::provider::{
    ImagePayload, ProviderMessage, ProviderMessageRole, ProviderRateLimit, ProviderType,
    StreamEvent, attachments_from_metadata, build_user_content_from_parts,
};
use garyx_router::{
    DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, RECENT_COMMITTED_RUN_IDS_LIMIT,
    RunTranscriptRecordDraft, ThreadHistoryRepository, ThreadRecordPatch, ThreadStore,
    history_message_count,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::provider_common::metadata_string;

/// Maximum number of messages to keep per session (matches Python's limit).
pub(super) const MAX_SESSION_MESSAGES: usize = 100;
const PROVIDER_SDK_SESSION_IDS_KEY: &str = "provider_sdk_session_ids";
const RUN_PERSISTENCE_PATCH_FIELDS: &[&str] = &[
    "pending_user_inputs",
    "provider_sdk_session_ids",
    "provider_type",
    "provider_key",
    "sdk_session_id",
    "history",
    "last_user_preview",
    "last_assistant_preview",
    "updated_at",
];

async fn persist_run_record_patch(
    store: &Arc<dyn ThreadStore>,
    thread_id: &str,
    observed: &Value,
    desired: &Value,
    record_existed: bool,
) -> bool {
    let patch = match ThreadRecordPatch::from_diff(observed, desired, RUN_PERSISTENCE_PATCH_FIELDS)
    {
        Ok(patch) => patch,
        Err(error) => {
            warn!(thread_id, error = %error, "invalid run persistence patch");
            return false;
        }
    };
    if !record_existed {
        return match store.set(thread_id, desired.clone()).await {
            Ok(()) => true,
            Err(error) => {
                warn!(thread_id, error = %error, "initial run persistence record did not persist");
                false
            }
        };
    }
    match store.patch(thread_id, patch).await {
        Ok(_) => true,
        Err(error) => {
            warn!(thread_id, error = %error, "run persistence patch did not persist");
            false
        }
    }
}

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
    if let Some(origin_id) = metadata_string(metadata, "client_intent_id") {
        merged_metadata
            .entry("origin_id".to_owned())
            .or_insert(Value::String(origin_id));
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_id: Option<String>,
    /// Attribution metadata carried from the originating dispatch (e.g.
    /// `source`/`automation_id` for scheduled turns queued into an active
    /// run) and merged into the acknowledged user record.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, Value>,
    pub status: PendingUserInputStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct RunControlRecord {
    pub after_content_count: usize,
    pub timestamp: String,
    pub message: Value,
}

impl RunControlRecord {
    pub fn new(
        kind: &str,
        thread_id: &str,
        run_id: &str,
        at: String,
        mut payload: serde_json::Map<String, Value>,
        after_content_count: usize,
    ) -> Self {
        payload.insert("kind".to_owned(), Value::String(kind.to_owned()));
        payload.insert("thread_id".to_owned(), Value::String(thread_id.to_owned()));
        payload.insert("run_id".to_owned(), Value::String(run_id.to_owned()));
        payload.insert("at".to_owned(), Value::String(at.clone()));
        Self {
            after_content_count,
            timestamp: at,
            message: serde_json::json!({
                "role": "system",
                "kind": "control",
                "internal": true,
                "internal_kind": "control",
                "control": Value::Object(payload),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum CapsuleAttachmentAction {
    Created,
    Updated,
}

impl CapsuleAttachmentAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CapsuleMutationAttachment {
    pub action: CapsuleAttachmentAction,
    pub capsule_id: String,
    pub title: String,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct CapsuleAttachmentMarkerKey {
    tool_use_id: Option<String>,
    after_content_count: Option<usize>,
    capsule_id: String,
    revision: i64,
    action: CapsuleAttachmentAction,
}

impl CapsuleMutationAttachment {
    pub(super) fn marker_key(
        &self,
        tool_use_id: Option<&str>,
        after_content_count: usize,
    ) -> CapsuleAttachmentMarkerKey {
        let tool_use_id = tool_use_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let after_content_count = if tool_use_id.is_some() {
            None
        } else {
            Some(after_content_count)
        };
        CapsuleAttachmentMarkerKey {
            tool_use_id,
            after_content_count,
            capsule_id: self.capsule_id.clone(),
            revision: self.revision,
            action: self.action,
        }
    }
}

fn remember_capsule_tool_name(
    tool_names_by_id: &mut HashMap<String, String>,
    message: &ProviderMessage,
) {
    let Some(tool_use_id) = message
        .tool_use_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let Some(tool_name) = message
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    tool_names_by_id.insert(tool_use_id.to_owned(), tool_name.to_owned());
}

pub(super) fn capsule_attached_control_record(
    thread_id: &str,
    run_id: &str,
    attachment: &CapsuleMutationAttachment,
    after_content_count: usize,
) -> RunControlRecord {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "capsule_id".to_owned(),
        Value::String(attachment.capsule_id.clone()),
    );
    payload.insert(
        "revision".to_owned(),
        Value::Number(serde_json::Number::from(attachment.revision)),
    );
    payload.insert(
        "action".to_owned(),
        Value::String(attachment.action.as_str().to_owned()),
    );
    payload.insert("title".to_owned(), Value::String(attachment.title.clone()));
    RunControlRecord::new(
        "capsule_attached",
        thread_id,
        run_id,
        Utc::now().to_rfc3339(),
        payload,
        after_content_count,
    )
}

pub(super) fn extract_capsule_attachment_from_tool_result(
    message: &ProviderMessage,
    tool_names_by_id: &HashMap<String, String>,
) -> Option<CapsuleMutationAttachment> {
    if message.is_error == Some(true) {
        return None;
    }

    let action_hint = message
        .tool_name
        .as_deref()
        .and_then(capsule_action_from_tool_name)
        .or_else(|| {
            message
                .tool_use_id
                .as_deref()
                .and_then(|tool_use_id| tool_names_by_id.get(tool_use_id))
                .and_then(|tool_name| capsule_action_from_tool_name(tool_name))
        });

    extract_capsule_attachment_from_value(&message.content, action_hint, 0)
}

fn capsule_action_from_tool_name(tool_name: &str) -> Option<CapsuleAttachmentAction> {
    let normalized = tool_name.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    let tool = normalized
        .rsplit([':', '_'])
        .next()
        .unwrap_or(normalized.as_str());
    if normalized == "capsule_create"
        || normalized.ends_with(":capsule_create")
        || normalized.ends_with("__capsule_create")
        || tool == "capsule_create"
    {
        return Some(CapsuleAttachmentAction::Created);
    }
    if normalized == "capsule_update"
        || normalized.ends_with(":capsule_update")
        || normalized.ends_with("__capsule_update")
        || tool == "capsule_update"
    {
        return Some(CapsuleAttachmentAction::Updated);
    }
    None
}

fn extract_capsule_attachment_from_value(
    value: &Value,
    action_hint: Option<CapsuleAttachmentAction>,
    depth: usize,
) -> Option<CapsuleMutationAttachment> {
    if depth > 24 {
        return None;
    }
    match value {
        Value::Object(object) => {
            if let Some(attachment) = capsule_attachment_from_object(object, action_hint) {
                return Some(attachment);
            }
            for nested in object.values() {
                if let Some(attachment) =
                    extract_capsule_attachment_from_value(nested, action_hint, depth + 1)
                {
                    return Some(attachment);
                }
            }
            None
        }
        Value::Array(items) => items
            .iter()
            .find_map(|item| extract_capsule_attachment_from_value(item, action_hint, depth + 1)),
        Value::String(text) => parse_nested_json_value(text).and_then(|value| {
            extract_capsule_attachment_from_value(&value, action_hint, depth + 1)
        }),
        _ => None,
    }
}

fn capsule_attachment_from_object(
    object: &serde_json::Map<String, Value>,
    action_hint: Option<CapsuleAttachmentAction>,
) -> Option<CapsuleMutationAttachment> {
    let action = action_hint.or_else(|| capsule_action_from_payload_object(object))?;
    let capsule_id = object
        .get("capsule_id")
        .or_else(|| object.get("capsuleId"))
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let revision = object.get("revision").and_then(value_i64)?;
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();

    if action_hint.is_none() && !capsule_payload_self_identifies(object) {
        return None;
    }

    Some(CapsuleMutationAttachment {
        action,
        capsule_id,
        title,
        revision,
    })
}

fn capsule_action_from_payload_object(
    object: &serde_json::Map<String, Value>,
) -> Option<CapsuleAttachmentAction> {
    ["tool", "tool_name", "toolName", "name"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))
        .and_then(capsule_action_from_tool_name)
}

fn capsule_payload_self_identifies(object: &serde_json::Map<String, Value>) -> bool {
    capsule_action_from_payload_object(object).is_some()
        || object
            .get("open_url")
            .or_else(|| object.get("openUrl"))
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| value.starts_with("garyx://capsules/"))
}

fn value_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn parse_nested_json_value(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return None;
    }
    serde_json::from_str::<Value>(trimmed).ok()
}

pub(super) enum ThreadPersistenceCommand {
    Stream {
        event: StreamEvent,
        after_commit: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    },
    QueuePendingInput(PendingUserInput),
    DropPendingInput {
        pending_input_id: String,
    },
    AbortTerminal {
        error: Option<String>,
        ack: tokio::sync::oneshot::Sender<()>,
    },
    Finish,
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
    pub sdk_session_id: Option<String>,
    pub capsule_tool_names_by_id: HashMap<String, String>,
    pub emitted_capsule_markers: HashSet<CapsuleAttachmentMarkerKey>,
    start_new_assistant_segment: bool,
    current_assistant_metadata: Option<HashMap<String, Value>>,
}

fn provider_message_with_timestamp(message: &ProviderMessage) -> ProviderMessage {
    let mut message = message.clone();
    if message
        .timestamp
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        message.timestamp = Some(Utc::now().to_rfc3339());
    }
    message
}

impl StreamingRunSnapshot {
    pub fn apply_stream_event(&mut self, event: &StreamEvent) -> bool {
        match event {
            StreamEvent::SessionBound { sdk_session_id } => {
                let trimmed = sdk_session_id.trim();
                if trimmed.is_empty() || self.sdk_session_id.as_deref() == Some(trimmed) {
                    return false;
                }
                self.sdk_session_id = Some(trimmed.to_owned());
                true
            }
            StreamEvent::Delta { text } => {
                if text.is_empty() {
                    return false;
                }
                if self.start_new_assistant_segment && !self.assistant_response.is_empty() {
                    self.assistant_response.push_str("\n\n");
                }
                self.assistant_response.push_str(text);
                let current_metadata = self.current_assistant_metadata.clone();
                self.append_assistant_text(text, current_metadata.as_ref());
                self.start_new_assistant_segment = false;
                true
            }
            StreamEvent::ToolUse { message } => {
                let message = provider_message_with_timestamp(message);
                self.remember_tool_name(&message);
                self.session_messages.push(message);
                self.start_new_assistant_segment = true;
                self.current_assistant_metadata = None;
                true
            }
            StreamEvent::ToolResult { message } => {
                self.session_messages
                    .push(provider_message_with_timestamp(message));
                self.start_new_assistant_segment = true;
                self.current_assistant_metadata = None;
                true
            }
            StreamEvent::Boundary { .. } => {
                self.start_new_assistant_segment = true;
                self.current_assistant_metadata = None;
                false
            }
            StreamEvent::ThreadTitleUpdated { .. } => false,
            StreamEvent::Done => {
                self.start_new_assistant_segment = true;
                self.current_assistant_metadata = None;
                true
            }
        }
    }

    fn remember_tool_name(&mut self, message: &ProviderMessage) {
        remember_capsule_tool_name(&mut self.capsule_tool_names_by_id, message);
    }

    pub(super) fn capsule_attachment_for_tool_result(
        &self,
        message: &ProviderMessage,
    ) -> Option<CapsuleMutationAttachment> {
        extract_capsule_attachment_from_tool_result(message, &self.capsule_tool_names_by_id)
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
        if let Some(origin_id) = pending_input.origin_id.as_deref() {
            metadata.insert("origin_id".to_owned(), Value::String(origin_id.to_owned()));
        }
        // Attribution carried from the originating dispatch; built-in queue
        // markers above win on conflict.
        for (key, value) in &pending_input.metadata {
            metadata.entry(key.clone()).or_insert_with(|| value.clone());
        }
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

    fn append_assistant_text(
        &mut self,
        text_fragment: &str,
        metadata: Option<&HashMap<String, Value>>,
    ) {
        if !self.start_new_assistant_segment
            && let Some(last_message) = self.session_messages.last_mut()
            && last_message.role == ProviderMessageRole::Assistant
        {
            if let Some(text) = last_message.text.as_mut() {
                text.push_str(text_fragment);
            } else {
                last_message.text = Some(text_fragment.to_owned());
            }

            match &mut last_message.content {
                Value::String(text) => text.push_str(text_fragment),
                Value::Null => {
                    last_message.content = Value::String(text_fragment.to_owned());
                }
                other => {
                    let mut content = other
                        .as_str()
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| serde_json::to_string(other).unwrap_or_default());
                    content.push_str(text_fragment);
                    *other = Value::String(content);
                }
            }
            return;
        }

        // Stamp the segment at creation. Unstamped rows get backfilled with
        // `now()` on every partial save, which re-stamps the whole run's
        // assistant rows to the latest flush moment and destroys their real
        // ordering against tool rows.
        let mut message =
            ProviderMessage::assistant_text(text_fragment).with_timestamp(Utc::now().to_rfc3339());
        if let Some(metadata) = metadata {
            message.metadata = metadata.clone();
        }
        self.session_messages.push(message);
    }

    /// Number of session messages that are finalized (durably appendable). The
    /// trailing assistant segment is still in-flight while deltas may extend it
    /// (`start_new_assistant_segment == false`); everything before it is final.
    pub(super) fn finalized_len(&self) -> usize {
        let in_flight = !self.start_new_assistant_segment
            && matches!(
                self.session_messages.last().map(|message| &message.role),
                Some(ProviderMessageRole::Assistant)
            );
        self.session_messages.len() - usize::from(in_flight)
    }
}

pub(super) struct PersistedRun<'a> {
    pub thread_id: &'a str,
    pub user_message: &'a str,
    pub user_timestamp: Option<&'a str>,
    pub user_images: &'a [ImagePayload],
    pub assistant_response: &'a str,
    pub sdk_session_id: Option<&'a str>,
    pub provider_key: &'a str,
    pub provider_type: ProviderType,
    pub session_messages: &'a [ProviderMessage],
    pub metadata: &'a HashMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct TerminalRunControl {
    pub duration_ms: Option<i64>,
    pub success: Option<bool>,
    pub error: Option<String>,
    pub thread_title: Option<String>,
    /// Present when the run failed because the provider's rolling usage quota
    /// was exhausted. Promotes the terminal `run_complete` status to
    /// `rate_limited` and embeds the reset context for the render layer and the
    /// gateway auto-resend reactor.
    pub rate_limit: Option<ProviderRateLimit>,
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

fn history_object_mut(
    object: &mut serde_json::Map<String, Value>,
) -> Option<&mut serde_json::Map<String, Value>> {
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

fn update_history_state(
    object: &mut serde_json::Map<String, Value>,
    history: &Arc<ThreadHistoryRepository>,
    thread_id: &str,
    message_count: usize,
    last_message_at: Option<&str>,
    recent_committed_run_ids: &[String],
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
}

fn build_run_messages(run: &PersistedRun<'_>) -> Vec<Value> {
    let mut messages = Vec::new();
    let message_metadata = run_message_metadata(run);
    let user_timestamp = run
        .user_timestamp
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    messages.push(Value::Object(build_user_object(
        run.user_message,
        build_user_content(run.user_message, run.user_images, run.metadata),
        user_timestamp,
        &message_metadata,
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
        for (key, value) in &message_metadata {
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
                        &message_metadata,
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
            &message_metadata,
            false,
        )));
    }

    messages
}

fn build_run_record_drafts(
    content_messages: Vec<Value>,
    transcript_controls: &[RunControlRecord],
) -> Vec<RunTranscriptRecordDraft> {
    let mut controls = transcript_controls.to_vec();
    controls.sort_by_key(|control| control.after_content_count);
    let mut control_index = 0usize;
    let mut drafts = Vec::with_capacity(content_messages.len() + controls.len());

    while control_index < controls.len() && controls[control_index].after_content_count == 0 {
        drafts.push(RunTranscriptRecordDraft::with_timestamp(
            controls[control_index].message.clone(),
            controls[control_index].timestamp.clone(),
        ));
        control_index += 1;
    }

    for (offset, message) in content_messages.into_iter().enumerate() {
        let content_count = offset + 1;
        drafts.push(RunTranscriptRecordDraft::from_message(message));
        while control_index < controls.len()
            && controls[control_index].after_content_count <= content_count
        {
            drafts.push(RunTranscriptRecordDraft::with_timestamp(
                controls[control_index].message.clone(),
                controls[control_index].timestamp.clone(),
            ));
            control_index += 1;
        }
    }

    while control_index < controls.len() {
        drafts.push(RunTranscriptRecordDraft::with_timestamp(
            controls[control_index].message.clone(),
            controls[control_index].timestamp.clone(),
        ));
        control_index += 1;
    }

    drafts
}

fn last_committed_user_preview(
    authoritative: &[RunTranscriptRecordDraft],
    committed_len: usize,
) -> Option<String> {
    garyx_models::message_preview::last_message_preview_for_role(
        authoritative[..committed_len.min(authoritative.len())]
            .iter()
            .map(|draft| &draft.message),
        "user",
    )
}

/// Run metadata that exists only to configure the provider runtime and must
/// never be persisted into transcript records or queued pending inputs.
///
/// Both direct commits and busy-thread enqueue persistence use this exact
/// denylist so a dispatch cannot gain or lose durable metadata merely because
/// it raced an already-active run.
pub(super) const RUNTIME_ONLY_METADATA_KEYS: &[&str] = &[
    "garyx_mcp_auth_token",
    "remote_mcp_servers",
    "garyx_mcp_headers",
    "provider_env",
    "system_prompt",
    "developer_instructions",
    "desktop_antigravity_env",
    "sdk_session_fork",
];

pub(super) fn strip_runtime_only_metadata(metadata: &mut HashMap<String, Value>) {
    for key in RUNTIME_ONLY_METADATA_KEYS {
        metadata.remove(*key);
    }
}

fn run_message_metadata(run: &PersistedRun<'_>) -> HashMap<String, Value> {
    let mut metadata = run.metadata.clone();
    strip_runtime_only_metadata(&mut metadata);
    match run
        .sdk_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(sdk_session_id) => {
            metadata.insert(
                "sdk_session_id".to_owned(),
                Value::String(sdk_session_id.to_owned()),
            );
        }
        None => {
            metadata.remove("sdk_session_id");
        }
    }
    metadata
}

/// Stream the run's messages to the committed transcript in real time (F1).
///
/// Appends the newly-finalized rows (`build_run_messages` of everything except
/// the trailing in-flight assistant segment, beyond `already_appended`) to the
/// jsonl with a seq. Returns the running count of finalized rows now committed
/// (the caller's cursor).
pub(super) async fn save_streaming_partial(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    run: PersistedRun<'_>,
    pending_user_inputs: &[PendingUserInput],
    transcript_controls: &[RunControlRecord],
    finalized_len: usize,
    already_appended: usize,
) -> (usize, Vec<(u64, Value)>) {
    let run_id = primary_run_identifier(run.metadata);
    let finalized_len = finalized_len.min(run.session_messages.len());
    let finalized_run = PersistedRun {
        thread_id: run.thread_id,
        user_message: run.user_message,
        user_timestamp: run.user_timestamp,
        user_images: run.user_images,
        assistant_response: "",
        sdk_session_id: run.sdk_session_id,
        provider_key: run.provider_key,
        provider_type: run.provider_type.clone(),
        session_messages: &run.session_messages[..finalized_len],
        metadata: run.metadata,
    };
    // Drop synthesized delivery-mirror rows from the streaming commit. Whether a
    // mirror is synthesized depends on `has_explicit_assistant_messages`, which
    // can flip later in the same run (a message-tool turn that later emits real
    // assistant text), so a streamed mirror is not a stable prefix of the final
    // authoritative set. The terminal commit re-derives mirrors with full context
    // and `reconcile_run_records_tail` rewrites the tail to match; streaming only
    // commits the stable real session rows.
    let authoritative_content: Vec<Value> = build_run_messages(&finalized_run)
        .into_iter()
        .filter(|message| {
            message
                .get("metadata")
                .and_then(|metadata| metadata.get("delivery_mirror"))
                .and_then(Value::as_bool)
                != Some(true)
        })
        .collect();
    let content_count = authoritative_content.len();
    let authoritative = build_run_record_drafts(
        authoritative_content,
        &transcript_controls
            .iter()
            .filter(|control| control.after_content_count <= content_count)
            .cloned()
            .collect::<Vec<_>>(),
    );

    let mut appended = already_appended.min(authoritative.len());
    let mut committed_total: Option<usize> = None;
    // The (seq, message) rows this flush appended, for the per-thread stream's live
    // `committed_message` events (S5). Derived from the post-append total: seq is
    // gapless, so the K appended rows are the last K records, seqs (total-K+1)..=total.
    let mut committed_pairs: Vec<(u64, Value)> = Vec::new();
    if authoritative.len() > appended {
        let suffix_start = appended;
        match history
            .transcript_store()
            .append_run_records(
                run.thread_id,
                run_id.as_deref(),
                &authoritative[suffix_start..],
            )
            .await
        {
            Ok(result) => {
                let total = result.total_messages;
                let suffix = &authoritative[suffix_start..];
                let k = suffix.len();
                for (offset, draft) in suffix.iter().enumerate() {
                    committed_pairs.push(((total - k + 1 + offset) as u64, draft.message.clone()));
                }
                appended = authoritative.len();
                committed_total = Some(total);
            }
            Err(error) => {
                warn!(thread_id = %run.thread_id, error = %error, "failed to append streaming transcript");
            }
        }
    }

    // A failed record read must not fall back to an empty record: writing
    // that skeleton back would clobber thread metadata. Skip this
    // persistence tick instead; the next tick retries.
    let (mut session_data, record_existed) = match store.get(run.thread_id).await {
        Ok(Some(existing)) => (existing, true),
        Ok(None) => (serde_json::json!({}), false),
        Err(error) => {
            warn!(thread_id = %run.thread_id, error = %error, "failed to read thread record; skipping streaming persistence tick");
            return (appended, committed_pairs);
        }
    };
    let observed_session_data = session_data.clone();
    let committed_total = match committed_total {
        Some(total) => total,
        None => history
            .transcript_store()
            .message_count(run.thread_id)
            .await
            .unwrap_or(appended),
    };
    let message_count = committed_total;
    let recent_run_ids = recent_committed_run_ids_from_value(&session_data);
    let merged_pending_inputs = merge_pending_inputs_for_persistence(
        &session_data,
        run_id.as_deref(),
        pending_user_inputs,
        should_clear_abandoned_pending_inputs(&run),
    );

    if let Some(obj) = session_data.as_object_mut() {
        // Publish only from the authoritative prefix known to be committed.
        // A later user row can already be finalized while its append fails;
        // reading the full finalized set here would expose that uncommitted row.
        if let Some(preview) = last_committed_user_preview(&authoritative, appended) {
            obj.insert(
                garyx_models::message_preview::LAST_USER_PREVIEW_FIELD.to_owned(),
                Value::String(preview),
            );
        }
        obj.insert("pending_user_inputs".to_owned(), merged_pending_inputs);
        let sdk_session_update = match run.sdk_session_id {
            Some(sid) => SdkSessionUpdate::Set(sid),
            None => SdkSessionUpdate::Preserve,
        };
        update_provider_sdk_session_id(obj, run.provider_key, &sdk_session_update);
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
        if let SdkSessionUpdate::Set(sid) = sdk_session_update {
            obj.insert("sdk_session_id".to_owned(), Value::String(sid.to_owned()));
        }
        update_history_state(
            obj,
            history,
            run.thread_id,
            message_count,
            None,
            &recent_run_ids,
        );
        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
    }
    if !persist_run_record_patch(
        store,
        run.thread_id,
        &observed_session_data,
        &session_data,
        record_existed,
    )
    .await
    {
        warn!(thread_id = %run.thread_id, "thread record write did not persist");
    }
    (appended, committed_pairs)
}

/// Save user and provider-emitted messages to the thread store after a run completes.
#[cfg(test)]
pub(super) async fn save_thread_messages(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    run: PersistedRun<'_>,
) -> Vec<(u64, Value)> {
    save_thread_messages_with_terminal_control(store, history, run, &[], None).await
}

/// Serialize a `ProviderRateLimit` into the `rate_limit` control payload shape
/// consumed by the transcript run-state reducer. `will_auto_resend` is true
/// whenever a concrete reset time is known: the gateway resends both the
/// 5-hour and weekly windows the moment they recover.
fn rate_limit_control_value(rate_limit: &ProviderRateLimit) -> Value {
    let mut object = serde_json::Map::new();
    object.insert(
        "provider".to_owned(),
        Value::String(rate_limit.provider.clone()),
    );
    if let Some(reset_at) = &rate_limit.reset_at {
        object.insert("reset_at".to_owned(), Value::String(reset_at.clone()));
    }
    if let Some(window) = &rate_limit.window {
        object.insert("window".to_owned(), Value::String(window.clone()));
    }
    if let Some(used_percent) = rate_limit.used_percent {
        object.insert(
            "used_percent".to_owned(),
            Value::Number(used_percent.into()),
        );
    }
    if let Some(reached_type) = &rate_limit.reached_type {
        object.insert(
            "reached_type".to_owned(),
            Value::String(reached_type.clone()),
        );
    }
    if let Some(message) = &rate_limit.message {
        object.insert("message".to_owned(), Value::String(message.clone()));
    }
    object.insert(
        "will_auto_resend".to_owned(),
        Value::Bool(rate_limit.reset_at.is_some()),
    );
    Value::Object(object)
}

pub(super) async fn save_thread_messages_with_terminal_control(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    run: PersistedRun<'_>,
    transcript_controls: &[RunControlRecord],
    terminal_control: Option<TerminalRunControl>,
) -> Vec<(u64, Value)> {
    let sdk_session_update = match run.sdk_session_id {
        Some(sid) => SdkSessionUpdate::Set(sid),
        None => SdkSessionUpdate::Clear,
    };
    save_thread_messages_with_session_update(
        store,
        history,
        run,
        sdk_session_update,
        transcript_controls,
        terminal_control,
    )
    .await
}

/// Save messages produced by a failed run while preserving any previously
/// committed provider SDK session id for the thread.
pub(super) async fn save_failed_thread_messages_with_terminal_control(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    run: PersistedRun<'_>,
    transcript_controls: &[RunControlRecord],
    terminal_control: Option<TerminalRunControl>,
) -> Vec<(u64, Value)> {
    save_thread_messages_with_session_update(
        store,
        history,
        run,
        SdkSessionUpdate::Preserve,
        transcript_controls,
        terminal_control,
    )
    .await
}

async fn save_thread_messages_with_session_update(
    store: &Arc<dyn ThreadStore>,
    history: &Arc<ThreadHistoryRepository>,
    run: PersistedRun<'_>,
    sdk_session_update: SdkSessionUpdate<'_>,
    transcript_controls: &[RunControlRecord],
    terminal_control: Option<TerminalRunControl>,
) -> Vec<(u64, Value)> {
    // Same clobber guard as the streaming path: a read failure must not
    // degrade into writing a skeleton record over live metadata.
    let (mut session_data, record_existed) = match store.get(run.thread_id).await {
        Ok(Some(existing)) => (existing, true),
        Ok(None) => (serde_json::json!({}), false),
        Err(error) => {
            warn!(thread_id = %run.thread_id, error = %error, "failed to read thread record; skipping final run persistence");
            return Vec::new();
        }
    };
    let observed_session_data = session_data.clone();
    let run_messages = build_run_messages(&run);
    let current_run_id = primary_run_identifier(run.metadata);
    let existing_recent_run_ids = recent_committed_run_ids_from_value(&session_data);

    // The `messages` snapshot is no longer rebuilt or written: the committed
    // transcript is the only provider-session content source (#TASK-1864
    // batch 1c). Existing snapshots on legacy records are left untouched
    // until Batch 2's import strips them.
    let mut message_count = history_message_count(&session_data);
    let mut last_message_at = run_messages
        .last()
        .and_then(|message| message.get("timestamp"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let mut committed_pairs = Vec::new();
    let mut transcript_controls = transcript_controls.to_vec();
    if let Some(terminal_control) = terminal_control
        && let Some(run_id) = current_run_id.as_deref()
    {
        let after_content_count = run_messages.len();
        if let Some(title) = terminal_control
            .thread_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let mut payload = serde_json::Map::new();
            payload.insert("title".to_owned(), Value::String(title.to_owned()));
            transcript_controls.push(RunControlRecord::new(
                "thread_title_updated",
                run.thread_id,
                run_id,
                Utc::now().to_rfc3339(),
                payload,
                after_content_count,
            ));
        }

        let mut payload = serde_json::Map::new();
        if let Some(duration_ms) = terminal_control.duration_ms {
            payload.insert(
                "duration_ms".to_owned(),
                Value::Number(serde_json::Number::from(duration_ms)),
            );
        }
        let status = match (terminal_control.success, &terminal_control.rate_limit) {
            // A quota-exhaustion failure is surfaced as its own terminal status
            // so the render reducer can show a countdown banner and the gateway
            // can schedule an automatic resend, rather than a generic failure.
            (Some(false), Some(_)) => "rate_limited",
            (Some(true), _) => "completed",
            (Some(false), None) => "failed",
            (None, _) => "completed",
        };
        payload.insert("status".to_owned(), Value::String(status.to_owned()));
        if let Some(error) = terminal_control
            .error
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            payload.insert("error".to_owned(), Value::String(error.to_owned()));
        }
        if let Some(rate_limit) = &terminal_control.rate_limit {
            payload.insert(
                "rate_limit".to_owned(),
                rate_limit_control_value(rate_limit),
            );
        }
        transcript_controls.push(RunControlRecord::new(
            "run_complete",
            run.thread_id,
            run_id,
            Utc::now().to_rfc3339(),
            payload,
            after_content_count,
        ));
    }
    // The streaming worker (F1) already appended this run's finalized rows to the
    // committed transcript during the run. Reconcile the run's tail to the final
    // authoritative set instead of blindly appending again: this is idempotent —
    // a no-op when nothing changed, a cheap suffix-append for the trailing segment
    // that was still in flight at the last streaming flush, and a tail rewrite only
    // when a retry re-streamed divergent content. An unconditional append here
    // would now double-write every streamed row.
    let authoritative_records = build_run_record_drafts(run_messages.clone(), &transcript_controls);
    match history
        .transcript_store()
        .reconcile_run_records_tail(
            run.thread_id,
            current_run_id.as_deref().unwrap_or_default(),
            &authoritative_records,
        )
        .await
    {
        Ok(result) => {
            message_count = result.total_messages;
            last_message_at = result.last_message_at;
            committed_pairs = result
                .appended_records
                .into_iter()
                .map(|record| (record.seq, record.message))
                .collect();
        }
        Err(error) => {
            warn!(thread_id = %run.thread_id, error = %error, "failed to reconcile thread transcript tail");
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

    // Write-time preview fields mirror the committed transcript tail — the
    // same bounded content window the retired `messages` snapshot held
    // (reconcile above has already made this run's rows, including replay
    // retractions, authoritative). Missing roles clear their field
    // (#TASK-1864 batch 1, review #TASK-1882 finding 1).
    let preview_source = match history
        .provider_session_tail(run.thread_id, MAX_SESSION_MESSAGES)
        .await
    {
        Ok(messages) => messages,
        Err(error) => {
            warn!(
                thread_id = %run.thread_id,
                error = %error,
                "failed to read transcript tail for preview fields; keeping previous values"
            );
            Vec::new()
        }
    };

    if let Some(obj) = session_data.as_object_mut() {
        if !preview_source.is_empty() {
            for role in ["user", "assistant"] {
                let Some(field) = garyx_models::message_preview::preview_field_for_role(role)
                else {
                    continue;
                };
                match garyx_models::message_preview::last_message_preview_for_role(
                    preview_source.iter(),
                    role,
                ) {
                    Some(preview) => {
                        obj.insert(field.to_owned(), Value::String(preview));
                    }
                    None => {
                        obj.remove(field);
                    }
                }
            }
        }
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
        update_history_state(
            obj,
            history,
            run.thread_id,
            message_count,
            last_message_at.as_deref(),
            &recent_run_ids,
        );
        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
    }

    if !persist_run_record_patch(
        store,
        run.thread_id,
        &observed_session_data,
        &session_data,
        record_existed,
    )
    .await
    {
        warn!(thread_id = %run.thread_id, "thread record write did not persist");
    }
    committed_pairs
}

#[cfg(test)]
mod tests;
