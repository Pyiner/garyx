//! Codex app-server agent provider.
//!
//! Rust port of `the original codex_provider.py`.
//! Implements `AgentLoopProvider` backed by `codex_sdk::CodexClient`,
//! managing thread/turn lifecycle and streaming notifications.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use codex_sdk::types::{coerce_f64, coerce_i64};
use codex_sdk::{
    CodexClient, CodexClientConfig, CodexError, InputItem, JsonRpcNotification, ThreadResumeParams,
    ThreadStartParams,
};
use garyx_models::provider::{
    CodexAppServerConfig, ImagePayload, PromptAttachment, ProviderMessage, ProviderMessageRole,
    ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput, StreamBoundaryKind,
    StreamEvent, attachments_from_metadata, build_prompt_message_with_attachments,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::gary_prompt::{
    append_task_suffix_to_user_message, compose_gary_instructions,
    prepend_auto_memory_to_user_message, task_cli_env,
};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

const CODEX_CLIENT_IDLE_TTL: Duration = Duration::from_secs(180);

// ---------------------------------------------------------------------------
// Helper functions (provider-level domain mapping)
// ---------------------------------------------------------------------------

/// Check whether a notification's params match our expected thread/turn.
fn matches_turn(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    if let Some(event_thread) = params.get("threadId").and_then(|v| v.as_str()) {
        if !event_thread.is_empty() && event_thread != thread_id {
            return false;
        }
    }
    if let Some(event_turn) = params.get("turnId").and_then(|v| v.as_str()) {
        if !event_turn.is_empty() && event_turn != turn_id {
            return false;
        }
    }
    if let Some(turn_obj_id) = params
        .get("turn")
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
    {
        if !turn_obj_id.is_empty() && turn_obj_id != turn_id {
            return false;
        }
    }
    true
}

/// Extract usage (input_tokens, output_tokens, cost) from a completed turn.
fn extract_usage(turn: &Value) -> (i64, i64, f64) {
    let usage = match turn.get("usage") {
        Some(u) if u.is_object() => u,
        _ => return (0, 0, 0.0),
    };

    let input_tokens = ["inputTokens", "input_tokens", "input", "prompt_tokens"]
        .iter()
        .find_map(|k| usage.get(*k).filter(|v| !v.is_null()))
        .map(coerce_i64)
        .unwrap_or(0);

    let output_tokens = [
        "outputTokens",
        "output_tokens",
        "output",
        "completion_tokens",
    ]
    .iter()
    .find_map(|k| usage.get(*k).filter(|v| !v.is_null()))
    .map(coerce_i64)
    .unwrap_or(0);

    let cost = ["totalCostUsd", "total_cost_usd", "costUsd", "cost"]
        .iter()
        .find_map(|k| usage.get(*k).filter(|v| !v.is_null()))
        .map(coerce_f64)
        .unwrap_or(0.0);

    (input_tokens, output_tokens, cost)
}

/// Build typed `InputItem` vector from `ProviderRunOptions`.
fn build_input_items_from_parts(
    message: &str,
    images: &[ImagePayload],
    attachments: &[PromptAttachment],
) -> Vec<InputItem> {
    let message = build_prompt_message_with_attachments(message, attachments);
    if !attachments.is_empty() {
        return vec![InputItem::Text { text: message }];
    }

    let mut items = Vec::with_capacity(images.len() + 1);
    if !message.trim().is_empty() || images.is_empty() {
        items.push(InputItem::Text { text: message });
    }

    for image in images {
        if image.data.trim().is_empty() {
            continue;
        }
        items.push(InputItem::Image {
            url: format!("data:{};base64,{}", image.media_type, image.data),
        });
    }

    items
}

#[derive(Debug, Default, serde::Deserialize)]
struct CodexCliConfigFile {
    model: Option<String>,
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn default_codex_config_path() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(home).join("config.toml"));
    }

    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".codex").join("config.toml"))
}

fn read_codex_cli_default_model_from_path(path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let parsed: CodexCliConfigFile = toml::from_str(&contents).ok()?;
    normalize_non_empty(parsed.model.as_deref())
}

fn resolve_codex_actual_model_with_config_path(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
    config_path: Option<&Path>,
) -> Option<String> {
    normalize_non_empty(metadata.get("model").and_then(Value::as_str))
        .or_else(|| normalize_non_empty(Some(config.model.as_str())))
        .or_else(|| normalize_non_empty(Some(config.default_model.as_str())))
        .or_else(|| config_path.and_then(read_codex_cli_default_model_from_path))
}

fn resolve_codex_actual_model(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    let config_path = default_codex_config_path();
    resolve_codex_actual_model_with_config_path(config, metadata, config_path.as_deref())
}

fn normalize_codex_mcp_servers(metadata: &HashMap<String, Value>) -> Option<Value> {
    let servers = metadata.get("remote_mcp_servers")?.as_object()?;
    let mut normalized = serde_json::Map::new();

    for (name, raw_server) in servers {
        let Some(server) = raw_server.as_object() else {
            continue;
        };
        let mut entry = serde_json::Map::new();

        if let Some(command) = server
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            entry.insert("command".to_owned(), Value::String(command.to_owned()));
            entry.insert(
                "args".to_owned(),
                Value::Array(
                    server
                        .get("args")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.as_str().map(|value| value.to_owned()))
                                .map(Value::String)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                ),
            );

            let env = server
                .get("env")
                .and_then(Value::as_object)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|(env_key, env_value)| {
                            env_value.as_str().map(|env_value| {
                                (env_key.clone(), Value::String(env_value.to_owned()))
                            })
                        })
                        .collect::<serde_json::Map<_, _>>()
                })
                .unwrap_or_default();
            if !env.is_empty() {
                entry.insert("env".to_owned(), Value::Object(env));
            }
            if let Some(enabled) = server.get("enabled").and_then(Value::as_bool) {
                entry.insert("enabled".to_owned(), Value::Bool(enabled));
            }
            if let Some(cwd) = server
                .get("cwd")
                .or_else(|| server.get("working_dir"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let canonical = std::fs::canonicalize(cwd)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| cwd.to_owned());
                entry.insert("cwd".to_owned(), Value::String(canonical));
            }
        } else if let Some(url) = server
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            entry.insert("url".to_owned(), Value::String(url.to_owned()));
            if let Some(enabled) = server.get("enabled").and_then(Value::as_bool) {
                entry.insert("enabled".to_owned(), Value::Bool(enabled));
            }
            if let Some(headers) = server.get("headers").and_then(Value::as_object) {
                let http_headers = headers
                    .iter()
                    .filter_map(|(header_key, header_value)| {
                        header_value.as_str().map(|header_value| {
                            (header_key.clone(), Value::String(header_value.to_owned()))
                        })
                    })
                    .collect::<serde_json::Map<_, _>>();
                if !http_headers.is_empty() {
                    entry.insert("http_headers".to_owned(), Value::Object(http_headers));
                }
            }
            if matches!(
                server.get("type").and_then(Value::as_str),
                Some(kind) if kind.eq_ignore_ascii_case("sse")
            ) {
                entry.insert("transport".to_owned(), Value::String("sse".to_owned()));
            }
        }

        if !entry.is_empty() {
            normalized.insert(name.clone(), Value::Object(entry));
        }
    }

    (!normalized.is_empty()).then_some(Value::Object(normalized))
}

fn metadata_string_map(metadata: &HashMap<String, Value>, key: &str) -> HashMap<String, String> {
    metadata
        .get(key)
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(env_key, env_value)| {
                    env_value
                        .as_str()
                        .map(|env_value| (env_key.clone(), env_value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_runtime_codex_env(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
) -> HashMap<String, String> {
    let mut env = config.env.clone();
    env.extend(task_cli_env(metadata));
    env.extend(metadata_string_map(metadata, "desktop_codex_env"));
    env
}

fn garyx_mcp_server(
    config: &CodexAppServerConfig,
    thread_id: &str,
    run_id: &str,
    metadata: &HashMap<String, Value>,
) -> Option<Value> {
    let base_url = config.mcp_base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return None;
    }

    let mut http_headers = serde_json::Map::from_iter([
        ("X-Run-Id".to_owned(), Value::String(run_id.to_owned())),
        (
            "X-Thread-Id".to_owned(),
            Value::String(thread_id.to_owned()),
        ),
        (
            "X-Session-Key".to_owned(),
            Value::String(thread_id.to_owned()),
        ),
    ]);
    for (key, value) in metadata_string_map(metadata, "garyx_mcp_headers") {
        http_headers.insert(key, Value::String(value));
    }

    // Encode thread_id and run_id into the URL path so the gateway can
    // extract context even when the client strips custom headers (matches
    // the Claude Code workaround in claude_provider.rs).
    let encoded_thread = urlencoding::encode(thread_id);
    let encoded_run = urlencoding::encode(run_id);
    let url = metadata
        .get("garyx_mcp_auth_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|token| {
            format!(
                "{base_url}/mcp/auth/{}/{}/{}",
                urlencoding::encode(token),
                encoded_thread,
                encoded_run
            )
        })
        .unwrap_or_else(|| format!("{base_url}/mcp/{encoded_thread}/{encoded_run}"));
    Some(json!({
        "url": url,
        "http_headers": http_headers,
    }))
}

fn build_codex_thread_config(
    provider_config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
    thread_id: &str,
    run_id: &str,
    workspace_dir: Option<&Path>,
) -> Option<Value> {
    let mut thread_config = serde_json::Map::new();

    let runtime_instructions = metadata
        .get("developer_instructions")
        .and_then(|v| v.as_str())
        .or_else(|| metadata.get("system_prompt").and_then(|v| v.as_str()));
    let automation_id = metadata.get("automation_id").and_then(|v| v.as_str());
    let instructions =
        compose_gary_instructions(runtime_instructions, workspace_dir, automation_id);
    thread_config.insert(
        "developer_instructions".to_owned(),
        Value::String(instructions),
    );

    let mut mcp_servers = match normalize_codex_mcp_servers(metadata) {
        Some(Value::Object(obj)) => obj,
        _ => serde_json::Map::new(),
    };
    if let Some(server) = garyx_mcp_server(provider_config, thread_id, run_id, metadata) {
        // Reserve `garyx` for the built-in local gateway endpoint so runtime
        // metadata cannot shadow it with a stale or malformed URL.
        mcp_servers.insert("garyx".to_owned(), server);
    }
    if !mcp_servers.is_empty() {
        thread_config.insert("mcp_servers".to_owned(), Value::Object(mcp_servers));
    }
    (!thread_config.is_empty()).then_some(Value::Object(thread_config))
}

fn build_input_items(options: &ProviderRunOptions, include_memory: bool) -> Vec<InputItem> {
    let message = build_native_skill_prompt(&options.message, &options.metadata)
        .unwrap_or_else(|| options.message.clone());
    let message = append_task_suffix_to_user_message(&message, &options.metadata);
    let message = prepend_auto_memory_to_user_message(&message, &options.metadata, include_memory);
    let attachments = attachments_from_metadata(&options.metadata);
    build_input_items_from_parts(
        &message,
        options.images.as_deref().unwrap_or_default(),
        &attachments,
    )
}

fn append_codex_assistant_session_message(
    session_messages: &mut Vec<ProviderMessage>,
    item_id: Option<&str>,
    delta: &str,
) {
    if delta.is_empty() {
        return;
    }

    let normalized_item_id = item_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let can_append = session_messages.last().is_some_and(|message| {
        message.role == ProviderMessageRole::Assistant
            && message.metadata.get("source").and_then(Value::as_str) == Some("codex_app_server")
            && message
                .metadata
                .get("item_id")
                .and_then(Value::as_str)
                .map(|value| value.to_owned())
                == normalized_item_id
    });

    if can_append {
        if let Some(last) = session_messages.last_mut() {
            let mut text = last.text.clone().unwrap_or_default();
            text.push_str(delta);
            last.text = Some(text.clone());
            last.content = Value::String(text);
        }
        return;
    }

    let mut message = ProviderMessage::assistant_text(delta)
        .with_timestamp(chrono::Utc::now().to_rfc3339())
        .with_metadata_value("source", serde_json::json!("codex_app_server"))
        .with_metadata_value("item_type", serde_json::json!("agentMessage"));
    if let Some(item_id) = normalized_item_id {
        message = message.with_metadata_value("item_id", serde_json::json!(item_id));
    }
    session_messages.push(message);
}

/// Build a tool session message from an item notification.
fn build_tool_session_message(item: &Value, is_completed: bool) -> Option<ProviderMessage> {
    let item_type = codex_thread_item_type(item)?;
    if !is_codex_structured_activity_item_type(item_type) {
        return None;
    }

    let tool_use_id = item
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    let tool_name = codex_structured_activity_name(item_type, item);

    // Garyx's existing stream protocol represents provider-side structured
    // activity as ToolUse/ToolResult frames. Preserve Codex's original item
    // type in metadata so each channel can decide how to render it.
    let mut msg = if is_completed {
        ProviderMessage::tool_result(
            item.clone(),
            (!tool_use_id.is_empty()).then_some(tool_use_id),
            Some(tool_name),
            Some(codex_structured_activity_is_error(item)),
        )
    } else {
        ProviderMessage::tool_use(
            item.clone(),
            (!tool_use_id.is_empty()).then_some(tool_use_id),
            Some(tool_name),
        )
    };

    msg = msg
        .with_timestamp(chrono::Utc::now().to_rfc3339())
        .with_metadata_value("source", serde_json::json!("codex_app_server"))
        .with_metadata_value("item_type", serde_json::json!(item_type));

    Some(msg)
}

fn codex_thread_item_type(item: &Value) -> Option<&str> {
    item.get("type")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
}

fn is_codex_structured_activity_item_type(item_type: &str) -> bool {
    [
        "hookPrompt",
        "plan",
        "reasoning",
        "commandExecution",
        "fileChange",
        "mcpToolCall",
        "dynamicToolCall",
        "collabAgentToolCall",
        "webSearch",
        "imageView",
        "imageGeneration",
        "enteredReviewMode",
        "exitedReviewMode",
        "contextCompaction",
    ]
    .iter()
    .any(|candidate| item_type.eq_ignore_ascii_case(candidate))
}

fn codex_structured_activity_name(item_type: &str, item: &Value) -> String {
    if item_type.eq_ignore_ascii_case("mcpToolCall") {
        let server = item.get("server").and_then(|v| v.as_str()).unwrap_or("");
        let tool = item.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        if !tool.is_empty() {
            return format!("mcp:{server}:{tool}");
        }
    }

    if item_type.eq_ignore_ascii_case("dynamicToolCall") {
        let namespace = item
            .get("namespace")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let tool = item
            .get("tool")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        return match (namespace, tool) {
            (Some(namespace), Some(tool)) => format!("{namespace}:{tool}"),
            (_, Some(tool)) => tool.to_owned(),
            _ => item_type.to_owned(),
        };
    }

    if item_type.eq_ignore_ascii_case("collabAgentToolCall") {
        if let Some(tool) = item
            .get("tool")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return tool.to_owned();
        }
    }

    item_type.to_owned()
}

fn codex_structured_activity_is_error(item: &Value) -> bool {
    let failed_status = item
        .get("status")
        .and_then(|v| v.as_str())
        .map(|status| {
            let status = status.trim();
            status.eq_ignore_ascii_case("failed")
                || status.eq_ignore_ascii_case("declined")
                || status.eq_ignore_ascii_case("error")
                || status.eq_ignore_ascii_case("canceled")
                || status.eq_ignore_ascii_case("cancelled")
        })
        .unwrap_or(false);
    if failed_status {
        return true;
    }

    let explicit_failure = item
        .get("success")
        .and_then(Value::as_bool)
        .map(|success| !success)
        .unwrap_or(false);
    explicit_failure || item.get("error").is_some_and(|error| !error.is_null())
}

fn is_agent_message_item(item: &Value) -> bool {
    item.get("type")
        .and_then(|v| v.as_str())
        .map(|kind| kind.eq_ignore_ascii_case("agentMessage"))
        .unwrap_or(false)
}

fn is_user_message_item(item: &Value) -> bool {
    item.get("type")
        .and_then(|v| v.as_str())
        .map(|kind| kind.eq_ignore_ascii_case("userMessage"))
        .unwrap_or(false)
}

#[cfg(test)]
fn is_tool_activity_item(item: &Value) -> bool {
    codex_thread_item_type(item)
        .map(is_codex_structured_activity_item_type)
        .unwrap_or(false)
}

fn maybe_emit_agent_message_separator(
    next_item_id: Option<&str>,
    current_item_id: &mut Option<String>,
    current_item_has_text: &mut bool,
    response_parts: &mut Vec<String>,
    on_chunk: &(dyn Fn(StreamEvent) + Send + Sync),
) {
    let Some(next_item_id) = next_item_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return;
    };

    let switched_items = current_item_id
        .as_deref()
        .map(|current| current != next_item_id)
        .unwrap_or(false);

    if switched_items && *current_item_has_text {
        let separator = "\n\n".to_owned();
        response_parts.push(separator.clone());
        on_chunk(StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        });
    }

    if current_item_id.as_deref() != Some(next_item_id) {
        *current_item_id = Some(next_item_id.to_owned());
        *current_item_has_text = false;
    }
}

fn emit_tool_stream_event(
    message: &ProviderMessage,
    on_chunk: &(dyn Fn(StreamEvent) + Send + Sync),
) {
    match message.role_str() {
        "tool_use" => on_chunk(StreamEvent::ToolUse {
            message: message.clone(),
        }),
        "tool_result" => on_chunk(StreamEvent::ToolResult {
            message: message.clone(),
        }),
        _ => {}
    }
}

/// Build `ThreadStartParams` from `CodexAppServerConfig`.
fn build_thread_start_params(
    config: &CodexAppServerConfig,
    workspace_dir_override: Option<&str>,
    thread_id: &str,
    run_id: &str,
    metadata: &HashMap<String, Value>,
) -> ThreadStartParams {
    let cwd = workspace_dir_override
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            config
                .workspace_dir
                .as_ref()
                .filter(|d| !d.is_empty())
                .cloned()
        });
    let model = metadata
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            if !config.model.is_empty() {
                Some(config.model.clone())
            } else if !config.default_model.is_empty() {
                Some(config.default_model.clone())
            } else {
                None
            }
        });
    let model_reasoning_effort = config
        .model_reasoning_effort
        .trim()
        .is_empty()
        .then_some(None)
        .unwrap_or_else(|| Some(config.model_reasoning_effort.clone()));

    ThreadStartParams {
        cwd: cwd.clone(),
        config: build_codex_thread_config(
            config,
            metadata,
            thread_id,
            run_id,
            cwd.as_deref().map(Path::new),
        ),
        model,
        model_reasoning_effort,
        approval_policy: if config.approval_policy.is_empty() {
            None
        } else {
            Some(config.approval_policy.clone())
        },
        sandbox: if config.sandbox_mode.is_empty() {
            None
        } else {
            Some(config.sandbox_mode.clone())
        },
    }
}

/// Map a `CodexError` into a `BridgeError`.
fn map_codex_error(context: &str, e: CodexError) -> BridgeError {
    BridgeError::RunFailed(format!("{context}: {e}"))
}

fn resolve_existing_thread_id(
    session_map: &HashMap<String, String>,
    thread_id: &str,
    sdk_session_id: Option<&str>,
) -> Option<String> {
    session_map
        .get(thread_id)
        .cloned()
        .or_else(|| sdk_session_id.map(ToOwned::to_owned))
}

async fn resume_or_start_thread<Resume, ResumeFut, Start, StartFut>(
    existing_thread_id: Option<String>,
    thread_params: ThreadStartParams,
    mut resume: Resume,
    mut start: Start,
) -> Result<String, BridgeError>
where
    Resume: FnMut(ThreadResumeParams) -> ResumeFut,
    ResumeFut: Future<Output = Result<String, CodexError>>,
    Start: FnMut(ThreadStartParams) -> StartFut,
    StartFut: Future<Output = Result<String, CodexError>>,
{
    if let Some(existing_thread_id) = existing_thread_id {
        let resume_params = ThreadResumeParams {
            thread_id: existing_thread_id.clone(),
            cwd: thread_params.cwd.clone(),
            config: thread_params.config.clone(),
            model: thread_params.model.clone(),
            model_reasoning_effort: thread_params.model_reasoning_effort.clone(),
            approval_policy: thread_params.approval_policy.clone(),
            sandbox: thread_params.sandbox.clone(),
        };

        match resume(resume_params).await {
            Ok(thread_id) => return Ok(thread_id),
            Err(error) => {
                tracing::warn!(
                    thread_id = %existing_thread_id,
                    error = %error,
                    "codex resume failed, starting new thread"
                );
            }
        }
    }

    start(thread_params)
        .await
        .map_err(|e| map_codex_error("thread/start failed", e))
}

// ---------------------------------------------------------------------------
// CodexAgentProvider
// ---------------------------------------------------------------------------

/// Agent provider backed by `codex app-server` via `codex_sdk::CodexClient`.
pub struct CodexAgentProvider {
    config: CodexAppServerConfig,
    clients: CodexClientMap,
    /// Maps Garyx thread IDs to codex thread IDs.
    session_map: Mutex<HashMap<String, String>>,
    /// run_id -> active Codex thread/turn record.
    active_runs: Mutex<HashMap<String, ActiveCodexRun>>,
    /// thread_id -> (codex_thread_id, turn_id, run_id)
    active_session_turns: Mutex<HashMap<String, (String, String, String)>>,
    /// thread_id -> (run_id, live callback)
    active_session_callbacks: Mutex<HashMap<String, ActiveSessionCallback>>,
    /// thread_id -> (run_id, pending userMessage markers waiting for Codex item events)
    active_session_pending_acks: Mutex<HashMap<String, PendingCodexAcks>>,
    ready: Mutex<bool>,
}

type CodexClientMap = Arc<Mutex<HashMap<String, Arc<CodexClientSlot>>>>;
type ActiveSessionCallback = (String, Arc<dyn Fn(StreamEvent) + Send + Sync>);
type PendingCodexAcks = (String, VecDeque<PendingCodexAckMarker>);

struct CodexClientSlot {
    client: Mutex<CodexClient>,
    env: HashMap<String, String>,
    active_runs: AtomicUsize,
    last_used: Mutex<Instant>,
}

#[derive(Debug, Clone)]
struct ActiveCodexRun {
    garyx_thread_id: String,
    codex_thread_id: String,
    turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingCodexAckMarker {
    RootUserMessage,
    QueuedInput(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexClientReuseDecision {
    Reuse,
    ReplaceIdle,
}

fn decide_codex_client_reuse(
    existing_env: &HashMap<String, String>,
    desired_env: &HashMap<String, String>,
    active_run_count: usize,
) -> CodexClientReuseDecision {
    if existing_env == desired_env || active_run_count > 0 {
        CodexClientReuseDecision::Reuse
    } else {
        CodexClientReuseDecision::ReplaceIdle
    }
}

impl CodexClientSlot {
    fn new(client: CodexClient, env: HashMap<String, String>) -> Self {
        Self {
            client: Mutex::new(client),
            env,
            active_runs: AtomicUsize::new(0),
            last_used: Mutex::new(Instant::now()),
        }
    }

    fn active_run_count(&self) -> usize {
        self.active_runs.load(Ordering::SeqCst)
    }

    fn begin_run(&self) {
        self.active_runs.fetch_add(1, Ordering::SeqCst);
    }

    async fn finish_run(&self) {
        let _ = self
            .active_runs
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                Some(count.saturating_sub(1))
            });
        self.mark_used().await;
    }

    async fn mark_used(&self) {
        *self.last_used.lock().await = Instant::now();
    }

    async fn shutdown(&self) {
        self.client.lock().await.shutdown().await;
    }
}

fn schedule_idle_client_cleanup(
    clients: CodexClientMap,
    garyx_thread_id: String,
    slot: Arc<CodexClientSlot>,
    ttl: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(ttl).await;

        if slot.active_run_count() > 0 {
            return;
        }

        let last_used = *slot.last_used.lock().await;
        if last_used.elapsed() < ttl {
            return;
        }

        let removed = {
            let mut clients = clients.lock().await;
            if clients.get(&garyx_thread_id).is_some_and(|current| {
                Arc::ptr_eq(current, &slot) && current.active_run_count() == 0
            }) {
                clients.remove(&garyx_thread_id)
            } else {
                None
            }
        };

        if let Some(slot) = removed {
            tracing::info!(
                garyx_thread_id = %garyx_thread_id,
                idle_ttl_secs = ttl.as_secs(),
                "shutting down idle codex app-server"
            );
            slot.shutdown().await;
        }
    });
}

impl CodexAgentProvider {
    /// Create a new Codex provider with the given config.
    pub fn new(config: CodexAppServerConfig) -> Self {
        Self {
            config,
            clients: Arc::new(Mutex::new(HashMap::new())),
            session_map: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            active_session_turns: Mutex::new(HashMap::new()),
            active_session_callbacks: Mutex::new(HashMap::new()),
            active_session_pending_acks: Mutex::new(HashMap::new()),
            ready: Mutex::new(false),
        }
    }

    fn build_client_config(&self, env: HashMap<String, String>) -> CodexClientConfig {
        let codex_bin = if self.config.codex_bin.is_empty() {
            "codex".to_owned()
        } else {
            self.config.codex_bin.clone()
        };

        let model = if !self.config.model.is_empty() {
            Some(self.config.model.clone())
        } else if !self.config.default_model.is_empty() {
            Some(self.config.default_model.clone())
        } else {
            None
        };

        CodexClientConfig {
            codex_bin,
            workspace_dir: self.config.workspace_dir.clone(),
            model,
            approval_policy: self.config.approval_policy.clone(),
            sandbox_mode: self.config.sandbox_mode.clone(),
            experimental_api: self.config.experimental_api,
            request_timeout: Duration::from_secs_f64(self.config.request_timeout_seconds),
            startup_timeout: Duration::from_secs_f64(self.config.startup_timeout_seconds),
            env,
            ..CodexClientConfig::default()
        }
    }

    async fn create_client_slot(
        &self,
        env: HashMap<String, String>,
    ) -> Result<Arc<CodexClientSlot>, BridgeError> {
        let mut client = CodexClient::new(self.build_client_config(env.clone()));
        client
            .initialize()
            .await
            .map_err(|e| BridgeError::Internal(format!("codex client init failed: {e}")))?;

        Ok(Arc::new(CodexClientSlot::new(client, env)))
    }

    async fn client_for_options(
        &self,
        options: &ProviderRunOptions,
    ) -> Result<Arc<CodexClientSlot>, BridgeError> {
        let desired_env = resolve_runtime_codex_env(&self.config, &options.metadata);
        let garyx_thread_id = options.thread_id.clone();

        loop {
            let existing = self.clients.lock().await.get(&garyx_thread_id).cloned();
            if let Some(slot) = existing {
                match decide_codex_client_reuse(&slot.env, &desired_env, slot.active_run_count()) {
                    CodexClientReuseDecision::Reuse => {
                        slot.mark_used().await;
                        return Ok(slot);
                    }
                    CodexClientReuseDecision::ReplaceIdle => {
                        let removed = {
                            let mut clients = self.clients.lock().await;
                            if clients.get(&garyx_thread_id).is_some_and(|current| {
                                Arc::ptr_eq(current, &slot) && current.active_run_count() == 0
                            }) {
                                clients.remove(&garyx_thread_id)
                            } else {
                                None
                            }
                        };
                        if let Some(old_slot) = removed {
                            tracing::info!(
                                garyx_thread_id = %garyx_thread_id,
                                "restarting idle codex app-server because startup env changed"
                            );
                            old_slot.shutdown().await;
                        }
                        continue;
                    }
                }
            }

            let new_slot = self.create_client_slot(desired_env.clone()).await?;
            let mut clients = self.clients.lock().await;
            if clients.contains_key(&garyx_thread_id) {
                drop(clients);
                new_slot.shutdown().await;
                continue;
            }
            clients.insert(garyx_thread_id.clone(), new_slot.clone());
            return Ok(new_slot);
        }
    }

    async fn client_for_thread(&self, garyx_thread_id: &str) -> Option<Arc<CodexClientSlot>> {
        self.clients.lock().await.get(garyx_thread_id).cloned()
    }

    async fn finish_client_run(&self, garyx_thread_id: &str, slot: Arc<CodexClientSlot>) {
        slot.finish_run().await;
        schedule_idle_client_cleanup(
            self.clients.clone(),
            garyx_thread_id.to_owned(),
            slot,
            CODEX_CLIENT_IDLE_TTL,
        );
    }

    async fn shutdown_thread_client(&self, garyx_thread_id: &str) {
        let slot = self.clients.lock().await.remove(garyx_thread_id);
        if let Some(slot) = slot {
            slot.shutdown().await;
        }
    }

    async fn cleanup_active_run_state(&self, run_id: &str) {
        self.active_runs.lock().await.remove(run_id);

        let thread_ids: Vec<String> = {
            let turns = self.active_session_turns.lock().await;
            turns
                .iter()
                .filter(|(_, (_, _, active_run_id))| active_run_id == run_id)
                .map(|(thread_id, _)| thread_id.clone())
                .collect()
        };

        let mut pending_acks = self.active_session_pending_acks.lock().await;
        pending_acks.retain(|_, (active_run_id, _)| active_run_id != run_id);
        drop(pending_acks);

        if thread_ids.is_empty() {
            return;
        }

        {
            let mut turns = self.active_session_turns.lock().await;
            for thread_id in &thread_ids {
                turns.remove(thread_id);
            }
        }

        let mut callbacks = self.active_session_callbacks.lock().await;
        for thread_id in thread_ids {
            let should_remove = callbacks
                .get(&thread_id)
                .map(|(active_run_id, _)| active_run_id == run_id)
                .unwrap_or(false);
            if should_remove {
                callbacks.remove(&thread_id);
            }
        }
    }

    async fn enqueue_streaming_input_ack(
        &self,
        garyx_thread_id: &str,
        run_id: &str,
        pending_input_id: Option<String>,
    ) -> bool {
        let Some(pending_input_id) = pending_input_id
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
        else {
            return false;
        };

        let mut pending_acks = self.active_session_pending_acks.lock().await;
        let entry = pending_acks
            .entry(garyx_thread_id.to_owned())
            .or_insert_with(|| (run_id.to_owned(), VecDeque::new()));
        if entry.0 != run_id {
            *entry = (run_id.to_owned(), VecDeque::new());
        }
        entry
            .1
            .push_back(PendingCodexAckMarker::QueuedInput(pending_input_id));
        true
    }

    async fn rollback_streaming_input_ack(
        &self,
        garyx_thread_id: &str,
        run_id: &str,
        pending_input_id: Option<&str>,
    ) {
        let Some(pending_input_id) = pending_input_id.map(str::trim).filter(|id| !id.is_empty())
        else {
            return;
        };

        let mut pending_acks = self.active_session_pending_acks.lock().await;
        if let Some((active_run_id, queue)) = pending_acks.get_mut(garyx_thread_id) {
            if active_run_id == run_id {
                if let Some(index) = queue.iter().position(|marker| {
                    matches!(marker, PendingCodexAckMarker::QueuedInput(id) if id == pending_input_id)
                }) {
                    queue.remove(index);
                }
            }
        }
    }

    async fn emit_streaming_input_ack_boundary(
        &self,
        garyx_thread_id: &str,
        run_id: &str,
        pending_input_id: Option<String>,
    ) -> bool {
        let callback = {
            self.active_session_callbacks
                .lock()
                .await
                .get(garyx_thread_id)
                .and_then(|(active_run_id, callback)| {
                    if active_run_id == run_id {
                        Some(callback.clone())
                    } else {
                        None
                    }
                })
        };
        if let Some(callback) = callback {
            callback(StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id,
            });
            true
        } else {
            false
        }
    }

    async fn acknowledge_next_codex_user_message(
        &self,
        garyx_thread_id: &str,
        run_id: &str,
    ) -> bool {
        let marker = {
            let mut pending_acks = self.active_session_pending_acks.lock().await;
            let next = pending_acks
                .get_mut(garyx_thread_id)
                .and_then(|(active_run_id, queue)| {
                    if active_run_id == run_id {
                        queue.pop_front()
                    } else {
                        None
                    }
                });
            if pending_acks
                .get(garyx_thread_id)
                .is_some_and(|(active_run_id, queue)| active_run_id == run_id && queue.is_empty())
            {
                pending_acks.remove(garyx_thread_id);
            }
            next
        };

        match marker {
            Some(PendingCodexAckMarker::QueuedInput(pending_input_id)) => {
                self.emit_streaming_input_ack_boundary(
                    garyx_thread_id,
                    run_id,
                    Some(pending_input_id),
                )
                .await
            }
            Some(PendingCodexAckMarker::RootUserMessage) | None => false,
        }
    }

    /// Core streaming run implementation.
    async fn run_streaming_impl(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
        client_slot: Arc<CodexClientSlot>,
    ) -> Result<ProviderRunResult, BridgeError> {
        let client_guard = client_slot.client.lock().await;
        let client = &*client_guard;
        let live_callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(on_chunk);

        let run_id = options
            .metadata
            .get("bridge_run_id")
            .and_then(|v| v.as_str())
            .or_else(|| {
                options
                    .metadata
                    .get("client_run_id")
                    .and_then(|v| v.as_str())
            })
            .or_else(|| options.metadata.get("run_id").and_then(|v| v.as_str()))
            .map(|s| s.to_owned())
            .unwrap_or_else(|| {
                format!(
                    "run_{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                )
            });

        let start = Instant::now();
        let actual_model = resolve_codex_actual_model(&self.config, &options.metadata);
        let mut response_parts: Vec<String> = Vec::new();
        let mut session_messages: Vec<ProviderMessage> = Vec::new();
        let mut notification_rx = client.subscribe_events();

        // Resolve or create thread
        let sdk_session_id = options
            .metadata
            .get("sdk_session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());

        let existing_thread_id = {
            let session_map = self.session_map.lock().await;
            resolve_existing_thread_id(&session_map, &options.thread_id, sdk_session_id.as_deref())
        };
        let include_memory = existing_thread_id.is_none();
        let thread_params = build_thread_start_params(
            &self.config,
            options.workspace_dir.as_deref(),
            &options.thread_id,
            &run_id,
            &options.metadata,
        );
        let thread_id = resume_or_start_thread(
            existing_thread_id,
            thread_params,
            |params| client.resume_thread(params),
            |params| client.start_thread(params),
        )
        .await?;
        self.session_map
            .lock()
            .await
            .insert(options.thread_id.clone(), thread_id.clone());

        // Start turn
        let input_items = build_input_items(options, include_memory);
        let turn_id = client
            .start_turn(&thread_id, input_items)
            .await
            .map_err(|e| map_codex_error("turn/start failed", e))?;

        // Track active run
        {
            self.active_runs.lock().await.insert(
                run_id.clone(),
                ActiveCodexRun {
                    garyx_thread_id: options.thread_id.clone(),
                    codex_thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                },
            );
            self.active_session_turns.lock().await.insert(
                options.thread_id.clone(),
                (thread_id.clone(), turn_id.clone(), run_id.clone()),
            );
            self.active_session_callbacks.lock().await.insert(
                options.thread_id.clone(),
                (run_id.clone(), live_callback.clone()),
            );
            self.active_session_pending_acks.lock().await.insert(
                options.thread_id.clone(),
                (
                    run_id.clone(),
                    VecDeque::from([PendingCodexAckMarker::RootUserMessage]),
                ),
            );
        }

        // Drop the client lock before entering notification loop
        drop(client_guard);

        // Notification loop
        let mut completed_turn: Option<Value> = None;
        let mut streamed_error_message: Option<String> = None;
        let mut current_agent_message_item_id: Option<String> = None;
        let mut current_agent_message_has_text = false;

        let timeout = Duration::from_secs_f64(self.config.request_timeout_seconds);

        let loop_result: Result<(), BridgeError> = async {
            loop {
                let notification: JsonRpcNotification =
                    tokio::time::timeout(timeout, notification_rx.recv())
                        .await
                        .map_err(|_| BridgeError::Timeout)?
                        .map_err(|e| {
                            BridgeError::RunFailed(format!("notification channel error: {e}"))
                        })?;

                let method = &notification.method;
                let params = &notification.params;

                // Fatal transport error
                if method == "transport/fatal" {
                    let error_msg = params
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("codex transport fatal error")
                        .to_owned();
                    return Err(BridgeError::RunFailed(error_msg));
                }

                if !matches_turn(params, &thread_id, &turn_id) {
                    continue;
                }

                match method.as_str() {
                    "item/agentMessage/delta" => {
                        maybe_emit_agent_message_separator(
                            params.get("itemId").and_then(|v| v.as_str()),
                            &mut current_agent_message_item_id,
                            &mut current_agent_message_has_text,
                            &mut response_parts,
                            live_callback.as_ref(),
                        );

                        let delta = params.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                        if !delta.is_empty() {
                            response_parts.push(delta.to_owned());
                            append_codex_assistant_session_message(
                                &mut session_messages,
                                params.get("itemId").and_then(|v| v.as_str()),
                                delta,
                            );
                            live_callback(StreamEvent::Delta {
                                text: delta.to_owned(),
                            });
                            current_agent_message_has_text = true;
                        }
                    }
                    "item/started" => {
                        if let Some(item) = params.get("item") {
                            if is_user_message_item(item) {
                                // `turn/steer` only means the input reached the active turn.
                                // Codex confirms consumption by replaying it as a userMessage item.
                                self.acknowledge_next_codex_user_message(
                                    &options.thread_id,
                                    &run_id,
                                )
                                .await;
                            }
                            if is_agent_message_item(item) {
                                maybe_emit_agent_message_separator(
                                    item.get("id").and_then(|v| v.as_str()),
                                    &mut current_agent_message_item_id,
                                    &mut current_agent_message_has_text,
                                    &mut response_parts,
                                    live_callback.as_ref(),
                                );
                            }
                            if let Some(msg) = build_tool_session_message(item, false) {
                                emit_tool_stream_event(&msg, live_callback.as_ref());
                                session_messages.push(msg);
                            }
                        }
                    }
                    "item/completed" => {
                        if let Some(item) = params.get("item") {
                            if let Some(msg) = build_tool_session_message(item, true) {
                                emit_tool_stream_event(&msg, live_callback.as_ref());
                                session_messages.push(msg);
                            }
                        }
                    }
                    "error" => {
                        if let Some(err_obj) = params.get("error") {
                            streamed_error_message = Some(
                                err_obj
                                    .get("message")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("codex turn error")
                                    .to_owned(),
                            );
                        }
                    }
                    "turn/completed" => {
                        let turn = params
                            .get("turn")
                            .cloned()
                            .unwrap_or(Value::Object(serde_json::Map::new()));
                        completed_turn = Some(turn);
                        break;
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        .await;

        // Cleanup tracking
        self.cleanup_active_run_state(&run_id).await;

        let duration_ms = start.elapsed().as_millis() as i64;
        let response = response_parts.join("");

        // If the loop errored, return a failure result
        if let Err(e) = loop_result {
            tracing::error!(error = %e, "codex provider run_streaming error");
            return Ok(ProviderRunResult {
                run_id,
                thread_id: options.thread_id.clone(),
                response,
                session_messages,
                sdk_session_id: Some(thread_id),
                actual_model,
                success: false,
                error: Some(e.to_string()),
                input_tokens: 0,
                output_tokens: 0,
                cost: 0.0,
                duration_ms,
            });
        }

        // Build result from completed turn
        let completed = completed_turn.unwrap_or(Value::Object(serde_json::Map::new()));
        let status = completed
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("completed")
            .to_lowercase();
        let success = status != "failed";

        let error = if status == "failed" {
            let from_turn = completed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());
            Some(
                from_turn
                    .or(streamed_error_message)
                    .unwrap_or_else(|| "codex turn failed".to_owned()),
            )
        } else {
            None
        };

        if !success {
            tracing::warn!(
                run_id = %run_id,
                thread_id = %options.thread_id,
                sdk_session_id = %thread_id,
                status = %status,
                error = %error.as_deref().unwrap_or("unknown codex turn failure"),
                "codex turn completed with failure",
            );
        }

        let (input_tokens, output_tokens, cost) = extract_usage(&completed);

        live_callback(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id,
            thread_id: options.thread_id.clone(),
            response,
            session_messages,
            sdk_session_id: Some(thread_id),
            actual_model,
            success,
            error,
            input_tokens,
            output_tokens,
            cost,
            duration_ms,
        })
    }
}

#[async_trait]
impl AgentLoopProvider for CodexAgentProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::CodexAppServer
    }

    fn is_ready(&self) -> bool {
        // Use try_lock to avoid blocking; if lock is held, provider is busy but ready
        self.ready.try_lock().map(|g| *g).unwrap_or(false)
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        if *self.ready.lock().await {
            return Ok(());
        }

        *self.ready.lock().await = true;
        tracing::info!("codex provider initialized; app-server clients are started per thread");
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        tracing::info!("shutting down codex provider");

        let clients: Vec<Arc<CodexClientSlot>> = {
            let mut clients = self.clients.lock().await;
            clients.drain().map(|(_, slot)| slot).collect()
        };
        for client in clients {
            client.shutdown().await;
        }

        self.active_runs.lock().await.clear();
        self.active_session_turns.lock().await.clear();
        self.active_session_callbacks.lock().await.clear();
        self.active_session_pending_acks.lock().await.clear();

        *self.ready.lock().await = false;
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        if !*self.ready.lock().await {
            return Err(BridgeError::ProviderNotReady);
        }
        let client_slot = self.client_for_options(options).await?;
        client_slot.begin_run();
        let result = self
            .run_streaming_impl(options, on_chunk, client_slot.clone())
            .await;
        self.finish_client_run(&options.thread_id, client_slot)
            .await;
        result
    }

    async fn abort(&self, run_id: &str) -> bool {
        let active = self.active_runs.lock().await.get(run_id).cloned();
        let Some(active) = active else {
            self.cleanup_active_run_state(run_id).await;
            return false;
        };

        let Some(client_slot) = self.client_for_thread(&active.garyx_thread_id).await else {
            self.cleanup_active_run_state(run_id).await;
            return false;
        };

        let client_guard = client_slot.client.lock().await;
        // Try interrupt with timeout; force-cleanup on failure
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            client_guard.interrupt_turn(&active.codex_thread_id, &active.turn_id),
        )
        .await;

        match result {
            Ok(Ok(())) => {
                self.cleanup_active_run_state(run_id).await;
                true
            }
            Ok(Err(e)) => {
                tracing::warn!(run_id, error = %e, "codex abort failed");
                self.cleanup_active_run_state(run_id).await;
                false
            }
            Err(_) => {
                tracing::warn!(run_id, "codex abort timed out, force-cleaning up");
                self.cleanup_active_run_state(run_id).await;
                false
            }
        }
    }

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn add_streaming_input(&self, thread_id: &str, input: QueuedUserInput) -> bool {
        let garyx_thread_id = thread_id.to_owned();
        let active = {
            self.active_session_turns
                .lock()
                .await
                .get(&garyx_thread_id)
                .cloned()
        };

        let Some((codex_thread_id, turn_id, run_id)) = active else {
            return false;
        };

        let Some(client_slot) = self.client_for_thread(&garyx_thread_id).await else {
            return false;
        };

        let pending_input_id = input.pending_input_id.clone();
        self.enqueue_streaming_input_ack(&garyx_thread_id, &run_id, pending_input_id.clone())
            .await;
        let input = build_input_items_from_parts(&input.message, &input.images, &input.attachments);

        let client_guard = client_slot.client.lock().await;
        match client_guard
            .steer_turn(&codex_thread_id, &turn_id, input)
            .await
        {
            Ok(()) => {
                tracing::debug!(
                    garyx_thread_id = %garyx_thread_id,
                    codex_thread_id = %codex_thread_id,
                    run_id = %run_id,
                    "steered codex turn with additional input; waiting for userMessage item ack"
                );
                true
            }
            Err(e) => {
                self.rollback_streaming_input_ack(
                    &garyx_thread_id,
                    &run_id,
                    pending_input_id.as_deref(),
                )
                .await;
                tracing::warn!(
                    garyx_thread_id = %garyx_thread_id,
                    codex_thread_id = %codex_thread_id,
                    run_id = %run_id,
                    error = %e,
                    "failed to steer codex turn"
                );
                false
            }
        }
    }

    async fn interrupt_streaming_session(&self, thread_id: &str) -> bool {
        let active = {
            self.active_session_turns
                .lock()
                .await
                .get(thread_id)
                .cloned()
        };

        let Some((_thread_id, _turn_id, run_id)) = active else {
            return false;
        };

        self.abort(&run_id).await
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        let map = self.session_map.lock().await;
        if let Some(existing_thread_id) = map.get(thread_id) {
            return Ok(existing_thread_id.clone());
        }
        // No existing thread - return a placeholder; actual thread creation
        // happens in run() via thread/start.
        Ok(String::new())
    }

    async fn clear_session(&self, thread_id: &str) -> bool {
        self.session_map.lock().await.remove(thread_id);
        self.active_session_turns.lock().await.remove(thread_id);
        self.active_session_callbacks.lock().await.remove(thread_id);
        self.active_session_pending_acks
            .lock()
            .await
            .remove(thread_id);
        self.shutdown_thread_client(thread_id).await;
        true
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
