use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use garyx_models::local_paths::home_dir;
use garyx_models::provider::{
    AntigravityCliConfig, PromptAttachment, ProviderMessage, ProviderMessageRole,
    ProviderRunOptions, ProviderRunResult, ProviderType, SDK_SESSION_FORK_METADATA_KEY,
    StreamEvent, attachments_from_metadata, build_prompt_message_with_attachments,
    default_antigravity_model, stage_image_payloads_for_prompt,
};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::gary_prompt::{
    compose_gary_instructions, prepend_initial_context_to_user_message, task_cli_env,
};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_trait::{
    AgentLoopProvider, BridgeError, ProviderModelDefaults, ProviderRuntimeSelection, StreamCallback,
};

const DEFAULT_REQUEST_TIMEOUT_SECS: f64 = 300.0;
const TRANSCRIPT_POLL_INTERVAL: Duration = Duration::from_millis(250);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

fn resolve_run_id(metadata: &HashMap<String, Value>) -> String {
    metadata
        .get("bridge_run_id")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("client_run_id").and_then(Value::as_str))
        .or_else(|| metadata.get("run_id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("run_{}", Uuid::new_v4()))
}

fn metadata_string_map(metadata: &HashMap<String, Value>, key: &str) -> HashMap<String, String> {
    metadata
        .get(key)
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(name, value)| {
                    value.as_str().map(|value| (name.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_runtime_antigravity_env(
    config: &AntigravityCliConfig,
    metadata: &HashMap<String, Value>,
) -> HashMap<String, String> {
    let mut env = config.env.clone();
    env.extend(task_cli_env(metadata));
    env.extend(metadata_string_map(metadata, "desktop_antigravity_env"));
    env
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn metadata_bool(metadata: &HashMap<String, Value>, key: &str) -> bool {
    metadata.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn antigravity_bin(config: &AntigravityCliConfig) -> &str {
    let trimmed = config.antigravity_bin.trim();
    if trimmed.is_empty() { "agy" } else { trimmed }
}

fn model_id(config: &AntigravityCliConfig, metadata: &HashMap<String, Value>) -> String {
    normalize_non_empty(metadata.get("model").and_then(Value::as_str))
        .or_else(|| normalize_non_empty(Some(config.model.as_str())))
        .or_else(|| normalize_non_empty(Some(config.default_model.as_str())))
        .unwrap_or_else(default_antigravity_model)
}

fn request_timeout(config: &AntigravityCliConfig) -> Duration {
    let timeout = if config.timeout_seconds > 0.0 {
        config.timeout_seconds
    } else {
        DEFAULT_REQUEST_TIMEOUT_SECS
    };
    Duration::from_secs_f64(timeout)
}

fn outer_timeout(config: &AntigravityCliConfig) -> Duration {
    request_timeout(config).saturating_add(Duration::from_secs(10))
}

fn print_timeout_arg(timeout: Duration) -> String {
    format!("{}s", timeout.as_secs().max(1))
}

fn resolve_workspace_dir(
    config: &AntigravityCliConfig,
    options: &ProviderRunOptions,
) -> Option<PathBuf> {
    options
        .workspace_dir
        .as_ref()
        .or(config.workspace_dir.as_ref())
        .map(|value| PathBuf::from(shellexpand::tilde(value).as_ref()))
        .filter(|value| value.exists())
        .or_else(|| std::env::current_dir().ok())
}

fn configured_brain_root(config: &AntigravityCliConfig) -> Option<PathBuf> {
    normalize_non_empty(Some(config.antigravity_brain_root.as_str()))
        .map(|value| PathBuf::from(shellexpand::tilde(&value).as_ref()))
        .or_else(|| {
            home_dir().map(|home| home.join(".gemini").join("antigravity-cli").join("brain"))
        })
}

fn antigravity_base_dir(brain_root: &Path) -> PathBuf {
    if brain_root.file_name().and_then(|value| value.to_str()) == Some("brain")
        && let Some(parent) = brain_root.parent()
    {
        return parent.to_path_buf();
    }
    brain_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| brain_root.to_path_buf())
}

fn transcript_path(brain_root: &Path, conversation_id: &str) -> PathBuf {
    brain_root
        .join(conversation_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl")
}

fn transcript_full_path(compact_path: &Path) -> PathBuf {
    compact_path.with_file_name("transcript_full.jsonl")
}

fn run_log_path() -> PathBuf {
    let dir = std::env::temp_dir().join("garyx-antigravity");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("run-{}.log", Uuid::new_v4()))
}

fn build_prompt_text(
    options: &ProviderRunOptions,
    workspace_dir: Option<&Path>,
    include_instructions: bool,
) -> String {
    let mut attachments = attachments_from_metadata(&options.metadata);
    if attachments.is_empty() {
        attachments.extend(stage_image_payloads_for_prompt(
            "garyx-antigravity",
            options.images.as_deref().unwrap_or_default(),
        ));
    }
    build_prompt_text_from_attachments(options, workspace_dir, include_instructions, &attachments)
}

fn build_prompt_text_from_attachments(
    options: &ProviderRunOptions,
    workspace_dir: Option<&Path>,
    include_instructions: bool,
    attachments: &[PromptAttachment],
) -> String {
    let message = build_native_skill_prompt(&options.message, &options.metadata)
        .unwrap_or_else(|| options.message.clone());
    let message =
        prepend_initial_context_to_user_message(&message, &options.metadata, include_instructions);
    let user_message = build_prompt_message_with_attachments(&message, attachments);
    if !include_instructions {
        return user_message;
    }

    let runtime_system_prompt = options
        .metadata
        .get("system_prompt")
        .and_then(Value::as_str);
    let automation_id = options
        .metadata
        .get("automation_id")
        .and_then(Value::as_str);
    let instructions =
        compose_gary_instructions(runtime_system_prompt, workspace_dir, automation_id);

    if user_message.trim().is_empty() {
        format!("<system_instructions>\n{instructions}\n</system_instructions>")
    } else {
        format!(
            "<system_instructions>\n{instructions}\n</system_instructions>\n\n<user_request>\n{user_message}\n</user_request>"
        )
    }
}

fn build_command_args(
    prompt: &str,
    model: &str,
    conversation_id: Option<&str>,
    workspace_dir: Option<&Path>,
    log_path: &Path,
    timeout: Duration,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_owned(),
        prompt.to_owned(),
        "--model".to_owned(),
        model.to_owned(),
        "--dangerously-skip-permissions".to_owned(),
        "--print-timeout".to_owned(),
        print_timeout_arg(timeout),
        "--log-file".to_owned(),
        log_path.to_string_lossy().into_owned(),
    ];
    if let Some(conversation_id) = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push("--conversation".to_owned());
        args.push(conversation_id.to_owned());
    }
    if let Some(workspace_dir) = workspace_dir {
        args.push("--add-dir".to_owned());
        args.push(workspace_dir.to_string_lossy().into_owned());
    }
    args
}

#[derive(Debug, Clone)]
struct TranscriptRow {
    row_type: String,
    step_index: Option<i64>,
    created_at: Option<String>,
    content: Option<Value>,
    thinking: Option<Value>,
    tool_calls: Option<Value>,
    error: Option<Value>,
    is_truncated: bool,
    truncated_fields: HashSet<String>,
    raw: Value,
}

impl TranscriptRow {
    fn from_value(raw: Value) -> Option<Self> {
        let row_type = raw
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_owned();
        Some(Self {
            row_type,
            step_index: raw.get("step_index").and_then(value_i64),
            created_at: raw
                .get("created_at")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            content: raw.get("content").filter(|value| !value.is_null()).cloned(),
            thinking: raw
                .get("thinking")
                .filter(|value| !value.is_null())
                .cloned(),
            tool_calls: raw
                .get("tool_calls")
                .filter(|value| !value.is_null())
                .cloned(),
            error: raw.get("error").filter(|value| !value.is_null()).cloned(),
            is_truncated: raw
                .get("is_truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            truncated_fields: truncated_fields_from_value(&raw),
            raw,
        })
    }
}

fn truncated_fields_from_value(raw: &Value) -> HashSet<String> {
    match raw.get("truncated_fields") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        Some(Value::String(field)) => {
            let field = field.trim();
            if field.is_empty() {
                HashSet::new()
            } else {
                HashSet::from([field.to_owned()])
            }
        }
        _ => HashSet::new(),
    }
}

fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
}

fn parse_jsonl_rows(contents: &str) -> Vec<TranscriptRow> {
    contents
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<Value>(trimmed)
                .ok()
                .and_then(TranscriptRow::from_value)
        })
        .collect()
}

fn read_jsonl_rows(path: &Path) -> Vec<TranscriptRow> {
    std::fs::read_to_string(path)
        .map(|contents| parse_jsonl_rows(&contents))
        .unwrap_or_default()
}

fn value_payload_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        _ => value.to_string().len(),
    }
}

fn field_payload_richer(
    compact_field: Option<&Value>,
    full_field: Option<&Value>,
    compact_field_truncated: bool,
) -> bool {
    let Some(full_field) = full_field else {
        return false;
    };
    if compact_field_truncated {
        return true;
    }
    compact_field.is_none_or(|compact_field| {
        full_field != compact_field
            && value_payload_len(full_field) > value_payload_len(compact_field)
    })
}

fn row_payload_richer(compact: &TranscriptRow, full: &TranscriptRow) -> bool {
    compact.is_truncated
        || field_payload_richer(
            compact.content.as_ref(),
            full.content.as_ref(),
            compact.truncated_fields.contains("content"),
        )
        || field_payload_richer(
            compact.thinking.as_ref(),
            full.thinking.as_ref(),
            compact.truncated_fields.contains("thinking"),
        )
        || field_payload_richer(
            compact.tool_calls.as_ref(),
            full.tool_calls.as_ref(),
            compact.truncated_fields.contains("tool_calls"),
        )
        || field_payload_richer(
            compact.error.as_ref(),
            full.error.as_ref(),
            compact.truncated_fields.contains("error"),
        )
}

fn read_transcript_rows(compact_path: &Path) -> Vec<TranscriptRow> {
    let compact_rows = read_jsonl_rows(compact_path);
    if compact_rows.is_empty() {
        return compact_rows;
    }
    let full_path = transcript_full_path(compact_path);
    let full_rows = read_jsonl_rows(&full_path);
    if full_rows.is_empty() {
        return compact_rows;
    }
    let mut full_by_step = full_rows
        .into_iter()
        .filter_map(|row| row.step_index.map(|step| (step, row)))
        .collect::<HashMap<_, _>>();
    compact_rows
        .into_iter()
        .map(|row| {
            if let Some(step) = row.step_index
                && let Some(full_row) = full_by_step.remove(&step)
                && row_payload_richer(&row, &full_row)
            {
                return full_row;
            }
            row
        })
        .collect()
}

fn max_step_index(compact_path: &Path) -> i64 {
    read_transcript_rows(compact_path)
        .into_iter()
        .filter_map(|row| row.step_index)
        .max()
        .unwrap_or(-1)
}

fn row_value_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| text.to_owned())
        }
        Value::Array(_) | Value::Object(_) => Some(value?.to_string()),
        other => {
            let text = other.to_string();
            (!text.trim().is_empty()).then_some(text)
        }
    }
}

fn normalize_tool_name(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn canonical_tool_name(value: &str) -> String {
    let normalized = normalize_tool_name(value);
    match normalized.as_str() {
        "list_dir" | "list_directory" => "list_directory".to_owned(),
        _ => normalized,
    }
}

fn provider_message_timestamp(row: &TranscriptRow) -> String {
    row.created_at
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339())
}

fn source_metadata() -> Value {
    json!("antigravity")
}

fn append_antigravity_assistant_session_message(
    session_messages: &mut Vec<ProviderMessage>,
    delta: &str,
    row: &TranscriptRow,
    reasoning: Option<&str>,
) {
    if delta.is_empty() {
        return;
    }
    let can_append = session_messages.last().is_some_and(|message| {
        message.role == ProviderMessageRole::Assistant
            && message.metadata.get("source").and_then(Value::as_str) == Some("antigravity")
    });
    if can_append {
        if let Some(last) = session_messages.last_mut() {
            let mut text = last.text.clone().unwrap_or_default();
            text.push_str(delta);
            last.text = Some(text.clone());
            last.content = Value::String(text);
            if let Some(reasoning) = reasoning.filter(|value| !value.trim().is_empty()) {
                let current = last
                    .metadata
                    .get("provider_reasoning")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let joined = if current.is_empty() {
                    reasoning.to_owned()
                } else {
                    format!("{current}\n\n{reasoning}")
                };
                last.metadata
                    .insert("provider_reasoning".to_owned(), Value::String(joined));
            }
        }
        return;
    }

    let mut entry = ProviderMessage::assistant_text(delta)
        .with_timestamp(provider_message_timestamp(row))
        .with_metadata_value("source", source_metadata());
    if let Some(reasoning) = reasoning.filter(|value| !value.trim().is_empty()) {
        entry = entry.with_metadata_value("provider_reasoning", json!(reasoning));
    }
    session_messages.push(entry);
}

#[derive(Debug, Clone)]
struct ToolCall {
    name: String,
    args: Value,
}

fn parse_tool_args(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => {
            serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        Some(value) => value.clone(),
        None => Value::Null,
    }
}

fn transcript_tool_calls(row: &TranscriptRow) -> Vec<ToolCall> {
    let Some(Value::Array(items)) = row.tool_calls.as_ref() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| item.get("functionName").and_then(Value::as_str))
                .or_else(|| item.get("tool_name").and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())?
                .to_owned();
            let args = parse_tool_args(item.get("args").or_else(|| item.get("input")));
            Some(ToolCall { name, args })
        })
        .collect()
}

fn skip_visible_row(row_type: &str) -> bool {
    matches!(
        row_type,
        "USER_INPUT" | "CONVERSATION_HISTORY" | "SYSTEM_MESSAGE" | "CHECKPOINT"
    )
}

fn is_tool_result_row(row: &TranscriptRow) -> bool {
    if skip_visible_row(&row.row_type) || row.row_type == "PLANNER_RESPONSE" {
        return false;
    }
    row.content.is_some()
}

#[derive(Default)]
struct TranscriptMapper {
    response: String,
    session_messages: Vec<ProviderMessage>,
    processed_steps: HashSet<i64>,
    pending_tools: HashMap<String, VecDeque<String>>,
    anonymous_pending: VecDeque<String>,
    pending_reasoning: String,
    error: Option<String>,
}

impl TranscriptMapper {
    fn new() -> Self {
        Self::default()
    }

    fn apply_rows(
        &mut self,
        rows: Vec<TranscriptRow>,
        baseline_step_index: i64,
        on_chunk: &StreamCallback,
    ) {
        for row in rows {
            let Some(step_index) = row.step_index else {
                continue;
            };
            if step_index <= baseline_step_index || !self.processed_steps.insert(step_index) {
                continue;
            }
            self.apply_row(row, on_chunk);
        }
    }

    fn apply_row(&mut self, row: TranscriptRow, on_chunk: &StreamCallback) {
        match row.row_type.as_str() {
            "PLANNER_RESPONSE" => self.apply_planner_response(row, on_chunk),
            "ERROR_MESSAGE" => self.apply_error_message(row, on_chunk),
            row_type if skip_visible_row(row_type) => {}
            _ if is_tool_result_row(&row) => self.apply_tool_result(row, on_chunk),
            _ => {}
        }
    }

    fn apply_planner_response(&mut self, row: TranscriptRow, on_chunk: &StreamCallback) {
        if let Some(thinking) = row_value_text(row.thinking.as_ref()) {
            if !self.pending_reasoning.is_empty() {
                self.pending_reasoning.push_str("\n\n");
            }
            self.pending_reasoning.push_str(&thinking);
        }

        for (index, call) in transcript_tool_calls(&row).into_iter().enumerate() {
            let step = row.step_index.unwrap_or_default();
            let tool_use_id = format!("antigravity-tool-{step}-{index}");
            let content = json!({
                "name": call.name,
                "args": call.args,
            });
            let message = ProviderMessage::tool_use(
                content,
                Some(tool_use_id.clone()),
                Some(call.name.clone()),
            )
            .with_timestamp(provider_message_timestamp(&row))
            .with_metadata_value("source", source_metadata());
            self.pending_tools
                .entry(canonical_tool_name(&call.name))
                .or_default()
                .push_back(tool_use_id.clone());
            self.anonymous_pending.push_back(tool_use_id);
            on_chunk(StreamEvent::ToolUse {
                message: message.clone(),
            });
            self.session_messages.push(message);
        }

        let Some(delta) = row_value_text(row.content.as_ref()) else {
            return;
        };
        self.response.push_str(&delta);
        on_chunk(StreamEvent::Delta {
            text: delta.clone(),
        });
        let reasoning = (!self.pending_reasoning.is_empty()).then(|| {
            let value = self.pending_reasoning.clone();
            self.pending_reasoning.clear();
            value
        });
        append_antigravity_assistant_session_message(
            &mut self.session_messages,
            &delta,
            &row,
            reasoning.as_deref(),
        );
    }

    fn apply_tool_result(&mut self, row: TranscriptRow, on_chunk: &StreamCallback) {
        let tool_name = row.row_type.clone();
        let tool_use_id = self.pop_tool_id_for_name(&tool_name);
        let message =
            ProviderMessage::tool_result(row.raw.clone(), tool_use_id, Some(tool_name), None)
                .with_timestamp(provider_message_timestamp(&row))
                .with_metadata_value("source", source_metadata());
        on_chunk(StreamEvent::ToolResult {
            message: message.clone(),
        });
        self.session_messages.push(message);
    }

    fn apply_error_message(&mut self, row: TranscriptRow, on_chunk: &StreamCallback) {
        let error = row_value_text(row.error.as_ref())
            .or_else(|| row_value_text(row.content.as_ref()))
            .unwrap_or_else(|| "antigravity CLI reported an error".to_owned());
        self.error = Some(error.clone());
        if let Some(tool_use_id) = self.pop_any_tool_id() {
            let mut content = Map::new();
            content.insert("type".to_owned(), Value::String(row.row_type.clone()));
            content.insert("error".to_owned(), Value::String(error));
            let message = ProviderMessage::tool_result(
                Value::Object(content),
                Some(tool_use_id),
                Some(row.row_type.clone()),
                Some(true),
            )
            .with_timestamp(provider_message_timestamp(&row))
            .with_metadata_value("source", source_metadata());
            on_chunk(StreamEvent::ToolResult {
                message: message.clone(),
            });
            self.session_messages.push(message);
        }
    }

    fn pop_tool_id_for_name(&mut self, tool_name: &str) -> Option<String> {
        let normalized = canonical_tool_name(tool_name);
        let mut keys = vec![normalized.clone()];
        if let Some(stripped) = normalized.strip_suffix("_result") {
            keys.push(canonical_tool_name(stripped));
        }
        for key in keys {
            if let Some(queue) = self.pending_tools.get_mut(&key)
                && let Some(tool_use_id) = queue.pop_front()
            {
                self.remove_anonymous_pending(&tool_use_id);
                return Some(tool_use_id);
            }
        }
        self.pop_any_tool_id()
    }

    fn pop_any_tool_id(&mut self) -> Option<String> {
        let tool_use_id = self.anonymous_pending.pop_front()?;
        for queue in self.pending_tools.values_mut() {
            if let Some(position) = queue.iter().position(|value| value == &tool_use_id) {
                queue.remove(position);
                break;
            }
        }
        Some(tool_use_id)
    }

    fn remove_anonymous_pending(&mut self, tool_use_id: &str) {
        if let Some(position) = self
            .anonymous_pending
            .iter()
            .position(|value| value == tool_use_id)
        {
            self.anonymous_pending.remove(position);
        }
    }
}

fn is_invalid_session_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("session not found")
        || lower.contains("invalid session")
        || lower.contains("conversation not found")
}

fn append_process_output(
    message: impl Into<String>,
    stdout_output: &str,
    stderr_output: &str,
) -> String {
    let mut message = message.into();
    let stdout_output = stdout_output.trim();
    let stderr_output = stderr_output.trim();
    if !stderr_output.is_empty() {
        message.push_str(" | stderr: ");
        message.push_str(stderr_output);
    }
    if !stdout_output.is_empty() {
        message.push_str(" | stdout: ");
        message.push_str(stdout_output);
    }
    message
}

async fn read_stream_to_string<T>(stream: T) -> String
where
    T: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream).lines();
    let mut output = Vec::new();
    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            output.push(trimmed.to_owned());
        }
    }
    output.join("\n")
}

fn uuid_candidates(contents: &str) -> Vec<String> {
    contents
        .split(|ch: char| !(ch.is_ascii_hexdigit() || ch == '-'))
        .filter(|candidate| {
            candidate.len() == 36
                && candidate.chars().enumerate().all(|(index, ch)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        ch == '-'
                    } else {
                        ch.is_ascii_hexdigit()
                    }
                })
        })
        .map(ToOwned::to_owned)
        .collect()
}

fn discover_from_run_log(log_path: &Path, brain_root: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(log_path).ok()?;
    uuid_candidates(&contents)
        .into_iter()
        .find(|candidate| transcript_path(brain_root, candidate).exists())
}

fn normalized_contains(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.split_whitespace().collect::<Vec<_>>().join(" ");
    let needle = needle.split_whitespace().collect::<Vec<_>>().join(" ");
    let needle = needle.trim();
    !needle.is_empty() && haystack.contains(needle)
}

fn prompt_matches_text(prompt_text: &str, prompt: &str, user_message: &str) -> bool {
    if normalized_contains(prompt_text, prompt) || normalized_contains(prompt, prompt_text) {
        return true;
    }
    let prompt_prefix = prompt.chars().take(512).collect::<String>();
    normalized_contains(prompt_text, &prompt_prefix)
        || normalized_contains(prompt_text, user_message)
}

fn conversation_matches_prompt(
    brain_root: &Path,
    conversation_id: &str,
    prompt: &str,
    user_message: &str,
) -> bool {
    let path = transcript_path(brain_root, conversation_id);
    read_transcript_rows(&path)
        .into_iter()
        .find(|row| row.row_type == "USER_INPUT")
        .and_then(|row| row_value_text(row.content.as_ref()))
        .is_some_and(|text| prompt_matches_text(&text, prompt, user_message))
}

fn candidate_conversation_ids(
    conversations_dir: &Path,
    run_start: SystemTime,
) -> Vec<(String, SystemTime)> {
    let threshold = run_start
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(run_start);
    let mut candidates = std::fs::read_dir(conversations_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("db") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            if modified < threshold {
                return None;
            }
            let id = path.file_stem()?.to_str()?.trim().to_owned();
            (!id.is_empty()).then_some((id, modified))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(_, modified)| *modified);
    candidates
}

fn discover_from_conversations(
    conversations_dir: &Path,
    brain_root: &Path,
    run_start: SystemTime,
    prompt: &str,
    user_message: &str,
) -> Option<String> {
    let candidates = candidate_conversation_ids(conversations_dir, run_start);
    if candidates.is_empty() {
        return None;
    }
    let matched = candidates
        .iter()
        .filter(|(id, _)| conversation_matches_prompt(brain_root, id, prompt, user_message))
        .cloned()
        .collect::<Vec<_>>();
    let selected = if matched.is_empty() && candidates.len() == 1 {
        candidates
    } else {
        matched
    };
    selected
        .into_iter()
        .max_by_key(|(_, modified)| *modified)
        .map(|(id, _)| id)
}

pub struct AntigravityCliProvider {
    config: AntigravityCliConfig,
    /// Hot-reloadable model defaults. Config reloads reconcile onto the live
    /// provider instance (the provider key excludes model defaults to keep
    /// thread affinity stable), so model resolution must read these instead
    /// of the frozen `config` fields.
    model_defaults: std::sync::RwLock<ProviderModelDefaults>,
    session_map: Mutex<HashMap<String, String>>,
    active_runs: Mutex<HashMap<String, Arc<Mutex<Child>>>>,
    run_session_map: Mutex<HashMap<String, String>>,
    fresh_session_lock: Mutex<()>,
    ready: bool,
}

impl AntigravityCliProvider {
    pub fn new(config: AntigravityCliConfig) -> Self {
        let model_defaults = std::sync::RwLock::new(ProviderModelDefaults {
            model: config.model.clone(),
            default_model: config.default_model.clone(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
        });
        Self {
            config,
            model_defaults,
            session_map: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            run_session_map: Mutex::new(HashMap::new()),
            fresh_session_lock: Mutex::new(()),
            ready: false,
        }
    }

    /// Clone the frozen config with the hot-reloadable model defaults
    /// overlaid, so model resolution observes the latest reloaded defaults.
    fn effective_config(&self) -> AntigravityCliConfig {
        let defaults = self
            .model_defaults
            .read()
            .expect("antigravity model defaults lock poisoned")
            .clone();
        let mut config = self.config.clone();
        config.model = if defaults.model.is_empty() {
            defaults.default_model.clone()
        } else {
            defaults.model.clone()
        };
        config.default_model = defaults.default_model;
        config
    }

    async fn register_run(&self, run_id: &str, thread_id: &str, child: Arc<Mutex<Child>>) {
        self.active_runs
            .lock()
            .await
            .insert(run_id.to_owned(), child);
        self.run_session_map
            .lock()
            .await
            .insert(run_id.to_owned(), thread_id.to_owned());
    }

    async fn unregister_run(&self, run_id: &str) -> (Option<Arc<Mutex<Child>>>, Option<String>) {
        let child = self.active_runs.lock().await.remove(run_id);
        let thread_id = self.run_session_map.lock().await.remove(run_id);
        (child, thread_id)
    }

    async fn cleanup_run_io(
        &self,
        child: Option<Arc<Mutex<Child>>>,
        stdout_task: tokio::task::JoinHandle<String>,
        stderr_task: tokio::task::JoinHandle<String>,
        kill_child: bool,
    ) -> (String, String) {
        tokio::time::timeout(Duration::from_secs(2), async move {
            if let Some(child) = child {
                let mut child = child.lock().await;
                if kill_child {
                    let _ = child.kill().await;
                }
                let _ = child.wait().await;
            }
            let stdout = stdout_task.await.unwrap_or_default();
            let stderr = stderr_task.await.unwrap_or_default();
            (stdout, stderr)
        })
        .await
        .unwrap_or_default()
    }

    async fn discover_conversation_id(
        &self,
        run_log: &Path,
        brain_root: &Path,
        conversations_dir: &Path,
        run_start: SystemTime,
        prompt: &str,
        user_message: &str,
    ) -> Option<String> {
        let started = Instant::now();
        loop {
            if let Some(id) = discover_from_run_log(run_log, brain_root) {
                return Some(id);
            }
            if let Some(id) = discover_from_conversations(
                conversations_dir,
                brain_root,
                run_start,
                prompt,
                user_message,
            ) {
                return Some(id);
            }
            if started.elapsed() >= DISCOVERY_TIMEOUT {
                return None;
            }
            tokio::time::sleep(TRANSCRIPT_POLL_INTERVAL).await;
        }
    }

    async fn run_once(
        &self,
        options: &ProviderRunOptions,
        run_id: &str,
        session_id: Option<&str>,
        on_chunk: &StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let workspace_dir = resolve_workspace_dir(&self.config, options);
        let cwd = workspace_dir.as_ref().ok_or_else(|| {
            BridgeError::RunFailed("antigravity workspace directory is unavailable".to_owned())
        })?;
        let brain_root = configured_brain_root(&self.config).ok_or_else(|| {
            BridgeError::RunFailed("antigravity brain root is unavailable".to_owned())
        })?;
        let conversations_dir = antigravity_base_dir(&brain_root).join("conversations");
        let timeout = request_timeout(&self.config);
        let model = model_id(&self.effective_config(), &options.metadata);
        let prompt = build_prompt_text(options, Some(cwd.as_path()), session_id.is_none());
        let run_log = run_log_path();
        let baseline_step_index = session_id
            .map(|id| max_step_index(&transcript_path(&brain_root, id)))
            .unwrap_or(-1);
        let fresh_guard = if session_id.is_none() {
            Some(self.fresh_session_lock.lock().await)
        } else {
            None
        };
        let run_start = SystemTime::now();
        let args = build_command_args(
            &prompt,
            &model,
            session_id,
            Some(cwd.as_path()),
            &run_log,
            timeout,
        );
        let mut command = Command::new(antigravity_bin(&self.config));
        command.args(&args);
        command.current_dir(cwd);
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);
        command.envs(resolve_runtime_antigravity_env(
            &self.config,
            &options.metadata,
        ));

        let mut child = command.spawn().map_err(|error| {
            BridgeError::Internal(format!("failed to spawn antigravity CLI: {error}"))
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BridgeError::Internal("antigravity stdout unavailable".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BridgeError::Internal("antigravity stderr unavailable".to_owned()))?;
        let stdout_task = tokio::spawn(read_stream_to_string(stdout));
        let stderr_task = tokio::spawn(read_stream_to_string(stderr));
        let child = Arc::new(Mutex::new(child));
        self.register_run(run_id, &options.thread_id, child.clone())
            .await;

        let conversation_id = if let Some(session_id) = session_id {
            session_id.to_owned()
        } else {
            match self
                .discover_conversation_id(
                    &run_log,
                    &brain_root,
                    &conversations_dir,
                    run_start,
                    &prompt,
                    &options.message,
                )
                .await
            {
                Some(id) => id,
                None => {
                    let (child, _) = self.unregister_run(run_id).await;
                    let (stdout_output, stderr_output) = self
                        .cleanup_run_io(child, stdout_task, stderr_task, true)
                        .await;
                    return Err(BridgeError::RunFailed(append_process_output(
                        "antigravity conversation id discovery timed out",
                        &stdout_output,
                        &stderr_output,
                    )));
                }
            }
        };
        drop(fresh_guard);
        self.session_map
            .lock()
            .await
            .insert(options.thread_id.clone(), conversation_id.clone());
        on_chunk(StreamEvent::SessionBound {
            sdk_session_id: conversation_id.clone(),
        });

        let transcript = transcript_path(&brain_root, &conversation_id);
        let started = Instant::now();
        let mut mapper = TranscriptMapper::new();
        let exit_status = loop {
            mapper.apply_rows(
                read_transcript_rows(&transcript),
                baseline_step_index,
                on_chunk,
            );
            let maybe_status = {
                let mut child = child.lock().await;
                child.try_wait().map_err(|error| {
                    BridgeError::RunFailed(format!("antigravity process wait failed: {error}"))
                })?
            };
            if let Some(status) = maybe_status {
                break status;
            }
            tokio::time::sleep(TRANSCRIPT_POLL_INTERVAL).await;
        };

        for _ in 0..3 {
            mapper.apply_rows(
                read_transcript_rows(&transcript),
                baseline_step_index,
                on_chunk,
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let duration_ms = started.elapsed().as_millis() as i64;
        let (child, _) = self.unregister_run(run_id).await;
        let (stdout_output, stderr_output) = self
            .cleanup_run_io(child, stdout_task, stderr_task, false)
            .await;

        let mut success = exit_status.success() && mapper.error.is_none();
        let mut error = mapper.error.clone();
        if !exit_status.success() {
            let message = append_process_output(
                format!("antigravity CLI exited with status {exit_status}"),
                &stdout_output,
                &stderr_output,
            );
            if mapper.session_messages.is_empty() && mapper.response.trim().is_empty() {
                return Err(BridgeError::RunFailed(message));
            }
            success = false;
            error.get_or_insert(message);
        }
        if !success && error.is_none() {
            error = Some("antigravity run failed".to_owned());
        }

        on_chunk(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id: run_id.to_owned(),
            thread_id: options.thread_id.clone(),
            response: mapper.response,
            session_messages: mapper.session_messages,
            sdk_session_id: Some(conversation_id),
            actual_model: Some(model),
            thread_title: None,
            success,
            error,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            duration_ms,
        })
    }
}

#[async_trait]
impl AgentLoopProvider for AntigravityCliProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::AntigravityCli
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    fn resolve_runtime_selection(&self, options: &ProviderRunOptions) -> ProviderRuntimeSelection {
        ProviderRuntimeSelection {
            model: Some(model_id(&self.effective_config(), &options.metadata)),
            model_reasoning_effort: None,
            model_service_tier: None,
        }
    }

    fn update_model_defaults(&self, defaults: &ProviderModelDefaults) {
        *self
            .model_defaults
            .write()
            .expect("antigravity model defaults lock poisoned") = defaults.clone();
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        if self.ready {
            return Ok(());
        }
        let output = Command::new(antigravity_bin(&self.config))
            .arg("models")
            .output()
            .await
            .map_err(|error| {
                BridgeError::Internal(format!("failed to invoke antigravity CLI: {error}"))
            })?;
        if !output.status.success() {
            return Err(BridgeError::ProviderNotReady);
        }
        self.ready = true;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        let run_ids = self
            .active_runs
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for run_id in run_ids {
            let _ = self.abort(&run_id).await;
        }
        self.session_map.lock().await.clear();
        self.ready = false;
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        if !self.ready {
            return Err(BridgeError::ProviderNotReady);
        }

        if metadata_bool(&options.metadata, SDK_SESSION_FORK_METADATA_KEY) {
            return Err(BridgeError::SessionError(
                "antigravity provider does not support sdk session fork".to_owned(),
            ));
        }

        let run_id = resolve_run_id(&options.metadata);
        let session_id = {
            let map = self.session_map.lock().await;
            map.get(&options.thread_id).cloned()
        }
        .or_else(|| {
            normalize_non_empty(
                options
                    .metadata
                    .get("sdk_session_id")
                    .and_then(Value::as_str),
            )
        });

        let run = self.run_once(options, &run_id, session_id.as_deref(), &on_chunk);
        let first_attempt = tokio::time::timeout(outer_timeout(&self.config), run).await;
        let mut result = match first_attempt {
            Ok(Ok(result)) => result,
            Ok(Err(error))
                if session_id.is_some() && is_invalid_session_error(&error.to_string()) =>
            {
                self.session_map.lock().await.remove(&options.thread_id);
                let retry = self.run_once(options, &run_id, None, &on_chunk);
                match tokio::time::timeout(outer_timeout(&self.config), retry).await {
                    Ok(result) => result?,
                    Err(_) => {
                        let _ = self.abort(&run_id).await;
                        return Err(BridgeError::Timeout);
                    }
                }
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                let _ = self.abort(&run_id).await;
                return Err(BridgeError::Timeout);
            }
        };

        if !result.success
            && let Some(error) = result.error.as_deref()
            && session_id.is_some()
            && is_invalid_session_error(error)
        {
            self.session_map.lock().await.remove(&options.thread_id);
            let retry = self.run_once(options, &run_id, None, &on_chunk);
            result = match tokio::time::timeout(outer_timeout(&self.config), retry).await {
                Ok(result) => result?,
                Err(_) => {
                    let _ = self.abort(&run_id).await;
                    return Err(BridgeError::Timeout);
                }
            };
        }

        Ok(result)
    }

    async fn abort(&self, run_id: &str) -> bool {
        let (child, _) = self.unregister_run(run_id).await;
        let Some(child) = child else {
            return false;
        };

        let mut child = child.lock().await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        true
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(self
            .session_map
            .lock()
            .await
            .get(thread_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn clear_session(&self, thread_id: &str) -> bool {
        let active_run_ids = {
            let run_session_map = self.run_session_map.lock().await;
            run_session_map
                .iter()
                .filter(|(_, mapped_thread_id)| mapped_thread_id.as_str() == thread_id)
                .map(|(run_id, _)| run_id.clone())
                .collect::<Vec<_>>()
        };
        for run_id in active_run_ids {
            let _ = self.abort(&run_id).await;
        }
        self.session_map.lock().await.remove(thread_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc as StdArc, Mutex as StdMutex};

    #[test]
    fn command_args_use_current_antigravity_flags() {
        let log = PathBuf::from("/tmp/garyx-antigravity-test.log");
        let args = build_command_args(
            "hello",
            "Claude Opus 4.6 (Thinking)",
            Some("session-1"),
            Some(Path::new("/tmp/workspace")),
            &log,
            Duration::from_secs(12),
        );

        assert_eq!(args[0], "-p");
        assert!(args.contains(&"--model".to_owned()));
        assert!(args.contains(&"Claude Opus 4.6 (Thinking)".to_owned()));
        assert!(args.contains(&"--dangerously-skip-permissions".to_owned()));
        assert!(args.contains(&"--conversation".to_owned()));
        assert!(args.contains(&"session-1".to_owned()));
        assert!(args.contains(&"--add-dir".to_owned()));
        assert!(args.contains(&"--log-file".to_owned()));
        assert!(args.contains(&"--print-timeout".to_owned()));
        assert!(args.contains(&"12s".to_owned()));
    }

    #[test]
    fn transcript_mapping_emits_delta_tool_use_and_tool_result() {
        let rows = parse_jsonl_rows(
            r#"{"type":"USER_INPUT","step_index":1,"content":"hello"}
{"type":"PLANNER_RESPONSE","step_index":2,"created_at":"2026-01-01T00:00:00Z","thinking":"checking","tool_calls":[{"name":"RUN_COMMAND","args":"{\"command\":\"pwd\"}"}]}
{"type":"RUN_COMMAND","step_index":3,"created_at":"2026-01-01T00:00:01Z","content":"stdout"}
{"type":"PLANNER_RESPONSE","step_index":4,"created_at":"2026-01-01T00:00:02Z","content":"done"}
"#,
        );
        let events = StdArc::new(StdMutex::new(Vec::new()));
        let events_for_callback = StdArc::clone(&events);
        let callback: StreamCallback = Box::new(move |event| {
            events_for_callback.lock().unwrap().push(event);
        });
        let mut mapper = TranscriptMapper::new();
        mapper.apply_rows(rows, 1, &callback);

        assert_eq!(mapper.response, "done");
        let events = events.lock().unwrap();
        assert!(matches!(events[0], StreamEvent::ToolUse { .. }));
        assert!(matches!(events[1], StreamEvent::ToolResult { .. }));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamEvent::Delta { .. }))
                .count(),
            1
        );
        let assistant = mapper
            .session_messages
            .iter()
            .find(|message| message.role == ProviderMessageRole::Assistant)
            .expect("assistant message");
        assert_eq!(
            assistant
                .metadata
                .get("provider_reasoning")
                .and_then(Value::as_str),
            Some("checking")
        );
    }

    #[test]
    fn transcript_full_row_replaces_real_truncated_fields_schema() {
        let temp = tempfile::tempdir().expect("tempdir");
        let logs = temp.path().join("logs");
        fs::create_dir_all(&logs).expect("logs");
        let compact = logs.join("transcript.jsonl");
        let full = logs.join("transcript_full.jsonl");
        let compact_content = "x".repeat(4_096);
        let full_content = format!("{compact_content}{}", "y".repeat(512));
        fs::write(
            &compact,
            serde_json::to_string(&json!({
                "type": "PLANNER_RESPONSE",
                "step_index": 1,
                "truncated_fields": ["content"],
                "content": compact_content,
            }))
            .expect("compact json")
                + "\n",
        )
        .expect("write compact");
        fs::write(
            &full,
            serde_json::to_string(&json!({
                "type": "PLANNER_RESPONSE",
                "step_index": 1,
                "content": full_content,
            }))
            .expect("full json")
                + "\n",
        )
        .expect("write full");

        let rows = read_transcript_rows(&compact);
        let events = StdArc::new(StdMutex::new(Vec::new()));
        let events_for_callback = StdArc::clone(&events);
        let callback: StreamCallback = Box::new(move |event| {
            events_for_callback.lock().unwrap().push(event);
        });
        let mut mapper = TranscriptMapper::new();
        mapper.apply_rows(rows, -1, &callback);

        assert_eq!(mapper.response, full_content);
        assert_eq!(mapper.response.len(), 4_608);
    }

    #[test]
    fn list_directory_result_pairs_with_list_dir_tool_call() {
        let rows = parse_jsonl_rows(
            r#"{"type":"PLANNER_RESPONSE","step_index":1,"created_at":"2026-01-01T00:00:00Z","tool_calls":[{"name":"list_dir","args":{"path":"."}}]}
{"type":"LIST_DIRECTORY","step_index":2,"created_at":"2026-01-01T00:00:01Z","content":"listing"}
"#,
        );
        let callback: StreamCallback = Box::new(|_| {});
        let mut mapper = TranscriptMapper::new();
        mapper.apply_rows(rows, 0, &callback);

        let tool_use_id = mapper
            .session_messages
            .iter()
            .find(|message| message.role == ProviderMessageRole::ToolUse)
            .and_then(|message| message.tool_use_id.as_deref())
            .expect("tool use id");
        let tool_result_id = mapper
            .session_messages
            .iter()
            .find(|message| message.role == ProviderMessageRole::ToolResult)
            .and_then(|message| message.tool_use_id.as_deref())
            .expect("tool result id");

        assert_eq!(tool_result_id, tool_use_id);
    }

    #[test]
    fn discovery_uses_prompt_match_when_multiple_candidates_exist() {
        let temp = tempfile::tempdir().expect("tempdir");
        let base = temp.path().join(".gemini").join("antigravity-cli");
        let brain = base.join("brain");
        let conversations = base.join("conversations");
        fs::create_dir_all(&conversations).expect("conversations");
        for (id, prompt) in [
            ("wrong-session", "other prompt"),
            ("right-session", "target prompt"),
        ] {
            fs::write(conversations.join(format!("{id}.db")), "").expect("db");
            let logs = brain.join(id).join(".system_generated").join("logs");
            fs::create_dir_all(&logs).expect("logs");
            fs::write(
                logs.join("transcript.jsonl"),
                format!(r#"{{"type":"USER_INPUT","step_index":1,"content":"{prompt}"}}"#),
            )
            .expect("transcript");
        }

        let discovered = discover_from_conversations(
            &conversations,
            &brain,
            SystemTime::now()
                .checked_sub(Duration::from_secs(1))
                .expect("time"),
            "target prompt",
            "target prompt",
        );

        assert_eq!(discovered.as_deref(), Some("right-session"));
    }

    #[tokio::test]
    async fn run_streaming_tails_fake_antigravity_transcript() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_dir = temp.path().join("workspace");
        fs::create_dir_all(&workspace_dir).expect("workspace");
        let brain_root = temp
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain");
        let conversations_dir = temp
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("conversations");
        fs::create_dir_all(&brain_root).expect("brain");
        fs::create_dir_all(&conversations_dir).expect("conversations");
        let script_path = temp.path().join("fake-agy.py");
        let script = r#"#!/usr/bin/env python3
import json
import os
import sys
import time

if len(sys.argv) > 1 and sys.argv[1] == "models":
    print("Claude Opus 4.6 (Thinking)")
    sys.exit(0)

brain = os.environ["FAKE_AGY_BRAIN_ROOT"]
conv = None
prompt = ""
for index, arg in enumerate(sys.argv):
    if arg == "--conversation":
        conv = sys.argv[index + 1]
    if arg == "-p":
        prompt = sys.argv[index + 1]
if not conv:
    conv = "fake-session"

base = os.path.dirname(brain)
os.makedirs(os.path.join(base, "conversations"), exist_ok=True)
open(os.path.join(base, "conversations", conv + ".db"), "a").close()
logs = os.path.join(brain, conv, ".system_generated", "logs")
os.makedirs(logs, exist_ok=True)
path = os.path.join(logs, "transcript.jsonl")
mode = "a" if os.path.exists(path) else "w"
with open(path, mode) as f:
    f.write(json.dumps({"type":"USER_INPUT","step_index":1 if mode == "w" else 4,"content":prompt}) + "\n")
    f.flush()
    time.sleep(0.1)
    f.write(json.dumps({"type":"PLANNER_RESPONSE","step_index":2 if mode == "w" else 5,"content":"hello from agy"}) + "\n")
    f.flush()
sys.exit(0)
"#;
        fs::write(&script_path, script).expect("script");
        let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("chmod");

        let mut provider = AntigravityCliProvider::new(AntigravityCliConfig {
            antigravity_bin: script_path.to_string_lossy().to_string(),
            antigravity_brain_root: brain_root.to_string_lossy().to_string(),
            workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
            timeout_seconds: 5.0,
            env: HashMap::from([(
                "FAKE_AGY_BRAIN_ROOT".to_owned(),
                brain_root.to_string_lossy().to_string(),
            )]),
            ..Default::default()
        });
        provider.initialize().await.expect("initialize");

        let events = StdArc::new(StdMutex::new(Vec::new()));
        let events_for_callback = StdArc::clone(&events);
        let callback: StreamCallback = Box::new(move |event| {
            events_for_callback.lock().unwrap().push(event);
        });
        let result = provider
            .run_streaming(
                &ProviderRunOptions {
                    thread_id: "thread::antigravity::fake".to_owned(),
                    message: "say hi".to_owned(),
                    workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
                    images: None,
                    metadata: HashMap::new(),
                },
                callback,
            )
            .await
            .expect("run");

        assert!(result.success, "unexpected error: {:?}", result.error);
        assert_eq!(result.sdk_session_id.as_deref(), Some("fake-session"));
        assert_eq!(result.response, "hello from agy");
        let events = events.lock().unwrap();
        assert!(matches!(
            events.first(),
            Some(StreamEvent::SessionBound { sdk_session_id }) if sdk_session_id == "fake-session"
        ));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, StreamEvent::Done))
        );
    }
}
