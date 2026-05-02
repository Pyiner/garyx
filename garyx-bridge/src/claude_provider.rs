use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use claude_agent_sdk::{
    ClaudeAgentDefinition, ClaudeAgentOptions, ClaudeRun, ClaudeRunControl, ContentBlock,
    McpServerConfig, Message, OutboundUserMessage, PermissionMode, TextBlock, UserInput,
    run_streaming as sdk_run_streaming,
};
use garyx_models::provider::{
    ClaudeCodeConfig, ImagePayload, PromptAttachment, ProviderMessage, ProviderRunOptions,
    ProviderRunResult, ProviderType, QueuedUserInput, StreamBoundaryKind, StreamEvent,
    attachments_from_metadata, build_prompt_message_with_attachments,
};
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::gary_prompt::{
    append_task_suffix_to_user_message, compose_gary_instructions, task_cli_env,
};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

// ---------------------------------------------------------------------------
// Retry configuration
// ---------------------------------------------------------------------------

#[cfg(test)]
const MAX_SESSION_FAILURES: u32 = 3;
const MAX_BUFFER_SIZE: usize = 10 * 1024 * 1024; // 10 MB
const ABORT_TIMEOUT_SECS: u64 = 10;
const STREAM_IDLE_TIMEOUT_SECS: u64 = 3600; // 1 hour

// ---------------------------------------------------------------------------
// Error classification helpers
// ---------------------------------------------------------------------------

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

fn should_retry_with_fresh_session(msg: &str) -> bool {
    let patterns = [
        "failed to connect to claude",
        "control protocol error",
        "cli process exited before responding",
        "no result from claude sdk",
    ];
    let lower = msg.to_lowercase();
    is_session_corrupted_error(msg) || patterns.iter().any(|pattern| lower.contains(pattern))
}

fn non_empty_session_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
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

fn assistant_blocks_start_with_newline(blocks: &[ContentBlock]) -> bool {
    blocks.iter().find_map(|block| match block {
        ContentBlock::Text(TextBlock { text }) if !text.is_empty() => Some(text.starts_with('\n')),
        _ => None,
    }) == Some(true)
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
            ContentBlock::Image(_) | ContentBlock::Thinking(_) => {}
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

fn build_user_message_input(options: &ProviderRunOptions) -> UserInput {
    let images = options.images.as_deref().unwrap_or_default();
    let attachments = attachments_from_metadata(&options.metadata);
    let message = build_native_skill_prompt(&options.message, &options.metadata)
        .unwrap_or_else(|| options.message.clone());
    let message = append_task_suffix_to_user_message(&message, &options.metadata);
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
        Self {
            config,
            session_map: Mutex::new(HashMap::new()),
            session_failure_counts: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            run_session_map: Mutex::new(HashMap::new()),
            run_pending_inputs: Mutex::new(HashMap::new()),
            last_messages: Mutex::new(HashMap::new()),
            #[cfg(test)]
            test_run_attempts: Mutex::new(VecDeque::new()),
            #[cfg(test)]
            test_recorded_session_attempts: Mutex::new(Vec::new()),
            ready: false,
        }
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
        let cwd = options
            .workspace_dir
            .as_ref()
            .or(self.config.workspace_dir.as_ref())
            .map(|ws| PathBuf::from(shellexpand::tilde(ws).as_ref()))
            .filter(|p| p.exists())
            .or_else(|| std::env::current_dir().ok());

        // Model: metadata override > config default
        let model = resolve_requested_model(&self.config, &options.metadata);

        let runtime_system_prompt = options
            .metadata
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .or(self.config.system_prompt.as_deref());

        let session_agent_id = options
            .metadata
            .get("agent_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
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
                let agent_prompt = runtime_system_prompt.unwrap_or("").to_owned();
                let description = format!("Garyx custom agent: {session_agent_name}");
                (
                    Some(agent_id.to_owned()),
                    HashMap::from([(
                        agent_id.to_owned(),
                        ClaudeAgentDefinition {
                            description,
                            prompt: agent_prompt,
                        },
                    )]),
                    None,
                    None,
                )
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
        env.extend(metadata_string_map(&options.metadata, "desktop_claude_env"));

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
            env,
            extra_args,
            max_buffer_size: Some(MAX_BUFFER_SIZE),
            setting_sources: (!self.config.setting_sources.is_empty())
                .then(|| self.config.setting_sources.clone()),
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
        if let Some(queue) = pending.get_mut(run_id) {
            if let Some(index) = queue.iter().position(|marker| {
                matches!(marker, PendingAckMarker::QueuedInput(candidate) if candidate == pending_input_id)
            }) {
                queue.remove(index);
            }
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

    /// Atomically check whether the pending-input queue is empty and, if so,
    /// **remove** the queue entry so that subsequent [`enqueue_pending_input`]
    /// calls for this `run_id` will fail.  This closes the race window where a
    /// new input is enqueued between the emptiness check and the loop break in
    /// [`process_messages_streaming`].
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
                content: build_user_message_input(options),
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

        let (response_text, result_data) = self
            .process_messages_streaming(run_id, &options.thread_id, &mut run, on_chunk)
            .await;

        let _ = run.close().await;
        self.unregister_run(run_id).await;

        if let Some(result) = result_data {
            Ok(Some(SdkRunOutcome {
                session_id: result.session_id,
                response_text,
                session_messages: result.session_messages,
                is_error: result.is_error,
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
                cost_usd: result.cost_usd,
                actual_model: result.actual_model,
            }))
        } else if response_text.is_empty() {
            // No result and no text — treat as failure so retry can kick in
            Err(BridgeError::RunFailed(
                "no result from claude SDK".to_owned(),
            ))
        } else {
            // Got some text but no formal ResultMessage (e.g. run was
            // interrupted mid-stream).  Preserve the session_id that was
            // used to start this run so that `run_streaming` does not
            // discard the session mapping — the underlying Claude session
            // is still valid and the next run should resume from it.
            Ok(Some(SdkRunOutcome {
                session_id: session_id.unwrap_or_default().to_owned(),
                response_text,
                session_messages: Vec::new(),
                is_error: false,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
                actual_model: None,
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
    ) -> (String, Option<ProcessedResult>) {
        let mut response_text = String::new();
        let mut result_data: Option<ProcessedResult> = None;
        let mut session_messages: Vec<ProviderMessage> = Vec::new();
        let mut assistant_or_tool_activity_seen = false;
        let mut actual_model: Option<String> = None;

        let idle_timeout = Duration::from_secs(STREAM_IDLE_TIMEOUT_SECS);

        loop {
            let msg = tokio::time::timeout(idle_timeout, source.next_message()).await;
            match msg {
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
                    }
                    Ok(Message::Result(result_msg)) => {
                        result_data = Some(ProcessedResult {
                            session_id: result_msg.session_id.clone(),
                            cost_usd: result_msg.total_cost_usd.unwrap_or(0.0),
                            input_tokens: result_msg
                                .usage
                                .as_ref()
                                .and_then(|u| u.get("input"))
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0),
                            output_tokens: result_msg
                                .usage
                                .as_ref()
                                .and_then(|u| u.get("output"))
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0),
                            is_error: result_msg.is_error,
                            actual_model: actual_model.clone(),
                            session_messages: session_messages.clone(),
                        });
                        // Atomically check-and-close: if no queued inputs remain,
                        // remove the queue entry so that concurrent
                        // add_streaming_input callers see the run as closed and
                        // fall through to start a new run instead of queuing
                        // into a dying session.
                        if self.try_close_pending_inputs(run_id).await {
                            break;
                        }
                    }
                    Ok(Message::System(sys_msg)) => {
                        // Eagerly capture the session_id from the `init` system
                        // message so it is persisted even if the run is
                        // interrupted before a formal Result message arrives.
                        if sys_msg.subtype == "init" {
                            if let Some(sid) = sys_msg
                                .data
                                .get("session_id")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                            {
                                let _ = self.stabilize_session_id(thread_id, sid).await;
                            }
                        }
                    }
                    Ok(Message::StreamEvent(_)) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "error receiving message from SDK");
                        break;
                    }
                },
            }
        }

        (response_text, result_data)
    }
}

/// Extracted result data from a `ResultMessage`.
struct ProcessedResult {
    session_id: String,
    cost_usd: f64,
    input_tokens: i64,
    output_tokens: i64,
    is_error: bool,
    actual_model: Option<String>,
    session_messages: Vec<ProviderMessage>,
}

/// Outcome of a single `execute_sdk_run` attempt.
struct SdkRunOutcome {
    session_id: String,
    response_text: String,
    session_messages: Vec<ProviderMessage>,
    is_error: bool,
    input_tokens: i64,
    output_tokens: i64,
    cost_usd: f64,
    actual_model: Option<String>,
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

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        // Verify the claude binary is available by checking if it can be found
        let check = tokio::process::Command::new("which")
            .arg("claude")
            .output()
            .await;

        match check {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                tracing::info!(
                    claude_bin = %path,
                    model = %self.config.default_model,
                    permission_mode = %self.config.permission_mode,
                    workspace_dir = ?self.config.workspace_dir,
                    "Claude SDK provider initialized"
                );
            }
            _ => {
                // Try claude directly
                let version_check = tokio::process::Command::new("claude")
                    .arg("--version")
                    .output()
                    .await;

                match version_check {
                    Ok(output) if output.status.success() => {
                        tracing::info!("Claude SDK provider initialized (claude on PATH)");
                    }
                    _ => {
                        return Err(BridgeError::Internal(
                            "claude binary not found in PATH".to_owned(),
                        ));
                    }
                }
            }
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

        let run_id = resolve_run_id(&options.metadata);

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
                Err(error) => should_retry_with_fresh_session(&error.to_string()),
                Ok(Some(outcome)) if outcome.is_error => {
                    should_retry_with_fresh_session(&outcome.response_text)
                }
                _ => false,
            };

        let mut result = if should_retry {
            // Log the original failure reason
            match &attempt_result {
                Err(e) => tracing::warn!(
                    thread_id = %options.thread_id,
                    error = %e,
                    "resume failed, retrying as new session"
                ),
                Ok(Some(outcome)) => tracing::warn!(
                    thread_id = %options.thread_id,
                    response = %outcome.response_text,
                    "resume returned error, retrying as new session"
                ),
                _ => {}
            }
            self.session_map.lock().await.remove(&options.thread_id);
            self.reset_failure_count(&options.thread_id).await;
            self.execute_sdk_run(options, None, &run_id, &on_chunk)
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
                actual_model: result
                    .actual_model
                    .or_else(|| resolve_requested_model(&self.config, &options.metadata)),
                success: !result.is_error,
                error: if result.is_error {
                    Some("claude SDK reported error".to_owned())
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
