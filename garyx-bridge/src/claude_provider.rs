use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use claude_agent_sdk::{
    AssistantMessage, AssistantMessageError, ClaudeAgentDefinition, ClaudeAgentOptions, ClaudeRun,
    ClaudeRunControl, ClaudeSDKError, ContentBlock, McpServerConfig, Message, OutboundUserMessage,
    PermissionMode, SystemMessage, TextBlock, UserInput, run_streaming as sdk_run_streaming,
};
use garyx_models::{
    is_builtin_provider_agent_id,
    provider::{
        ClaudeCodeConfig, ImagePayload, PromptAttachment, ProviderMessage, ProviderRateLimit,
        ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput,
        SDK_SESSION_FORK_METADATA_KEY, StreamBoundaryKind, StreamEvent, attachments_from_metadata,
        build_prompt_message_with_attachments, default_claude_cli_mode,
    },
};
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::gary_prompt::{
    compose_gary_instructions, prepend_initial_context_to_user_message, task_cli_env,
};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_trait::{
    AgentLoopProvider, BridgeError, ProviderModelDefaults, ProviderRuntimeSelection, StreamCallback,
};

// ---------------------------------------------------------------------------
// Retry configuration
// ---------------------------------------------------------------------------

#[cfg(test)]
const MAX_SESSION_FAILURES: u32 = 3;
const MAX_BUFFER_SIZE: usize = 10 * 1024 * 1024; // 10 MB
const ABORT_TIMEOUT_SECS: u64 = 10;
const STREAM_IDLE_TIMEOUT_SECS: u64 = 3600; // 1 hour
const POST_RESULT_DRAIN_TIMEOUT_SECS: u64 = 2;
const CLAUDE_MISSING_RESULT_ERROR: &str = "claude SDK stream ended without a result message";
const GARYX_CCTTY_PATH_ENV: &str = "GARYX_CCTTY_PATH";
const GARYX_CLAUDE_CLI_PATH_ENV: &str = "GARYX_CLAUDE_CLI_PATH";
const GARYX_CLAUDE_CLI_MODE_ENV: &str = "GARYX_CLAUDE_CLI_MODE";
const CLAUDE_CLI_MODE_CCTTY: &str = "cctty";
const CLAUDE_CLI_MODE_NATIVE: &str = "native";
const EMBEDDED_CCTTY_ARG: &str = "__cctty";

fn coerce_usage_i64(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_str()
            .and_then(|text| text.trim().parse::<f64>().ok())
            .map(|number| number.round() as i64)
    })
}

fn usage_value(usage: &HashMap<String, Value>, keys: &[&str]) -> i64 {
    keys.iter()
        .find_map(|key| usage.get(*key).filter(|value| !value.is_null()))
        .and_then(coerce_usage_i64)
        .unwrap_or(0)
}

fn result_usage_tokens(usage: Option<&HashMap<String, Value>>) -> (i64, i64) {
    let Some(usage) = usage else {
        return (0, 0);
    };

    let direct_input = usage_value(
        usage,
        &["inputTokens", "input", "promptTokens", "prompt_tokens"],
    );
    let input_tokens = if direct_input > 0 {
        direct_input
    } else {
        usage_value(usage, &["input_tokens"])
            + usage_value(
                usage,
                &["cacheCreationInputTokens", "cache_creation_input_tokens"],
            )
            + usage_value(usage, &["cacheReadInputTokens", "cache_read_input_tokens"])
    };

    let output_tokens = usage_value(
        usage,
        &[
            "outputTokens",
            "output",
            "completionTokens",
            "completion_tokens",
            "output_tokens",
        ],
    );

    (input_tokens.max(0), output_tokens.max(0))
}

fn claude_background_task_key(data: &Value) -> Option<String> {
    ["task_id", "taskId", "tool_use_id", "toolUseId"]
        .iter()
        .find_map(|key| {
            data.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn claude_background_task_status(data: &Value) -> Option<&str> {
    data.get("status")
        .and_then(Value::as_str)
        .or_else(|| {
            data.get("patch")
                .and_then(|patch| patch.get("status"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_terminal_claude_background_task_status(status: &str) -> bool {
    matches!(
        status,
        "completed"
            | "failed"
            | "cancelled"
            | "canceled"
            | "errored"
            | "error"
            | "interrupted"
            | "aborted"
            | "killed"
            | "stopped"
    )
}

fn update_claude_background_tasks(
    sys_msg: &SystemMessage,
    active_background_tasks: &mut HashSet<String>,
) {
    // `background_tasks_changed` carries the FULL set of live background tasks
    // with REPLACE semantics: swap the tracked set for the payload. This keeps
    // the post-result drain gate structurally current even when an individual
    // terminal `task_updated`/`task_notification` signal was missed.
    if sys_msg.subtype == "background_tasks_changed" {
        if let Some(tasks) = sys_msg.data.get("tasks").and_then(Value::as_array) {
            active_background_tasks.clear();
            for task in tasks {
                if let Some(task_id) = task
                    .get("task_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    active_background_tasks.insert(task_id.to_owned());
                }
            }
        }
        return;
    }

    let Some(task_key) = claude_background_task_key(&sys_msg.data) else {
        return;
    };

    match sys_msg.subtype.as_str() {
        "task_started" => {
            active_background_tasks.insert(task_key);
        }
        "task_updated" | "task_notification" => {
            if claude_background_task_status(&sys_msg.data)
                .map(is_terminal_claude_background_task_status)
                .unwrap_or(false)
            {
                active_background_tasks.remove(&task_key);
            } else {
                active_background_tasks.insert(task_key);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Run terminal-state and quota signals (Claude Agent SDK protocol)
// ---------------------------------------------------------------------------

/// Signals observed on the stream that matter for run-state classification
/// but arrive outside the formal `ResultMessage`.
#[derive(Debug, Default)]
struct StreamSignals {
    /// Latest `rate_limit_event` payload (`rate_limit_info` object).
    rate_limit_info: Option<Value>,
    /// Last assistant-level API error classification seen on the stream.
    last_assistant_error: Option<AssistantMessageError>,
}

fn unix_to_rfc3339(secs: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).map(|dt| dt.to_rfc3339())
}

/// Build a `ProviderRateLimit` from Claude's structured quota signals. Returns
/// `None` unless the run actually terminated on the subscription quota: the
/// result's `terminal_reason == "blocking_limit"`, or a `rate_limit_event`
/// with `status == "rejected"` was observed. A mere `allowed_warning` is NOT
/// enough — warning-level utilization must never trigger an automatic resend.
fn build_claude_rate_limit(
    provider_slug: &str,
    terminal_reason: Option<&str>,
    rate_limit_info: Option<&Value>,
    message: Option<&str>,
) -> Option<ProviderRateLimit> {
    let blocking_result = terminal_reason == Some("blocking_limit");
    let info = rate_limit_info.and_then(Value::as_object);
    let rejected = info
        .and_then(|object| object.get("status"))
        .and_then(Value::as_str)
        .map(|status| status.eq_ignore_ascii_case("rejected"))
        .unwrap_or(false);
    if !blocking_result && !rejected {
        return None;
    }

    Some(ProviderRateLimit {
        provider: provider_slug.to_owned(),
        reset_at: info
            .and_then(|object| object.get("resetsAt"))
            .and_then(Value::as_i64)
            .and_then(unix_to_rfc3339),
        window: info
            .and_then(|object| object.get("rateLimitType"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        used_percent: info
            .and_then(|object| object.get("utilization"))
            .and_then(Value::as_i64),
        reached_type: if blocking_result {
            terminal_reason.map(ToOwned::to_owned)
        } else {
            Some("rate_limit_rejected".to_owned())
        },
        message: message
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    })
}

/// Compose a structured run error message from the result frame's terminal
/// classification plus the last assistant-level API error, replacing the old
/// opaque "claude SDK reported error".
fn format_claude_run_error(
    result: &ProcessedResult,
    last_assistant_error: Option<&AssistantMessageError>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !result.subtype.is_empty() && result.subtype != "success" {
        parts.push(result.subtype.clone());
    }
    if let Some(terminal_reason) = result
        .terminal_reason
        .as_deref()
        .filter(|reason| !reason.is_empty())
    {
        parts.push(format!("terminal_reason={terminal_reason}"));
    }
    if let Some(stop_reason) = result
        .stop_reason
        .as_deref()
        .filter(|reason| !reason.is_empty())
    {
        parts.push(format!("stop_reason={stop_reason}"));
    }
    if let Some(status) = result.api_error_status {
        parts.push(format!("api_error_status={status}"));
    }
    if let Some(api_error) = last_assistant_error {
        parts.push(format!("api_error={}", api_error.as_label()));
    }
    let detail = result.errors.join("; ");

    match (parts.is_empty(), detail.is_empty()) {
        (false, false) => format!("claude run failed ({}): {detail}", parts.join(", ")),
        (false, true) => format!("claude run failed ({})", parts.join(", ")),
        (true, false) => format!("claude run failed: {detail}"),
        (true, true) => "claude SDK reported error".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Error classification helpers
// ---------------------------------------------------------------------------

fn executable_file_exists(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn explicit_claude_cli_path(config: &ClaudeCodeConfig) -> Option<PathBuf> {
    config
        .claude_cli_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            config
                .env
                .get(GARYX_CLAUDE_CLI_PATH_ENV)
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            std::env::var_os(GARYX_CLAUDE_CLI_PATH_ENV)
                .or_else(|| std::env::var_os(GARYX_CCTTY_PATH_ENV))
                .and_then(|value| {
                    let path = PathBuf::from(value);
                    (!path.as_os_str().is_empty()).then_some(path)
                })
        })
}

fn claude_sdk_cli_mode(config: &ClaudeCodeConfig) -> String {
    let env_mode = config
        .env
        .get(GARYX_CLAUDE_CLI_MODE_ENV)
        .cloned()
        .or_else(|| std::env::var(GARYX_CLAUDE_CLI_MODE_ENV).ok());
    let raw_mode = env_mode
        .as_deref()
        .unwrap_or(config.claude_cli_mode.as_str())
        .trim();
    let mode = if raw_mode.is_empty() {
        default_claude_cli_mode()
    } else {
        raw_mode.to_owned()
    };
    mode.to_ascii_lowercase()
}

fn uses_embedded_cctty(config: &ClaudeCodeConfig) -> bool {
    claude_sdk_cli_mode(config) != CLAUDE_CLI_MODE_NATIVE
        && explicit_claude_cli_path(config).is_none()
}

fn resolve_claude_sdk_cli_path(config: &ClaudeCodeConfig) -> Option<PathBuf> {
    if let Some(path) = explicit_claude_cli_path(config) {
        return Some(path);
    }

    match claude_sdk_cli_mode(config).as_str() {
        CLAUDE_CLI_MODE_NATIVE => None,
        _ => std::env::current_exe().ok(),
    }
}

fn resolve_claude_sdk_cli_prefix_args(config: &ClaudeCodeConfig) -> Vec<String> {
    uses_embedded_cctty(config)
        .then(|| EMBEDDED_CCTTY_ARG.to_owned())
        .into_iter()
        .collect()
}

fn claude_sdk_cli_mode_label(config: &ClaudeCodeConfig) -> &'static str {
    match claude_sdk_cli_mode(config).as_str() {
        CLAUDE_CLI_MODE_NATIVE => CLAUDE_CLI_MODE_NATIVE,
        _ => CLAUDE_CLI_MODE_CCTTY,
    }
}

#[cfg(test)]
fn is_retryable_error(msg: &str) -> bool {
    let patterns = [
        "overloaded",
        "rate_limit",
        "529",
        "503",
        "502",
        "timeout",
        "econnreset",
        "econnrefused",
        "connection refused",
        "connection reset",
    ];
    let lower = msg.to_lowercase();
    patterns.iter().any(|p| lower.contains(p))
}

fn is_session_corrupted_error(msg: &str) -> bool {
    let patterns = [
        "session not found",
        "invalid session",
        "corrupted",
        "conversation not found",
    ];
    let lower = msg.to_lowercase();
    patterns.iter().any(|p| lower.contains(p))
}

fn should_retry_message_with_fresh_session(msg: &str) -> bool {
    let patterns = [
        "failed to connect to claude",
        "control protocol error",
        "cli process exited before responding",
        "no result from claude sdk",
    ];
    let lower = msg.to_lowercase();
    is_session_corrupted_error(msg) || patterns.iter().any(|pattern| lower.contains(pattern))
}

fn should_retry_with_fresh_session(error: &BridgeError) -> bool {
    match error {
        BridgeError::SessionParseUnsupportedBlock(_) => false,
        other => should_retry_message_with_fresh_session(&other.to_string()),
    }
}

/// A resumed session left dangling mid tool-loop (e.g. the previous run was
/// killed by a gateway restart) makes the CLI close the turn with a synthetic
/// "No response requested." without ever invoking the model. Such a run
/// reports success with no output and would otherwise wedge the thread
/// forever, so it must retry on a fresh session.
fn resumed_run_stalled_without_response(outcome: &SdkRunOutcome) -> bool {
    if outcome.is_error || outcome.output_tokens > 0 {
        return false;
    }
    let text = outcome.response_text.trim();
    text.is_empty() || text == "No response requested."
}

fn bridge_error_from_sdk_stream_error(error: ClaudeSDKError) -> BridgeError {
    match error {
        ClaudeSDKError::MessageParse { message, data } => {
            if message.starts_with("Unknown content block type:") {
                let block_type = data
                    .as_ref()
                    .and_then(|value| value.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                BridgeError::SessionParseUnsupportedBlock(format!("{message} ({block_type})"))
            } else {
                BridgeError::RunFailed(format!("claude SDK message parse error: {message}"))
            }
        }
        other => BridgeError::RunFailed(format!("claude SDK stream error: {other}")),
    }
}

fn non_empty_session_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn metadata_bool(metadata: &HashMap<String, Value>, key: &str) -> bool {
    metadata.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn custom_standalone_agent_id(metadata: &HashMap<String, Value>) -> Option<&str> {
    if metadata.get("agent_team_id").is_some() {
        return None;
    }
    metadata
        .get("agent_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| !is_builtin_provider_agent_id(value))
}

// ---------------------------------------------------------------------------
// Content extraction helpers
// ---------------------------------------------------------------------------

fn emit_tool_stream_event(entry: &ProviderMessage, on_chunk: &StreamCallback) {
    match entry.role_str() {
        "tool_use" => on_chunk(StreamEvent::ToolUse {
            message: entry.clone(),
        }),
        "tool_result" => on_chunk(StreamEvent::ToolResult {
            message: entry.clone(),
        }),
        _ => {}
    }
}

fn resolve_run_id(metadata: &HashMap<String, serde_json::Value>) -> String {
    metadata
        .get("bridge_run_id")
        .and_then(|v| v.as_str())
        .or_else(|| metadata.get("client_run_id").and_then(|v| v.as_str()))
        .or_else(|| metadata.get("run_id").and_then(|v| v.as_str()))
        .map(String::from)
        .unwrap_or_else(|| format!("run_{}", Uuid::new_v4()))
}

fn resolve_requested_model(
    config: &ClaudeCodeConfig,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    metadata
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| (!config.default_model.trim().is_empty()).then(|| config.default_model.clone()))
}

/// Reasoning levels accepted by the Claude Code CLI `--effort` flag. Unknown
/// values are dropped so a stale or cross-provider level cannot break spawn.
fn claude_effort_for_reasoning_effort(effort: &str) -> Option<String> {
    let effort = effort.trim().to_ascii_lowercase();
    matches!(effort.as_str(), "low" | "medium" | "high" | "xhigh" | "max").then_some(effort)
}

fn resolve_requested_effort(
    config: &ClaudeCodeConfig,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    metadata
        .get("model_reasoning_effort")
        .and_then(|v| v.as_str())
        .and_then(claude_effort_for_reasoning_effort)
        .or_else(|| claude_effort_for_reasoning_effort(&config.model_reasoning_effort))
}

fn normalize_thread_title(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.chars().count() <= 80 {
        return trimmed.to_owned();
    }
    let mut clipped = trimmed.chars().take(79).collect::<String>();
    clipped.push('…');
    clipped
}

fn extract_claude_thread_title(data: &Value) -> Option<String> {
    [
        "session_name",
        "sessionName",
        "thread_title",
        "threadTitle",
        "title",
    ]
    .iter()
    .find_map(|key| data.get(*key).and_then(Value::as_str))
    .map(normalize_thread_title)
    .filter(|value| !value.is_empty())
}

fn extract_claude_ai_title_line(line: &str, session_id: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("ai-title") {
        return None;
    }
    if value
        .get("sessionId")
        .and_then(Value::as_str)
        .is_some_and(|observed| observed != session_id)
    {
        return None;
    }
    value
        .get("aiTitle")
        .and_then(Value::as_str)
        .map(normalize_thread_title)
        .filter(|value| !value.is_empty())
}

fn claude_config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
}

fn claude_project_dir_name(cwd: &Path) -> String {
    let text = cwd.to_string_lossy();
    let mapped = text
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    if mapped.is_empty() {
        "-".to_owned()
    } else {
        mapped
    }
}

fn claude_transcript_path(config_dir: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    config_dir
        .join("projects")
        .join(claude_project_dir_name(cwd))
        .join(format!("{session_id}.jsonl"))
}

fn resolve_claude_cwd(config: &ClaudeCodeConfig, options: &ProviderRunOptions) -> Option<PathBuf> {
    options
        .workspace_dir
        .as_ref()
        .or(config.workspace_dir.as_ref())
        .map(|ws| PathBuf::from(shellexpand::tilde(ws).as_ref()))
        .filter(|path| path.exists())
}

async fn read_claude_ai_title_from_transcript_path(
    transcript_path: &Path,
    session_id: &str,
) -> Option<String> {
    let content = tokio::fs::read_to_string(transcript_path).await.ok()?;
    content
        .lines()
        .rev()
        .find_map(|line| extract_claude_ai_title_line(line, session_id))
}

async fn read_claude_ai_title_from_transcript(cwd: &Path, session_id: &str) -> Option<String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let transcript_path = claude_transcript_path(&claude_config_dir()?, &cwd, session_id);
    read_claude_ai_title_from_transcript_path(&transcript_path, session_id).await
}

async fn count_claude_transcript_history_messages_at_path(transcript_path: &Path) -> Option<usize> {
    let content = tokio::fs::read_to_string(transcript_path).await.ok()?;
    Some(
        content
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|value| {
                value
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| matches!(kind, "user" | "assistant"))
            })
            .count(),
    )
}

async fn count_claude_transcript_history_messages(
    config: &ClaudeCodeConfig,
    options: &ProviderRunOptions,
    session_id: Option<&str>,
) -> Option<usize> {
    let session_id = session_id?.trim();
    if session_id.is_empty() {
        return None;
    }
    let cwd = resolve_claude_cwd(config, options)?;
    let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    let transcript_path = claude_transcript_path(&claude_config_dir()?, &cwd, session_id);
    count_claude_transcript_history_messages_at_path(&transcript_path).await
}

fn push_assistant_text_message(
    text: &str,
    response_text: &mut String,
    session_messages: &mut Vec<ProviderMessage>,
    on_chunk: &StreamCallback,
) {
    if text.is_empty() {
        return;
    }

    let entry = ProviderMessage::assistant_text(text)
        .with_timestamp(chrono::Utc::now().to_rfc3339())
        .with_metadata_value("source", serde_json::json!("claude_sdk"));
    session_messages.push(entry);
    response_text.push_str(text);
    on_chunk(StreamEvent::Delta {
        text: text.to_owned(),
    });
}

fn has_parent_tool_use_id(parent_tool_use_id: Option<&str>) -> bool {
    parent_tool_use_id
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

fn assistant_blocks_have_visible_text(blocks: &[ContentBlock]) -> bool {
    blocks.iter().any(|block| match block {
        ContentBlock::Text(TextBlock { text }) => !text.is_empty(),
        _ => false,
    })
}

fn is_synthetic_no_response_message(message: &AssistantMessage) -> bool {
    if message.model.trim() != "<synthetic>" {
        return false;
    }

    let mut text = String::new();
    for block in &message.content {
        match block {
            ContentBlock::Text(TextBlock { text: block_text }) => text.push_str(block_text),
            ContentBlock::Thinking(_) => {}
            _ => return false,
        }
    }

    text.trim() == "No response requested."
}

fn assistant_blocks_start_with_newline(blocks: &[ContentBlock]) -> bool {
    blocks.iter().find_map(|block| match block {
        ContentBlock::Text(TextBlock { text }) if !text.is_empty() => Some(text.starts_with('\n')),
        _ => None,
    }) == Some(true)
}

/// What `process_assistant_blocks_streaming` leaves as the trailing streamed
/// event for this message: `Some(true)` when it ends with visible text (the
/// assistant tail stays in flight downstream), `Some(false)` when it ends with
/// a tool event (the tail is finalized), `None` when nothing is emitted.
/// Mirrors that function's flush rules: text accumulates and flushes before a
/// tool block or at the end, and text is suppressed for subagent messages
/// (`parent_tool_use_id`).
fn assistant_blocks_trailing_text_emission(
    blocks: &[ContentBlock],
    parent_tool_use_id: Option<&str>,
) -> Option<bool> {
    let suppress_text = has_parent_tool_use_id(parent_tool_use_id);
    let mut trailing = None;
    for block in blocks {
        match block {
            ContentBlock::Text(TextBlock { text }) => {
                if !suppress_text && !text.is_empty() {
                    trailing = Some(true);
                }
            }
            ContentBlock::ToolUse(_) | ContentBlock::ToolResult(_) => {
                trailing = Some(false);
            }
            ContentBlock::Image(_)
            | ContentBlock::Document(_)
            | ContentBlock::Thinking(_)
            | ContentBlock::Unknown(_) => {}
        }
    }
    trailing
}

fn append_assistant_segment_separator(response_text: &mut String) {
    if response_text.is_empty() || response_text.ends_with("\n\n") {
        return;
    }
    if response_text.ends_with('\n') {
        response_text.push('\n');
    } else {
        response_text.push_str("\n\n");
    }
}

fn process_assistant_blocks_streaming(
    blocks: &[ContentBlock],
    response_text: &mut String,
    session_messages: &mut Vec<ProviderMessage>,
    on_chunk: &StreamCallback,
    parent_tool_use_id: Option<&str>,
) {
    let mut pending_text = String::new();
    let suppress_text = has_parent_tool_use_id(parent_tool_use_id);

    let flush_text = |pending_text: &mut String,
                      response_text: &mut String,
                      session_messages: &mut Vec<ProviderMessage>| {
        if pending_text.is_empty() {
            return;
        }
        let text = std::mem::take(pending_text);
        if suppress_text {
            return;
        }
        push_assistant_text_message(&text, response_text, session_messages, on_chunk);
    };

    for block in blocks {
        match block {
            ContentBlock::Text(TextBlock { text }) => {
                pending_text.push_str(text);
            }
            ContentBlock::ToolUse(_) | ContentBlock::ToolResult(_) => {
                flush_text(&mut pending_text, response_text, session_messages);
                extract_tool_session_messages(
                    std::slice::from_ref(block),
                    session_messages,
                    Some(on_chunk),
                    parent_tool_use_id,
                );
            }
            ContentBlock::Image(_)
            | ContentBlock::Document(_)
            | ContentBlock::Thinking(_)
            | ContentBlock::Unknown(_) => {}
        }
    }

    flush_text(&mut pending_text, response_text, session_messages);
}

/// Extract tool_use and tool_result blocks into session messages (Python parity).
fn extract_tool_session_messages(
    blocks: &[ContentBlock],
    session_messages: &mut Vec<ProviderMessage>,
    on_chunk: Option<&StreamCallback>,
    parent_tool_use_id: Option<&str>,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let parent_tool_use_id = parent_tool_use_id.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    for block in blocks {
        match block {
            ContentBlock::ToolUse(tu) => {
                let mut entry = ProviderMessage::tool_use(
                    serde_json::json!({
                        "tool": tu.name,
                        "input": tu.input,
                    }),
                    Some(tu.id.clone()),
                    Some(tu.name.clone()),
                )
                .with_timestamp(now.clone())
                .with_metadata_value("source", serde_json::json!("claude_sdk"));
                if let Some(parent_tool_use_id) = parent_tool_use_id {
                    entry = entry.with_metadata_value(
                        "parent_tool_use_id",
                        serde_json::json!(parent_tool_use_id),
                    );
                }
                if let Some(on_chunk) = on_chunk {
                    emit_tool_stream_event(&entry, on_chunk);
                }
                session_messages.push(entry);
            }
            ContentBlock::ToolResult(tr) => {
                let tool_text = tr
                    .content
                    .as_ref()
                    .and_then(|c| c.as_str().map(String::from))
                    .unwrap_or_default();
                let mut entry = ProviderMessage::tool_result(
                    serde_json::json!({
                        "result": tr.content,
                        "text": tool_text,
                    }),
                    Some(tr.tool_use_id.clone()),
                    None,
                    tr.is_error,
                )
                .with_timestamp(now.clone())
                .with_metadata_value("source", serde_json::json!("claude_sdk"));
                if let Some(parent_tool_use_id) = parent_tool_use_id {
                    entry = entry.with_metadata_value(
                        "parent_tool_use_id",
                        serde_json::json!(parent_tool_use_id),
                    );
                }
                entry.text = (!tool_text.is_empty()).then_some(tool_text);
                if let Some(on_chunk) = on_chunk {
                    emit_tool_stream_event(&entry, on_chunk);
                }
                session_messages.push(entry);
            }
            _ => {}
        }
    }
}

fn has_tool_result_blocks(blocks: &[ContentBlock]) -> bool {
    blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolResult(_)))
}

/// Name shared with the Codex provider's structured-activity mapping so
/// context compaction renders identically across providers.
const CONTEXT_COMPACTION_TOOL_NAME: &str = "contextCompaction";

/// Stable activity id for a synthesized system-activity frame: the system
/// message's own wire `uuid` when present, otherwise a fresh one.
fn claude_system_activity_id(data: &Value) -> String {
    data.get("uuid")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

/// Emit a paired `ToolUse`/`ToolResult` activity frame for a context
/// compaction. Compaction is completed-only on the Claude wire (a successful
/// compact surfaces as `compact_boundary`, a failed one as a `status` frame
/// with `compact_result: "failed"`), and a lone `ToolResult` frame is
/// invisible on channels that render tool activity from the `ToolUse` frame —
/// so both halves are synthesized together, mirroring the Codex
/// completed-only-item precedent. Stays inside the provider-neutral stream
/// contract.
fn emit_context_compaction_activity(
    session_messages: &mut Vec<ProviderMessage>,
    on_chunk: &StreamCallback,
    activity_id: &str,
    input: Value,
    result_content: Value,
    result_text: String,
    is_error: bool,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let tool_use = ProviderMessage::tool_use(
        serde_json::json!({
            "tool": CONTEXT_COMPACTION_TOOL_NAME,
            "input": input,
        }),
        Some(activity_id.to_owned()),
        Some(CONTEXT_COMPACTION_TOOL_NAME.to_owned()),
    )
    .with_timestamp(now.clone())
    .with_metadata_value("source", serde_json::json!("claude_sdk"));
    let mut tool_result = ProviderMessage::tool_result(
        serde_json::json!({
            "result": result_content,
            "text": result_text.clone(),
        }),
        Some(activity_id.to_owned()),
        Some(CONTEXT_COMPACTION_TOOL_NAME.to_owned()),
        Some(is_error),
    )
    .with_timestamp(now)
    .with_metadata_value("source", serde_json::json!("claude_sdk"));
    tool_result.text = (!result_text.is_empty()).then_some(result_text);

    for entry in [tool_use, tool_result] {
        emit_tool_stream_event(&entry, on_chunk);
        session_messages.push(entry);
    }
}

fn metadata_string_map(
    metadata: &HashMap<String, serde_json::Value>,
    key: &str,
) -> HashMap<String, String> {
    metadata
        .get(key)
        .and_then(|value| value.as_object())
        .map(|map| {
            map.iter()
                .filter_map(|(entry_key, entry_value)| {
                    entry_value
                        .as_str()
                        .map(|entry_value| (entry_key.clone(), entry_value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn metadata_mcp_servers(
    metadata: &HashMap<String, serde_json::Value>,
    key: &str,
) -> HashMap<String, McpServerConfig> {
    let Some(value) = metadata.get(key).and_then(serde_json::Value::as_object) else {
        return HashMap::new();
    };

    value
        .iter()
        .filter_map(|(name, raw_config)| {
            let config = if raw_config
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some()
            {
                serde_json::from_value::<McpServerConfig>(raw_config.clone())
                    .map_err(|e| tracing::warn!(server = %name, error = %e, "failed to parse MCP server config"))
                    .ok()
            } else {
                let command = raw_config.get("command")?.as_str()?.trim().to_owned();
                if command.is_empty() {
                    return None;
                }
                let enabled = raw_config
                    .get("enabled")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                if !enabled {
                    return None;
                }
                Some(McpServerConfig::Stdio {
                    command,
                    args: raw_config
                        .get("args")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                    env: raw_config
                        .get("env")
                        .and_then(serde_json::Value::as_object)
                        .map(|entries| {
                            entries
                                .iter()
                                .filter_map(|(env_key, env_value)| {
                                    env_value
                                        .as_str()
                                        .map(|env_value| (env_key.clone(), env_value.to_owned()))
                                })
                                .collect::<HashMap<_, _>>()
                        })
                        .unwrap_or_default(),
                })
            }?;
            Some((name.clone(), config))
        })
        .collect()
}

fn is_supported_image_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

/// Build typed user input payload for Claude streaming input.
///
/// If no valid image payloads are present, this returns a plain text content
/// string to preserve existing behavior. With images, it returns multimodal
/// blocks (`text` + `image`) mirroring Python's provider behavior.
fn build_user_message_input_from_parts(
    message: &str,
    image_payloads: &[ImagePayload],
    attachments: &[PromptAttachment],
) -> UserInput {
    let message = build_prompt_message_with_attachments(message, attachments);
    if !attachments.is_empty() {
        return UserInput::Text(message);
    }

    if image_payloads.is_empty() {
        return UserInput::Text(message);
    }

    let mut blocks = Vec::with_capacity(image_payloads.len() + 1);
    if !message.trim().is_empty() {
        blocks.push(serde_json::json!({
            "type": "text",
            "text": message.clone(),
        }));
    }

    for image in image_payloads {
        if image.data.trim().is_empty() || !is_supported_image_media_type(&image.media_type) {
            continue;
        }
        blocks.push(serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.media_type,
                "data": image.data,
            }
        }));
    }

    if blocks.is_empty() {
        UserInput::Text(message.to_owned())
    } else {
        UserInput::Blocks(blocks)
    }
}

fn build_user_message_input(options: &ProviderRunOptions, include_memory: bool) -> UserInput {
    let images = options.images.as_deref().unwrap_or_default();
    let attachments = attachments_from_metadata(&options.metadata);
    let message = build_native_skill_prompt(&options.message, &options.metadata)
        .unwrap_or_else(|| options.message.clone());
    let message =
        prepend_initial_context_to_user_message(&message, &options.metadata, include_memory);
    build_user_message_input_from_parts(&message, images, &attachments)
}

// ---------------------------------------------------------------------------
// ClaudeCliProvider
// ---------------------------------------------------------------------------

/// Agent provider using the Claude Agent SDK.
///
/// This is the Rust counterpart of the Python `ClaudeAgentProvider`.
/// It uses the `claude-agent-sdk` crate to communicate with the Claude CLI
/// via bidirectional JSONL, supporting streaming, session management,
/// retries, and abort.
pub struct ClaudeCliProvider {
    config: ClaudeCodeConfig,
    /// Hot-reloadable model defaults. Config reloads reconcile onto the live
    /// provider instance (the provider key excludes model defaults to keep
    /// thread affinity stable), so default-model resolution must read these
    /// instead of the frozen `config` fields.
    model_defaults: std::sync::RwLock<ProviderModelDefaults>,
    /// Maps Garyx thread IDs to Claude CLI session IDs.
    session_map: Mutex<HashMap<String, String>>,
    /// Tracks thread failure counts for auto-recovery.
    session_failure_counts: Mutex<HashMap<String, u32>>,
    /// Tracks active run controls by run_id for abort support.
    active_runs: Mutex<HashMap<String, ClaudeRunControl>>,
    /// Maps run_id to thread_id for reverse lookup during abort.
    run_session_map: Mutex<HashMap<String, String>>,
    /// User inputs accepted but not yet completed per live run.
    run_pending_inputs: Mutex<HashMap<String, VecDeque<PendingAckMarker>>>,
    /// Last user message per thread, used for auto-recovery replay.
    last_messages: Mutex<HashMap<String, String>>,
    /// Quota-exhaustion context staged per thread when a run terminates on the
    /// provider's rolling usage limit; consumed exactly once by the bridge
    /// run-completion path via `take_rate_limit`.
    pending_rate_limits: Mutex<HashMap<String, ProviderRateLimit>>,
    #[cfg(test)]
    test_run_attempts: Mutex<VecDeque<Result<Option<SdkRunOutcome>, BridgeError>>>,
    #[cfg(test)]
    test_recorded_session_attempts: Mutex<Vec<Option<String>>>,
    ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingAckMarker {
    RootUserMessage,
    QueuedInput(String),
}

impl ClaudeCliProvider {
    /// Create a new provider with the given config.
    pub fn new(config: ClaudeCodeConfig) -> Self {
        let model_defaults = std::sync::RwLock::new(ProviderModelDefaults {
            model: String::new(),
            default_model: config.default_model.clone(),
            model_reasoning_effort: config.model_reasoning_effort.clone(),
            model_service_tier: String::new(),
        });
        Self {
            config,
            model_defaults,
            session_map: Mutex::new(HashMap::new()),
            session_failure_counts: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            run_session_map: Mutex::new(HashMap::new()),
            run_pending_inputs: Mutex::new(HashMap::new()),
            last_messages: Mutex::new(HashMap::new()),
            pending_rate_limits: Mutex::new(HashMap::new()),
            #[cfg(test)]
            test_run_attempts: Mutex::new(VecDeque::new()),
            #[cfg(test)]
            test_recorded_session_attempts: Mutex::new(Vec::new()),
            ready: false,
        }
    }

    /// Clone the frozen config with the hot-reloadable model defaults
    /// overlaid, so run-request building and runtime selection observe the
    /// latest reloaded defaults.
    fn effective_config(&self) -> ClaudeCodeConfig {
        let defaults = self
            .model_defaults
            .read()
            .expect("claude model defaults lock poisoned")
            .clone();
        let mut config = self.config.clone();
        config.default_model = defaults.default_model;
        config.model_reasoning_effort = defaults.model_reasoning_effort;
        config
    }

    /// Build `ClaudeAgentOptions` from our config and run options.
    fn build_sdk_options(
        &self,
        options: &ProviderRunOptions,
        session_id: Option<&str>,
        run_id: &str,
    ) -> ClaudeAgentOptions {
        // Reserve `garyx` for the built-in control-plane MCP server so a
        // stale runtime override cannot shadow the local gateway endpoint.
        let mut mcp_servers = metadata_mcp_servers(&options.metadata, "remote_mcp_servers");
        if !self.config.mcp_base_url.is_empty() {
            tracing::info!(
                run_id = %run_id,
                thread_id = %options.thread_id,
                mcp_base_url = %self.config.mcp_base_url,
                "MCP headers: building garyx MCP config"
            );
            let mut mcp_headers = HashMap::new();
            mcp_headers.insert("X-Run-Id".to_string(), run_id.to_string());
            mcp_headers.insert("X-Thread-Id".to_string(), options.thread_id.clone());
            mcp_headers.insert("X-Session-Key".to_string(), options.thread_id.clone());
            mcp_headers.extend(metadata_string_map(&options.metadata, "garyx_mcp_headers"));

            // Encode thread_id and run_id into the URL path as a workaround
            // for Claude Code CLI stripping both custom headers and query
            // params from MCP tool call requests.
            // Format: /mcp/{thread_id}/{run_id}
            let encoded_thread = urlencoding::encode(&options.thread_id);
            let encoded_run = urlencoding::encode(run_id);
            let mcp_url = options
                .metadata
                .get("garyx_mcp_auth_token")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|token| {
                    format!(
                        "{}/mcp/auth/{}/{}/{}",
                        self.config.mcp_base_url,
                        urlencoding::encode(token),
                        encoded_thread,
                        encoded_run
                    )
                })
                .unwrap_or_else(|| {
                    format!(
                        "{}/mcp/{}/{}",
                        self.config.mcp_base_url, encoded_thread, encoded_run
                    )
                });
            mcp_servers.insert(
                "garyx".to_string(),
                McpServerConfig::Http {
                    url: mcp_url,
                    headers: mcp_headers,
                },
            );
        }

        // Workspace directory
        let cwd = resolve_claude_cwd(&self.config, options);

        // Model: metadata override > (hot-reloadable) config default
        let effective_config = self.effective_config();
        let model = resolve_requested_model(&effective_config, &options.metadata);
        // Thinking level: per-run metadata overrides the provider default and
        // is mapped to the Claude CLI `--effort` flag.
        let requested_effort = resolve_requested_effort(&effective_config, &options.metadata);

        let metadata_system_prompt = options
            .metadata
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let runtime_system_prompt = metadata_system_prompt.or(self.config.system_prompt.as_deref());

        let session_agent_id = custom_standalone_agent_id(&options.metadata);
        let session_agent_name = options
            .metadata
            .get("agent_display_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Garyx custom agent");

        let (agent, agents, system_prompt, append_system_prompt) =
            if let Some(agent_id) = session_agent_id {
                // Custom agent: use the agent's own system_prompt directly
                // as the --agents prompt. Task state is rendered into user
                // messages so system prompts stay stable for cache reuse.
                if let Some(agent_prompt) = metadata_system_prompt {
                    let description = format!("Garyx custom agent: {session_agent_name}");
                    (
                        Some(agent_id.to_owned()),
                        HashMap::from([(
                            agent_id.to_owned(),
                            ClaudeAgentDefinition {
                                description,
                                prompt: agent_prompt.to_owned(),
                            },
                        )]),
                        None,
                        None,
                    )
                } else {
                    (None, HashMap::new(), None, None)
                }
            } else {
                // Default Garyx: compose full instructions with --system-prompt
                let merged_instructions = compose_gary_instructions(
                    runtime_system_prompt,
                    cwd.as_deref(),
                    options
                        .metadata
                        .get("automation_id")
                        .and_then(|v| v.as_str()),
                );
                (None, HashMap::new(), Some(merged_instructions), None)
            };
        let mut env = self.config.env.clone();
        env.extend(task_cli_env(&options.metadata));
        env.extend(metadata_string_map(&options.metadata, "provider_env"));
        let cli_path = resolve_claude_sdk_cli_path(&self.config);
        let cli_prefix_args = resolve_claude_sdk_cli_prefix_args(&self.config);

        // Permission mode
        let permission_mode = match self.config.permission_mode.as_str() {
            "acceptEdits" => Some(PermissionMode::AcceptEdits),
            "plan" => Some(PermissionMode::Plan),
            "auto" => Some(PermissionMode::Auto),
            "dontAsk" => Some(PermissionMode::BypassPermissions),
            "bypassPermissions" => Some(PermissionMode::BypassPermissions),
            "default" => Some(PermissionMode::Default),
            _ => Some(PermissionMode::BypassPermissions),
        };

        // Extra args: replay-user-messages for UserMessage ACK
        let mut extra_args = HashMap::new();
        extra_args.insert("replay-user-messages".to_string(), None);
        if let Some(effort) = requested_effort {
            extra_args.insert("effort".to_string(), Some(effort));
        }
        let fork_session = metadata_bool(&options.metadata, SDK_SESSION_FORK_METADATA_KEY);

        ClaudeAgentOptions {
            agent,
            agents,
            system_prompt,
            append_system_prompt,
            mcp_servers,
            permission_mode,
            resume: session_id.map(String::from),
            max_turns: None,
            disallowed_tools: self.config.disallowed_tools.clone(),
            model,
            cwd,
            cli_path,
            cli_prefix_args,
            env,
            extra_args,
            max_buffer_size: Some(MAX_BUFFER_SIZE),
            setting_sources: (!self.config.setting_sources.is_empty())
                .then(|| self.config.setting_sources.clone()),
            fork_session,
            ..Default::default()
        }
    }

    /// Record a thread failure and return whether we should clear the provider session.
    #[cfg(test)]
    async fn record_failure(&self, thread_id: &str) -> bool {
        let mut counts = self.session_failure_counts.lock().await;
        let count = counts.entry(thread_id.to_owned()).or_insert(0);
        *count += 1;
        *count >= MAX_SESSION_FAILURES
    }

    /// Reset failure counter after a successful run.
    async fn reset_failure_count(&self, thread_id: &str) {
        self.session_failure_counts.lock().await.remove(thread_id);
    }

    /// Keep a stable thread->session mapping.
    ///
    /// A Claude session id should stay stable for a thread unless we
    /// explicitly clear it and start a fresh session.
    async fn stabilize_session_id(&self, thread_id: &str, observed_session_id: &str) -> String {
        let observed = observed_session_id.trim();
        if observed.is_empty() {
            return String::new();
        }

        let mut sessions = self.session_map.lock().await;
        if let Some(existing) = sessions.get(thread_id) {
            if existing != observed {
                tracing::warn!(
                    thread_id = %thread_id,
                    existing_session_id = %existing,
                    observed_session_id = observed,
                    "ignoring unexpected claude session_id change; keeping stable session binding"
                );
            }
            return existing.clone();
        }

        sessions.insert(thread_id.to_owned(), observed.to_owned());
        observed.to_owned()
    }

    /// Register a run control for abort tracking and map run_id to thread_id.
    async fn register_run(&self, run_id: &str, thread_id: &str, run: ClaudeRunControl) {
        self.active_runs.lock().await.insert(run_id.to_owned(), run);
        self.run_session_map
            .lock()
            .await
            .insert(run_id.to_owned(), thread_id.to_owned());
    }

    /// Unregister a run after completion and return handle for cleanup.
    async fn unregister_run(&self, run_id: &str) -> (Option<ClaudeRunControl>, Option<String>) {
        let run = self.active_runs.lock().await.remove(run_id);
        let thread_id = self.run_session_map.lock().await.remove(run_id);
        self.run_pending_inputs.lock().await.remove(run_id);
        (run, thread_id)
    }

    /// Best-effort run cleanup.
    async fn cleanup_run_handle(&self, run_id: &str, run: ClaudeRunControl) {
        if let Err(e) = run.close().await {
            tracing::warn!(run_id = %run_id, error = %e, "failed to close claude run");
        }
    }

    async fn initialize_pending_inputs(&self, run_id: &str) {
        self.run_pending_inputs.lock().await.insert(
            run_id.to_owned(),
            VecDeque::from([PendingAckMarker::RootUserMessage]),
        );
    }

    #[cfg(test)]
    async fn set_pending_inputs(&self, run_id: &str, count: usize) {
        let mut queue = VecDeque::with_capacity(count);
        for _ in 0..count {
            queue.push_back(PendingAckMarker::RootUserMessage);
        }
        self.run_pending_inputs
            .lock()
            .await
            .insert(run_id.to_owned(), queue);
    }

    async fn enqueue_pending_input(&self, run_id: &str, pending_input_id: String) -> bool {
        let mut pending = self.run_pending_inputs.lock().await;
        if let Some(queue) = pending.get_mut(run_id) {
            queue.push_back(PendingAckMarker::QueuedInput(pending_input_id));
            true
        } else {
            false
        }
    }

    async fn rollback_pending_input(&self, run_id: &str, pending_input_id: &str) {
        let mut pending = self.run_pending_inputs.lock().await;
        if let Some(queue) = pending.get_mut(run_id)
            && let Some(index) = queue.iter().position(|marker| {
                matches!(marker, PendingAckMarker::QueuedInput(candidate) if candidate == pending_input_id)
            }) {
                queue.remove(index);
            }
    }

    async fn acknowledge_next_pending_input(
        &self,
        run_id: &str,
        prefer_queued_input: bool,
    ) -> Option<String> {
        let mut pending = self.run_pending_inputs.lock().await;
        pending.get_mut(run_id).and_then(|queue| {
            if prefer_queued_input
                && matches!(queue.front(), Some(PendingAckMarker::RootUserMessage))
                && queue
                    .iter()
                    .any(|marker| matches!(marker, PendingAckMarker::QueuedInput(_)))
            {
                queue.pop_front();
            }
            match queue.pop_front() {
                Some(PendingAckMarker::QueuedInput(pending_input_id)) => Some(pending_input_id),
                Some(PendingAckMarker::RootUserMessage) | None => None,
            }
        })
    }

    /// Atomically check whether the pending-input queue is empty after the
    /// post-result drain window and, if so, **remove** the queue entry so that
    /// subsequent [`enqueue_pending_input`] calls for this `run_id` will fail.
    /// This closes the race window where a new input is enqueued between the
    /// emptiness check and the loop break in [`process_messages_streaming`].
    ///
    /// Returns `true` when the queue was empty (or already absent) and the run
    /// should exit its message loop.
    async fn try_close_pending_inputs(&self, run_id: &str) -> bool {
        let mut pending = self.run_pending_inputs.lock().await;
        match pending.get(run_id) {
            Some(queue) => {
                let has_queued = queue
                    .iter()
                    .any(|marker| matches!(marker, PendingAckMarker::QueuedInput(_)));
                if has_queued {
                    false
                } else {
                    // Empty — remove the entry so enqueue_pending_input cannot
                    // succeed for this run any more.
                    pending.remove(run_id);
                    true
                }
            }
            None => true, // Already cleaned up.
        }
    }

    async fn resolve_active_run_id_for_session(&self, thread_id: &str) -> Option<String> {
        let candidate_run_ids: Vec<String> = {
            let map = self.run_session_map.lock().await;
            map.iter()
                .filter(|(_, mapped_thread_id)| mapped_thread_id.as_str() == thread_id)
                .map(|(run_id, _)| run_id.clone())
                .collect()
        };

        if candidate_run_ids.is_empty() {
            return None;
        }

        let (active_run_ids, stale_run_ids): (Vec<_>, Vec<_>) = {
            let active_runs = self.active_runs.lock().await;
            candidate_run_ids
                .into_iter()
                .partition(|run_id| active_runs.contains_key(run_id))
        };

        for stale_run_id in stale_run_ids {
            let _ = self.unregister_run(&stale_run_id).await;
        }

        if active_run_ids.len() > 1 {
            tracing::warn!(
                thread_id = %thread_id,
                active_run_count = active_run_ids.len(),
                "multiple active claude runs found for thread"
            );
        }

        active_run_ids.into_iter().next()
    }

    /// Perform auto-recovery: clear the corrupted session, create a fresh one,
    /// and return the last user message for replay.
    #[cfg(test)]
    async fn auto_recover_session(&self, thread_id: &str) -> Option<String> {
        tracing::warn!(
            thread_id = %thread_id,
            "auto-recovery: clearing corrupted session and preparing fresh thread session"
        );
        self.session_map.lock().await.remove(thread_id);
        self.session_failure_counts.lock().await.remove(thread_id);
        self.last_messages.lock().await.get(thread_id).cloned()
    }

    #[cfg(test)]
    async fn enqueue_test_run_attempt(&self, result: Result<Option<SdkRunOutcome>, BridgeError>) {
        self.test_run_attempts.lock().await.push_back(result);
    }

    #[cfg(test)]
    async fn recorded_test_session_attempts(&self) -> Vec<Option<String>> {
        self.test_recorded_session_attempts.lock().await.clone()
    }

    /// Execute a single SDK run attempt: connect, send message, process results.
    ///
    /// When `session_id` is `Some`, the run uses `--resume` and enforces a
    /// timeout on the first message (the CLI may hang on stale sessions).
    /// Returns `None` if the run produced no result message but had some text.
    async fn execute_sdk_run(
        &self,
        options: &ProviderRunOptions,
        session_id: Option<&str>,
        run_id: &str,
        on_chunk: &StreamCallback,
    ) -> Result<Option<SdkRunOutcome>, BridgeError> {
        #[cfg(test)]
        {
            self.test_recorded_session_attempts
                .lock()
                .await
                .push(session_id.map(ToOwned::to_owned));
            if let Some(result) = self.test_run_attempts.lock().await.pop_front() {
                return result;
            }
        }

        let connect_future = sdk_run_streaming(self.build_sdk_options(options, session_id, run_id));
        let mut run = connect_future
            .await
            .map_err(|e| BridgeError::RunFailed(format!("failed to connect to claude: {e}")))?;

        let control = run.control();
        self.register_run(run_id, &options.thread_id, control.clone())
            .await;
        self.initialize_pending_inputs(run_id).await;

        if let Err(e) = control
            .send_user_message(OutboundUserMessage {
                content: build_user_message_input(options, session_id.is_none()),
                session_id: String::new(),
                parent_tool_use_id: None,
            })
            .await
        {
            let _ = run.close().await;
            self.unregister_run(run_id).await;
            return Err(BridgeError::RunFailed(format!(
                "failed to send user message: {e}"
            )));
        }

        let (response_text, result_data, signals) = self
            .process_messages_streaming(run_id, &options.thread_id, &mut run, on_chunk)
            .await?;

        let _ = run.finish().await;
        self.unregister_run(run_id).await;

        if let Some(result) = result_data {
            let transcript_thread_title = if result.thread_title.is_none() {
                match resolve_claude_cwd(&self.config, options) {
                    Some(cwd) => {
                        read_claude_ai_title_from_transcript(&cwd, &result.session_id).await
                    }
                    None => None,
                }
            } else {
                None
            };
            let error_message = result
                .is_error
                .then(|| format_claude_run_error(&result, signals.last_assistant_error.as_ref()));
            Ok(Some(SdkRunOutcome {
                session_id: result.session_id,
                response_text,
                session_messages: result.session_messages,
                is_error: result.is_error,
                error_message,
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
                cost_usd: result.cost_usd,
                actual_model: result.actual_model,
                thread_title: result.thread_title.or(transcript_thread_title),
            }))
        } else if response_text.is_empty() {
            // No result and no text — treat as failure so retry can kick in
            Err(BridgeError::RunFailed(
                "no result from claude SDK".to_owned(),
            ))
        } else {
            // Got some text but no formal ResultMessage (e.g. run was
            // interrupted mid-stream). Preserve the partial text and
            // session_id, but do not report success: task lifecycle depends
            // on the ResultMessage as the only reliable completion marker.
            let error_message = match signals.last_assistant_error.as_ref() {
                Some(api_error) => format!(
                    "{CLAUDE_MISSING_RESULT_ERROR} (api_error={})",
                    api_error.as_label()
                ),
                None => CLAUDE_MISSING_RESULT_ERROR.to_owned(),
            };
            Ok(Some(SdkRunOutcome {
                session_id: session_id.unwrap_or_default().to_owned(),
                response_text,
                session_messages: Vec::new(),
                is_error: true,
                error_message: Some(error_message),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
                actual_model: None,
                thread_title: None,
            }))
        }
    }

    /// Core message processing loop with optional streaming callback.
    async fn process_messages_streaming(
        &self,
        run_id: &str,
        thread_id: &str,
        source: &mut (impl MessageSource + Send),
        on_chunk: &StreamCallback,
    ) -> Result<(String, Option<ProcessedResult>, StreamSignals), BridgeError> {
        // Drop any quota stash left by a prior attempt on this thread so a
        // stale entry can never be attributed to this attempt's terminal
        // record; the freshest attempt re-stages below when it actually hits
        // the quota.
        self.pending_rate_limits.lock().await.remove(thread_id);

        let mut response_text = String::new();
        let mut result_data: Option<ProcessedResult> = None;
        let mut signals = StreamSignals::default();
        let mut session_messages: Vec<ProviderMessage> = Vec::new();
        let mut assistant_or_tool_activity_seen = false;
        let mut actual_model: Option<String> = None;
        let mut thread_title: Option<String> = None;
        let mut result_seen = false;
        // Whether the last streamed event was assistant text, i.e. the
        // persistence tail segment is still in flight and a ResultMessage
        // should finalize it immediately instead of leaving it to Done
        // (post-result drain + process teardown otherwise keep the final
        // answer invisible for seconds).
        let mut assistant_text_in_flight = false;
        let mut active_background_tasks = HashSet::new();

        let idle_timeout = Duration::from_secs(STREAM_IDLE_TIMEOUT_SECS);
        let post_result_drain_timeout = Duration::from_secs(POST_RESULT_DRAIN_TIMEOUT_SECS);

        loop {
            let waiting_for_post_result_idle = result_seen && active_background_tasks.is_empty();
            let read_timeout = if waiting_for_post_result_idle {
                post_result_drain_timeout
            } else {
                idle_timeout
            };
            let msg = tokio::time::timeout(read_timeout, source.next_message()).await;
            match msg {
                Err(_elapsed) if waiting_for_post_result_idle => {
                    if self.try_close_pending_inputs(run_id).await {
                        break;
                    }
                    result_seen = false;
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        run_id = %run_id,
                        "stream idle for {}s, treating run as dead",
                        STREAM_IDLE_TIMEOUT_SECS,
                    );
                    break;
                }
                Ok(None) => break, // stream closed normally
                Ok(Some(msg_result)) => match msg_result {
                    Ok(Message::User(user_msg)) => {
                        // Both branches below finalize the assistant tail
                        // downstream (ToolResult events or the UserAck
                        // boundary).
                        assistant_text_in_flight = false;
                        if let claude_agent_sdk::UserContent::Blocks(ref blocks) = user_msg.content
                        {
                            extract_tool_session_messages(
                                blocks,
                                &mut session_messages,
                                Some(on_chunk),
                                user_msg.parent_tool_use_id.as_deref(),
                            );
                            if has_tool_result_blocks(blocks) || user_msg.tool_use_result.is_some()
                            {
                                continue;
                            }
                        }
                        // Keep Python parity: normal UserMessage echoes are ACKs that
                        // indicate one queued user input has been consumed.
                        result_seen = false;
                        on_chunk(StreamEvent::Boundary {
                            kind: StreamBoundaryKind::UserAck,
                            pending_input_id: self
                                .acknowledge_next_pending_input(
                                    run_id,
                                    assistant_or_tool_activity_seen,
                                )
                                .await,
                        });
                    }
                    Ok(Message::Assistant(assistant_msg)) => {
                        if let Some(api_error) = assistant_msg.error.as_ref() {
                            tracing::warn!(
                                run_id = %run_id,
                                thread_id = %thread_id,
                                api_error = api_error.as_label(),
                                "claude assistant message carried an API error"
                            );
                            signals.last_assistant_error = Some(api_error.clone());
                        }
                        if is_synthetic_no_response_message(&assistant_msg) {
                            tracing::debug!(
                                run_id = %run_id,
                                thread_id = %thread_id,
                                "suppressing claude synthetic no-response placeholder"
                            );
                            continue;
                        }
                        result_seen = false;
                        assistant_or_tool_activity_seen = true;
                        if actual_model.is_none() {
                            actual_model = Some(assistant_msg.model.trim().to_owned())
                                .filter(|value| !value.is_empty());
                        }
                        if !response_text.is_empty()
                            && assistant_blocks_have_visible_text(&assistant_msg.content)
                            && !assistant_blocks_start_with_newline(&assistant_msg.content)
                        {
                            append_assistant_segment_separator(&mut response_text);
                            on_chunk(StreamEvent::Boundary {
                                kind: StreamBoundaryKind::AssistantSegment,
                                pending_input_id: None,
                            });
                        }
                        process_assistant_blocks_streaming(
                            &assistant_msg.content,
                            &mut response_text,
                            &mut session_messages,
                            on_chunk,
                            assistant_msg.parent_tool_use_id.as_deref(),
                        );
                        if let Some(trailing_text) = assistant_blocks_trailing_text_emission(
                            &assistant_msg.content,
                            assistant_msg.parent_tool_use_id.as_deref(),
                        ) {
                            assistant_text_in_flight = trailing_text;
                        }
                    }
                    Ok(Message::Result(result_msg)) => {
                        result_seen = true;
                        // The turn's output is complete: finalize the trailing
                        // assistant segment now so the final answer commits
                        // (and reaches the per-thread stream) without waiting
                        // for the post-result drain, CLI exit grace, and Done.
                        // Error results keep the old flow so fresh-session
                        // retries never commit a doomed attempt's tail early.
                        if assistant_text_in_flight && !result_msg.is_error {
                            assistant_text_in_flight = false;
                            on_chunk(StreamEvent::Boundary {
                                kind: StreamBoundaryKind::AssistantSegment,
                                pending_input_id: None,
                            });
                        }
                        let (input_tokens, output_tokens) =
                            result_usage_tokens(result_msg.usage.as_ref());
                        if !result_msg.permission_denials.is_empty() {
                            let denied_tools: Vec<&str> = result_msg
                                .permission_denials
                                .iter()
                                .filter_map(|denial| {
                                    denial.get("tool_name").and_then(Value::as_str)
                                })
                                .collect();
                            tracing::warn!(
                                run_id = %run_id,
                                thread_id = %thread_id,
                                count = result_msg.permission_denials.len(),
                                tools = ?denied_tools,
                                "claude run had permission-denied tool calls"
                            );
                        }
                        if let Some(model_usage) = result_msg.model_usage.as_ref()
                            && !model_usage.is_empty()
                        {
                            tracing::debug!(
                                run_id = %run_id,
                                thread_id = %thread_id,
                                model_usage = %serde_json::to_string(model_usage)
                                    .unwrap_or_default(),
                                "claude per-model usage breakdown"
                            );
                        }
                        result_data = Some(ProcessedResult {
                            session_id: result_msg.session_id.clone(),
                            cost_usd: result_msg.total_cost_usd.unwrap_or(0.0),
                            input_tokens,
                            output_tokens,
                            is_error: result_msg.is_error,
                            subtype: result_msg.subtype.clone(),
                            terminal_reason: result_msg.terminal_reason.clone(),
                            stop_reason: result_msg.stop_reason.clone(),
                            errors: result_msg.errors.clone(),
                            api_error_status: result_msg.api_error_status,
                            actual_model: actual_model.clone(),
                            thread_title: thread_title.clone(),
                            session_messages: session_messages.clone(),
                        });
                    }
                    Ok(Message::System(sys_msg)) => {
                        update_claude_background_tasks(&sys_msg, &mut active_background_tasks);
                        if sys_msg.subtype == "task_notification" {
                            result_seen = false;
                        }
                        match sys_msg.subtype.as_str() {
                            // Subscription quota snapshot. Unknown top-level
                            // message types are surfaced by the SDK as System
                            // messages keyed by the type string, so
                            // `rate_limit_event` arrives here.
                            "rate_limit_event" => {
                                if let Some(info) = sys_msg
                                    .data
                                    .get("rate_limit_info")
                                    .filter(|value| value.is_object())
                                {
                                    let status = info
                                        .get("status")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default();
                                    if status.eq_ignore_ascii_case("rejected") {
                                        tracing::warn!(
                                            run_id = %run_id,
                                            thread_id = %thread_id,
                                            rate_limit_info = %info,
                                            "claude subscription rate limit rejected the request"
                                        );
                                    }
                                    signals.rate_limit_info = Some(info.clone());
                                }
                            }
                            // Advisory: the CLI is retrying a failed API
                            // request. Log-only, mirroring the Codex
                            // `willRetry` handling.
                            "api_retry" => {
                                let attempt = sys_msg
                                    .data
                                    .get("attempt")
                                    .and_then(Value::as_i64)
                                    .unwrap_or(0);
                                let max_retries = sys_msg
                                    .data
                                    .get("max_retries")
                                    .and_then(Value::as_i64)
                                    .unwrap_or(0);
                                let retry_delay_ms = sys_msg
                                    .data
                                    .get("retry_delay_ms")
                                    .and_then(Value::as_i64)
                                    .unwrap_or(0);
                                let error_status =
                                    sys_msg.data.get("error_status").and_then(Value::as_i64);
                                let error = sys_msg
                                    .data
                                    .get("error")
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                tracing::warn!(
                                    run_id = %run_id,
                                    thread_id = %thread_id,
                                    attempt,
                                    max_retries,
                                    retry_delay_ms,
                                    error_status = ?error_status,
                                    error,
                                    "claude API request failed; CLI is retrying"
                                );
                            }
                            // The model refused and the CLI rerouted the turn
                            // to a fallback model: subsequent output runs on
                            // the fallback, so it becomes the run's actual
                            // model. Mirrors Codex `model/rerouted`.
                            "model_refusal_fallback" => {
                                let original = sys_msg
                                    .data
                                    .get("original_model")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                let direction = sys_msg
                                    .data
                                    .get("direction")
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                let category = sys_msg
                                    .data
                                    .get("api_refusal_category")
                                    .and_then(Value::as_str);
                                if let Some(fallback) = sys_msg
                                    .data
                                    .get("fallback_model")
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                {
                                    tracing::warn!(
                                        run_id = %run_id,
                                        thread_id = %thread_id,
                                        original_model = original,
                                        fallback_model = fallback,
                                        direction,
                                        category = ?category,
                                        "claude model refusal fallback; updating actual model"
                                    );
                                    actual_model = Some(fallback.to_owned());
                                }
                            }
                            "model_refusal_no_fallback" => {
                                let original_model = sys_msg
                                    .data
                                    .get("original_model")
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                let category = sys_msg
                                    .data
                                    .get("api_refusal_category")
                                    .and_then(Value::as_str);
                                tracing::error!(
                                    run_id = %run_id,
                                    thread_id = %thread_id,
                                    original_model,
                                    category = ?category,
                                    "claude model refused the request with no fallback model"
                                );
                            }
                            // A failed compaction only surfaces on the status
                            // frame (`compact_result: "failed"`); successful
                            // compaction is paired on `compact_boundary`
                            // below, so pairing on failure here cannot
                            // double-emit.
                            "status" => {
                                let compact_result = sys_msg
                                    .data
                                    .get("compact_result")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                if compact_result.eq_ignore_ascii_case("failed") {
                                    let compact_error = sys_msg
                                        .data
                                        .get("compact_error")
                                        .and_then(Value::as_str)
                                        .map(str::trim)
                                        .filter(|value| !value.is_empty())
                                        .unwrap_or("context compaction failed");
                                    let activity_id = claude_system_activity_id(&sys_msg.data);
                                    emit_context_compaction_activity(
                                        &mut session_messages,
                                        on_chunk,
                                        &activity_id,
                                        serde_json::json!({}),
                                        serde_json::json!({ "compact_error": compact_error }),
                                        compact_error.to_owned(),
                                        true,
                                    );
                                }
                            }
                            // Successful context compaction boundary: emit the
                            // paired activity frame with the token accounting.
                            "compact_boundary" => {
                                let metadata = sys_msg
                                    .data
                                    .get("compact_metadata")
                                    .cloned()
                                    .unwrap_or_else(|| serde_json::json!({}));
                                let pre_tokens = metadata.get("pre_tokens").and_then(Value::as_i64);
                                let post_tokens =
                                    metadata.get("post_tokens").and_then(Value::as_i64);
                                let trigger = metadata
                                    .get("trigger")
                                    .and_then(Value::as_str)
                                    .unwrap_or("auto");
                                let result_text = match (pre_tokens, post_tokens) {
                                    (Some(pre), Some(post)) => format!(
                                        "compacted context ({trigger}): {pre} -> {post} tokens"
                                    ),
                                    (Some(pre), None) => format!(
                                        "compacted context ({trigger}): {pre} tokens before compaction"
                                    ),
                                    _ => format!("compacted context ({trigger})"),
                                };
                                let activity_id = claude_system_activity_id(&sys_msg.data);
                                emit_context_compaction_activity(
                                    &mut session_messages,
                                    on_chunk,
                                    &activity_id,
                                    serde_json::json!({ "trigger": trigger }),
                                    metadata,
                                    result_text,
                                    false,
                                );
                            }
                            _ => {}
                        }
                        // Eagerly capture the session_id from the `init` system
                        // message so it is persisted even if the run is
                        // interrupted before a formal Result message arrives.
                        if sys_msg.subtype == "init"
                            && let Some(sid) = sys_msg
                                .data
                                .get("session_id")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                        {
                            let stable_session_id = self.stabilize_session_id(thread_id, sid).await;
                            on_chunk(StreamEvent::SessionBound {
                                sdk_session_id: stable_session_id,
                            });
                        }
                        if thread_title.is_none() {
                            thread_title = extract_claude_thread_title(&sys_msg.data);
                        }
                    }
                    Ok(Message::StreamEvent(_)) => {
                        result_seen = false;
                    }
                    Err(e) => {
                        let bridge_error = bridge_error_from_sdk_stream_error(e);
                        match &bridge_error {
                            BridgeError::SessionParseUnsupportedBlock(_) => tracing::error!(
                                run_id = %run_id,
                                thread_id = %thread_id,
                                error = %bridge_error,
                                "unsupported SDK content block while reading Claude stream"
                            ),
                            _ => tracing::warn!(
                                run_id = %run_id,
                                thread_id = %thread_id,
                                error = %bridge_error,
                                "error receiving message from SDK"
                            ),
                        }
                        return Err(bridge_error);
                    }
                },
            }
        }

        // Stage quota-exhaustion context for the bridge run-completion path
        // (`take_rate_limit`) when this attempt terminated on the subscription
        // quota: an explicit `blocking_limit` terminal reason, or a rejected
        // `rate_limit_event` on a run that did not complete successfully.
        let run_errored = result_data.as_ref().is_none_or(|result| result.is_error);
        if run_errored {
            let errors_joined = result_data
                .as_ref()
                .map(|result| result.errors.join("; "))
                .filter(|joined| !joined.is_empty());
            if let Some(rate_limit) = build_claude_rate_limit(
                self.config.provider_type.as_slug(),
                result_data
                    .as_ref()
                    .and_then(|result| result.terminal_reason.as_deref()),
                signals.rate_limit_info.as_ref(),
                errors_joined.as_deref(),
            ) {
                tracing::warn!(
                    run_id = %run_id,
                    thread_id = %thread_id,
                    provider = %rate_limit.provider,
                    window = ?rate_limit.window,
                    reset_at = ?rate_limit.reset_at,
                    "claude run hit usage quota; staging rate-limit context for auto-resend",
                );
                self.pending_rate_limits
                    .lock()
                    .await
                    .insert(thread_id.to_owned(), rate_limit);
            }
        }

        Ok((response_text, result_data, signals))
    }
}

/// Extracted result data from a `ResultMessage`.
#[derive(Debug)]
struct ProcessedResult {
    session_id: String,
    cost_usd: f64,
    input_tokens: i64,
    output_tokens: i64,
    is_error: bool,
    /// Result frame subtype (`success`, `error_during_execution`,
    /// `error_max_turns`, …).
    subtype: String,
    /// Machine-readable terminal classification (open string).
    terminal_reason: Option<String>,
    /// API stop reason for the final turn, when reported.
    stop_reason: Option<String>,
    /// Human-readable error strings from an error result.
    errors: Vec<String>,
    /// HTTP status of the failing API request, when the run died on one.
    api_error_status: Option<i64>,
    actual_model: Option<String>,
    thread_title: Option<String>,
    session_messages: Vec<ProviderMessage>,
}

/// Outcome of a single `execute_sdk_run` attempt.
struct SdkRunOutcome {
    session_id: String,
    response_text: String,
    session_messages: Vec<ProviderMessage>,
    is_error: bool,
    error_message: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cost_usd: f64,
    actual_model: Option<String>,
    thread_title: Option<String>,
}

#[async_trait]
trait MessageSource {
    async fn next_message(&mut self) -> Option<claude_agent_sdk::Result<Message>>;
}

#[async_trait]
impl MessageSource for ClaudeRun {
    async fn next_message(&mut self) -> Option<claude_agent_sdk::Result<Message>> {
        self.next_message().await
    }
}

#[async_trait]
impl MessageSource for tokio::sync::mpsc::Receiver<claude_agent_sdk::Result<Message>> {
    async fn next_message(&mut self) -> Option<claude_agent_sdk::Result<Message>> {
        self.recv().await
    }
}

#[async_trait]
impl AgentLoopProvider for ClaudeCliProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    fn resolve_runtime_selection(&self, options: &ProviderRunOptions) -> ProviderRuntimeSelection {
        let effective_config = self.effective_config();
        ProviderRuntimeSelection {
            model: resolve_requested_model(&effective_config, &options.metadata),
            model_reasoning_effort: resolve_requested_effort(&effective_config, &options.metadata),
            model_service_tier: None,
        }
    }

    fn update_model_defaults(&self, defaults: &ProviderModelDefaults) {
        let mut model_defaults = self
            .model_defaults
            .write()
            .expect("claude model defaults lock poisoned");
        model_defaults.default_model = defaults.default_model.clone();
        model_defaults.model_reasoning_effort = defaults.model_reasoning_effort.clone();
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        let cli_mode = claude_sdk_cli_mode_label(&self.config);
        if let Some(path) = resolve_claude_sdk_cli_path(&self.config) {
            if !executable_file_exists(&path) {
                return Err(BridgeError::Internal(format!(
                    "Claude SDK CLI path is not executable: {}",
                    path.display()
                )));
            }
            let cli_prefix_args = resolve_claude_sdk_cli_prefix_args(&self.config);
            tracing::info!(
                claude_sdk_cli = %path.display(),
                claude_sdk_cli_prefix_args = ?cli_prefix_args,
                claude_sdk_cli_mode = cli_mode,
                model = %self.config.default_model,
                permission_mode = %self.config.permission_mode,
                workspace_dir = ?self.config.workspace_dir,
                "Claude SDK provider initialized"
            );
        } else if cli_mode == CLAUDE_CLI_MODE_NATIVE {
            let check = tokio::process::Command::new("which")
                .arg("claude")
                .output()
                .await;

            match check {
                Ok(output) if output.status.success() => {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    tracing::warn!(
                        claude_bin = %path,
                        claude_sdk_cli_mode = cli_mode,
                        model = %self.config.default_model,
                        permission_mode = %self.config.permission_mode,
                        workspace_dir = ?self.config.workspace_dir,
                        "Claude SDK provider initialized with native Claude CLI"
                    );
                }
                _ => {
                    return Err(BridgeError::Internal(
                        "Claude SDK CLI not found: install claude on PATH, configure claude_cli_path, or set claude_cli_mode=cctty to use Garyx's embedded cctty runner".to_owned(),
                    ));
                }
            }
        } else {
            return Err(BridgeError::Internal(
                "embedded cctty runner could not resolve the current garyx executable; configure claude_cli_path or set claude_cli_mode=native to use the original Claude CLI".to_owned(),
            ));
        }

        self.ready = true;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        tracing::info!("shutting down Claude SDK provider");

        // Drain active runs so we can tear them down explicitly.
        let runs: Vec<(String, ClaudeRunControl)> = {
            let mut map = self.active_runs.lock().await;
            map.drain().collect()
        };
        self.run_session_map.lock().await.clear();
        self.run_pending_inputs.lock().await.clear();

        for (run_id, run) in runs {
            if let Err(e) = run.interrupt().await {
                tracing::debug!(run_id = %run_id, error = %e, "failed to interrupt run on shutdown");
            }
            self.cleanup_run_handle(&run_id, run).await;
        }

        self.last_messages.lock().await.clear();
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

        // Store the last message for auto-recovery replay
        self.last_messages
            .lock()
            .await
            .insert(options.thread_id.clone(), options.message.clone());

        // Drop any quota stash left by a prior run on this thread so a stale
        // entry can never be attributed to this run's terminal record (e.g.
        // when this run dies before its stream loop starts).
        self.pending_rate_limits
            .lock()
            .await
            .remove(&options.thread_id);

        let run_id = resolve_run_id(&options.metadata);
        // Capture the requested model before the run starts so a concurrent
        // defaults reload cannot relabel this run's fallback actual_model.
        let requested_model = resolve_requested_model(&self.effective_config(), &options.metadata);

        let start = Instant::now();

        let session_id = {
            let map = self.session_map.lock().await;
            map.get(&options.thread_id).cloned()
        }
        .or_else(|| {
            non_empty_session_id(
                options
                    .metadata
                    .get("sdk_session_id")
                    .and_then(|v| v.as_str()),
            )
        });

        // Try with resume first; if it fails, times out, or returns is_error
        // (e.g. "No conversation found"), retry without resume.
        let attempt_result = self
            .execute_sdk_run(options, session_id.as_deref(), &run_id, &on_chunk)
            .await;

        let should_retry = session_id.is_some()
            && match &attempt_result {
                Err(error) => should_retry_with_fresh_session(error),
                Ok(Some(outcome)) if outcome.is_error => {
                    should_retry_message_with_fresh_session(&outcome.response_text)
                }
                _ => false,
            };
        // A wedged-but-intact session: the run "succeeded" with zero output
        // (the CLI closed a dangling turn with a synthetic "No response
        // requested." and exited without calling the model). The session
        // content itself is fine, so retry ON THE SAME SESSION to keep the
        // full conversation context instead of falling back to a fresh one.
        let should_retry_same_session = !should_retry
            && session_id.is_some()
            && matches!(
                &attempt_result,
                Ok(Some(outcome)) if resumed_run_stalled_without_response(outcome)
            );

        let mut result = if should_retry {
            let lost_history_messages = count_claude_transcript_history_messages(
                &self.config,
                options,
                session_id.as_deref(),
            )
            .await;
            // Log the original failure reason
            match &attempt_result {
                Err(e) => tracing::error!(
                    thread_id = %options.thread_id,
                    sdk_session_id = session_id.as_deref().unwrap_or(""),
                    lost_history_messages = lost_history_messages.map(|count| count as i64).unwrap_or(-1),
                    lost_history_messages_known = lost_history_messages.is_some(),
                    error = %e,
                    "resume failed, retrying as new session"
                ),
                Ok(Some(outcome)) => tracing::error!(
                    thread_id = %options.thread_id,
                    sdk_session_id = session_id.as_deref().unwrap_or(""),
                    lost_history_messages = lost_history_messages.map(|count| count as i64).unwrap_or(-1),
                    lost_history_messages_known = lost_history_messages.is_some(),
                    response = %outcome.response_text,
                    is_error = outcome.is_error,
                    "resume returned error, retrying as new session"
                ),
                _ => {}
            }
            self.session_map.lock().await.remove(&options.thread_id);
            self.reset_failure_count(&options.thread_id).await;
            self.execute_sdk_run(options, None, &run_id, &on_chunk)
                .await?
        } else if should_retry_same_session {
            tracing::warn!(
                thread_id = %options.thread_id,
                sdk_session_id = session_id.as_deref().unwrap_or(""),
                "resumed run produced no response; retrying once on the same session"
            );
            self.execute_sdk_run(options, session_id.as_deref(), &run_id, &on_chunk)
                .await?
        } else {
            attempt_result?
        };

        let duration_ms = start.elapsed().as_millis() as i64;

        // Keep thread session id stable across runs. Only a cleared mapping
        // (e.g. fresh-session retry) is allowed to bind to a new session id.
        if let Some(ref mut res) = result {
            if !res.session_id.is_empty() {
                res.session_id = self
                    .stabilize_session_id(&options.thread_id, &res.session_id)
                    .await;
            }
            self.reset_failure_count(&options.thread_id).await;
        }

        // Send explicit completion event
        on_chunk(StreamEvent::Done);

        match result {
            Some(result) => Ok(ProviderRunResult {
                run_id,
                thread_id: options.thread_id.clone(),
                response: result.response_text,
                session_messages: result.session_messages,
                sdk_session_id: non_empty_session_id(Some(result.session_id.as_str())),
                actual_model: result.actual_model.or_else(|| requested_model.clone()),
                thread_title: result.thread_title,
                success: !result.is_error,
                error: if result.is_error {
                    result
                        .error_message
                        .or_else(|| Some("claude SDK reported error".to_owned()))
                } else {
                    None
                },
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
                cost: result.cost_usd,
                duration_ms,
            }),
            None => Err(BridgeError::RunFailed(
                "no result from claude SDK".to_owned(),
            )),
        }
    }

    async fn abort(&self, run_id: &str) -> bool {
        let (run, _thread_id) = self.unregister_run(run_id).await;
        if let Some(run) = run {
            // Try graceful interrupt with timeout; force-cleanup if it hangs
            let interrupt_result =
                tokio::time::timeout(Duration::from_secs(ABORT_TIMEOUT_SECS), run.interrupt())
                    .await;

            match interrupt_result {
                Ok(Ok(())) => {
                    tracing::info!(run_id = %run_id, "aborted claude run");
                }
                Ok(Err(e)) => {
                    tracing::warn!(run_id = %run_id, error = %e, "interrupt failed, force-cleaning up");
                }
                Err(_) => {
                    tracing::warn!(run_id = %run_id, "interrupt timed out after {}s, force-cleaning up", ABORT_TIMEOUT_SECS);
                }
            }

            self.cleanup_run_handle(run_id, run).await;

            true
        } else {
            false
        }
    }

    async fn take_rate_limit(&self, thread_id: &str) -> Option<ProviderRateLimit> {
        self.pending_rate_limits.lock().await.remove(thread_id)
    }

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn add_streaming_input(&self, thread_id: &str, input: QueuedUserInput) -> bool {
        if let Some(run_id) = self.resolve_active_run_id_for_session(thread_id).await {
            let run = { self.active_runs.lock().await.get(&run_id).cloned() };
            if let Some(run) = run {
                let pending_input_id = input.pending_input_id.unwrap_or_default();
                if pending_input_id.trim().is_empty() {
                    return false;
                }
                if !self
                    .enqueue_pending_input(&run_id, pending_input_id.clone())
                    .await
                {
                    return false;
                }
                let outbound = match build_user_message_input_from_parts(
                    &input.message,
                    &input.images,
                    &input.attachments,
                ) {
                    UserInput::Text(text) => OutboundUserMessage::text(text, ""),
                    UserInput::Blocks(blocks) => OutboundUserMessage::blocks(blocks, ""),
                };
                match run.send_user_message(outbound).await {
                    Ok(()) => {
                        tracing::debug!(thread_id = %thread_id, "queued streaming input");
                        true
                    }
                    Err(e) => {
                        self.rollback_pending_input(&run_id, &pending_input_id)
                            .await;
                        tracing::warn!(
                            thread_id = %thread_id,
                            error = %e,
                            "failed to queue streaming input"
                        );
                        false
                    }
                }
            } else {
                let _ = self.unregister_run(&run_id).await;
                false
            }
        } else {
            false
        }
    }

    async fn interrupt_streaming_session(&self, thread_id: &str) -> bool {
        if let Some(run_id) = self.resolve_active_run_id_for_session(thread_id).await {
            self.abort(&run_id).await
        } else {
            false
        }
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        let mut map = self.session_map.lock().await;
        if let Some(existing) = map.get(thread_id) {
            return Ok(existing.clone());
        }
        let new_id = Uuid::new_v4().to_string();
        map.insert(thread_id.to_owned(), new_id.clone());
        Ok(new_id)
    }

    async fn clear_session(&self, thread_id: &str) -> bool {
        let _ = self.interrupt_streaming_session(thread_id).await;
        let stale_run_ids: Vec<String> = {
            let map = self.run_session_map.lock().await;
            map.iter()
                .filter(|(_, mapped_thread_id)| mapped_thread_id.as_str() == thread_id)
                .map(|(run_id, _)| run_id.clone())
                .collect()
        };
        for run_id in stale_run_ids {
            let _ = self.unregister_run(&run_id).await;
        }
        let removed = self.session_map.lock().await.remove(thread_id);
        self.session_failure_counts.lock().await.remove(thread_id);
        self.last_messages.lock().await.remove(thread_id);
        tracing::info!(
            thread_id = %thread_id,
            removed = removed.is_some(),
            "cleared session"
        );
        removed.is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
