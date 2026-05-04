use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use garyx_models::provider::{
    GeminiCliConfig, PromptAttachment, ProviderMessage, ProviderMessageRole, ProviderRunOptions,
    ProviderRunResult, ProviderType, StreamEvent, attachments_from_metadata,
    build_prompt_message_with_attachments,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

#[cfg(test)]
use garyx_models::provider::ImagePayload;

use crate::gary_prompt::{
    compose_gary_instructions, prepend_initial_context_to_user_message, task_cli_env,
};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

const ACP_PROTOCOL_VERSION: i64 = 1;
const GEMINI_ACP_ARG: &str = "--acp";
const GEMINI_SKIP_TRUST_ARG: &str = "--skip-trust";
const DEFAULT_REQUEST_TIMEOUT_SECS: f64 = 300.0;
const ACTIVE_TOOL_IDLE_TIMEOUT_SECS: u64 = 900;

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

fn resolve_runtime_gemini_env(
    config: &GeminiCliConfig,
    metadata: &HashMap<String, Value>,
) -> HashMap<String, String> {
    let mut env = config.env.clone();
    env.extend(task_cli_env(metadata));
    env.extend(metadata_string_map(metadata, "desktop_gemini_env"));
    env
}

fn resolve_workspace_dir(
    config: &GeminiCliConfig,
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

fn request_timeout(config: &GeminiCliConfig) -> Duration {
    let timeout = if config.timeout_seconds > 0.0 {
        config.timeout_seconds
    } else {
        DEFAULT_REQUEST_TIMEOUT_SECS
    };
    Duration::from_secs_f64(timeout)
}

fn active_tool_idle_timeout(base_timeout: Duration) -> Duration {
    std::cmp::max(
        base_timeout,
        Duration::from_secs(ACTIVE_TOOL_IDLE_TIMEOUT_SECS),
    )
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn approval_mode(config: &GeminiCliConfig, metadata: &HashMap<String, Value>) -> String {
    let raw = metadata
        .get("approval_mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| config.approval_mode.clone());
    normalize_approval_mode(&raw)
}

fn normalize_approval_mode(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "default" => "default".to_owned(),
        "yolo" => "yolo".to_owned(),
        "plan" => "plan".to_owned(),
        "auto_edit" | "autoedit" => "autoEdit".to_owned(),
        _ => value.trim().to_owned(),
    }
}

fn model_id(config: &GeminiCliConfig, metadata: &HashMap<String, Value>) -> Option<String> {
    normalize_non_empty(metadata.get("model").and_then(Value::as_str))
        .or_else(|| normalize_non_empty(Some(config.model.as_str())))
        .or_else(|| normalize_non_empty(Some(config.default_model.as_str())))
}

fn gemini_bin(config: &GeminiCliConfig) -> &str {
    let trimmed = config.gemini_bin.trim();
    if trimmed.is_empty() {
        "gemini"
    } else {
        trimmed
    }
}

fn garyx_mcp_server(
    config: &GeminiCliConfig,
    thread_id: &str,
    run_id: &str,
    metadata: &HashMap<String, Value>,
) -> Option<Value> {
    let base_url = config.mcp_base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return None;
    }

    let mut headers = HashMap::from([
        ("X-Run-Id".to_owned(), run_id.to_owned()),
        ("X-Thread-Id".to_owned(), thread_id.to_owned()),
        ("X-Session-Key".to_owned(), thread_id.to_owned()),
    ]);
    headers.extend(metadata_string_map(metadata, "garyx_mcp_headers"));

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
        "type": "http",
        "name": "garyx",
        "url": url,
        "headers": header_array(&headers),
    }))
}

fn header_array(headers: &HashMap<String, String>) -> Vec<Value> {
    let mut pairs = headers
        .iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect::<Vec<_>>();
    pairs.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .cmp(&right.get("name").and_then(Value::as_str))
    });
    pairs
}

fn env_array(env: &HashMap<String, String>) -> Vec<Value> {
    let mut pairs = env
        .iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect::<Vec<_>>();
    pairs.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .cmp(&right.get("name").and_then(Value::as_str))
    });
    pairs
}

fn normalize_gemini_mcp_servers(metadata: &HashMap<String, Value>) -> Vec<Value> {
    let Some(servers) = metadata
        .get("remote_mcp_servers")
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };

    let mut normalized = Vec::new();
    for (name, raw_server) in servers {
        if name == "garyx" {
            continue;
        }
        let Some(server) = raw_server.as_object() else {
            continue;
        };
        if matches!(server.get("enabled").and_then(Value::as_bool), Some(false)) {
            continue;
        }

        if let Some(command) = server
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let args = server
                .get("args")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                        .map(Value::String)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let env = server
                .get("env")
                .and_then(Value::as_object)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|(env_name, env_value)| {
                            env_value
                                .as_str()
                                .map(|env_value| (env_name.clone(), env_value.to_owned()))
                        })
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default();
            normalized.push(json!({
                "name": name,
                "command": command,
                "args": args,
                "env": env_array(&env),
            }));
            continue;
        }

        let Some(url) = server
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let headers = server
            .get("headers")
            .and_then(Value::as_object)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|(header_name, header_value)| {
                        header_value
                            .as_str()
                            .map(|header_value| (header_name.clone(), header_value.to_owned()))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let server_type = match server.get("type").and_then(Value::as_str) {
            Some(kind) if kind.eq_ignore_ascii_case("sse") => "sse",
            _ => "http",
        };
        normalized.push(json!({
            "type": server_type,
            "name": name,
            "url": url,
            "headers": header_array(&headers),
        }));
    }

    normalized
}

fn build_mcp_servers(
    config: &GeminiCliConfig,
    thread_id: &str,
    run_id: &str,
    metadata: &HashMap<String, Value>,
) -> Vec<Value> {
    let mut servers = normalize_gemini_mcp_servers(metadata);
    if let Some(server) = garyx_mcp_server(config, thread_id, run_id, metadata) {
        servers.push(server);
    }
    servers
}

fn build_prompt_text_from_parts(
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

fn build_prompt_blocks(
    options: &ProviderRunOptions,
    workspace_dir: Option<&Path>,
    include_instructions: bool,
) -> Vec<Value> {
    let mut blocks = Vec::new();
    let attachments = attachments_from_metadata(&options.metadata);
    let text =
        build_prompt_text_from_parts(options, workspace_dir, include_instructions, &attachments);
    if !text.trim().is_empty()
        || (attachments.is_empty() && options.images.as_deref().unwrap_or_default().is_empty())
    {
        blocks.push(json!({
            "type": "text",
            "text": text,
        }));
    }
    if attachments.is_empty() {
        for image in options.images.as_deref().unwrap_or_default() {
            if image.data.trim().is_empty() {
                continue;
            }
            blocks.push(json!({
                "type": "image",
                "data": image.data,
                "mimeType": image.media_type,
            }));
        }
    }
    blocks
}

fn append_gemini_assistant_session_message(
    session_messages: &mut Vec<ProviderMessage>,
    delta: &str,
) {
    if delta.is_empty() {
        return;
    }
    let can_append = session_messages.last().is_some_and(|message| {
        message.role == ProviderMessageRole::Assistant
            && message.metadata.get("source").and_then(Value::as_str) == Some("gemini_cli")
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

    let entry = ProviderMessage::assistant_text(delta)
        .with_timestamp(chrono::Utc::now().to_rfc3339())
        .with_metadata_value("source", json!("gemini_cli"));
    session_messages.push(entry);
}

fn strip_gemini_thought_output(raw: &str) -> String {
    if !raw.contains("[Thought:") {
        return raw.to_owned();
    }
    let Some(marker_start) = raw.rfind("[Thought:") else {
        return raw.to_owned();
    };
    let suffix = &raw[marker_start..];
    let Some(marker_end) = suffix.find(']') else {
        return raw.to_owned();
    };
    suffix[(marker_end + 1)..]
        .trim_start_matches(|ch: char| ch.is_whitespace())
        .to_owned()
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

fn extract_quoted_topic_title(title: &str) -> Option<String> {
    let trimmed = title.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("update topic to:") {
        return None;
    }
    let after_prefix = trimmed.split_once(':')?.1.trim();
    let unquoted = after_prefix
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(after_prefix);
    Some(normalize_thread_title(unquoted)).filter(|value| !value.is_empty())
}

fn extract_gemini_thread_title(update: &Value) -> Option<String> {
    for path in [
        &["rawInput", "title"][..],
        &["args", "title"][..],
        &["input", "title"][..],
    ] {
        if let Some(title) = path
            .iter()
            .try_fold(update, |value, key| value.get(*key))
            .and_then(Value::as_str)
            .map(normalize_thread_title)
            .filter(|value| !value.is_empty())
        {
            return Some(title);
        }
    }

    update
        .get("title")
        .and_then(Value::as_str)
        .and_then(extract_quoted_topic_title)
}

fn first_non_empty_path_string(value: &Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        path.iter()
            .try_fold(value, |current, key| current.get(*key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn extract_gemini_tool_name(update: &Value) -> Option<String> {
    first_non_empty_path_string(
        update,
        &[
            &["rawInput", "name"],
            &["rawInput", "functionName"],
            &["input", "name"],
            &["input", "functionName"],
            &["toolCall", "name"],
            &["toolCall", "functionName"],
            &["functionCall", "name"],
            &["title"],
            &["kind"],
        ],
    )
}

fn collect_search_outputs(value: &Value, outputs: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(output) = map
                .get("functionResponse")
                .and_then(|item| item.get("response"))
                .and_then(|item| item.get("output"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                outputs.push(output.to_owned());
            }
            for child in map.values() {
                collect_search_outputs(child, outputs);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_search_outputs(item, outputs);
            }
        }
        _ => {}
    }
}

fn strip_url_punctuation(url: &str) -> &str {
    url.trim_matches(|ch: char| matches!(ch, ')' | ']' | '}' | '>' | '.' | ',' | ';' | ':'))
}

fn extract_urls_from_text(text: &str) -> Vec<Value> {
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    for raw in text.split_whitespace() {
        let Some(start) = raw.find("http://").or_else(|| raw.find("https://")) else {
            continue;
        };
        let url = strip_url_punctuation(&raw[start..]);
        if url.is_empty() || !seen.insert(url.to_owned()) {
            continue;
        }
        sources.push(json!({ "url": url }));
    }
    sources
}

fn extract_markdown_sources(text: &str) -> Vec<Value> {
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    let mut remainder = text;
    while let Some(open) = remainder.find('[') {
        let after_open = &remainder[open + 1..];
        let Some(close) = after_open.find("](") else {
            remainder = after_open;
            continue;
        };
        let title = after_open[..close].trim();
        let after_url = &after_open[close + 2..];
        let Some(end) = after_url.find(')') else {
            break;
        };
        let url = strip_url_punctuation(after_url[..end].trim());
        if (url.starts_with("http://") || url.starts_with("https://"))
            && seen.insert(url.to_owned())
        {
            sources.push(json!({
                "title": title,
                "url": url,
            }));
        }
        remainder = &after_url[end + 1..];
    }
    for source in extract_urls_from_text(text) {
        let Some(url) = source.get("url").and_then(Value::as_str) else {
            continue;
        };
        if seen.insert(url.to_owned()) {
            sources.push(source);
        }
    }
    sources
}

fn gemini_search_metadata(update: &Value, tool_name: Option<&str>) -> Option<Value> {
    let mut outputs = Vec::new();
    collect_search_outputs(update, &mut outputs);
    let search_like = match tool_name {
        Some(name) => {
            let lower = name.to_ascii_lowercase();
            lower.contains("google_web_search")
                || lower.contains("web_search")
                || lower.contains("google search")
                || lower.contains("search")
        }
        None => !outputs.is_empty(),
    };
    if !search_like {
        return None;
    }
    let output = outputs
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n\n");
    let sources = extract_markdown_sources(&output);
    Some(json!({
        "provider": "gemini_cli",
        "tool_name": tool_name,
        "output": output,
        "sources": sources,
    }))
}

fn tool_message(update: &Value, completed: bool) -> ProviderMessage {
    let tool_use_id = update
        .get("toolCallId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let tool_name = extract_gemini_tool_name(update);
    let search_metadata = completed
        .then(|| gemini_search_metadata(update, tool_name.as_deref()))
        .flatten();
    let mut message = if completed {
        let is_error = update
            .get("status")
            .and_then(Value::as_str)
            .map(|status| status.eq_ignore_ascii_case("failed"));
        ProviderMessage::tool_result(update.clone(), tool_use_id, tool_name, is_error)
    } else {
        ProviderMessage::tool_use(update.clone(), tool_use_id, tool_name)
    };
    message = message
        .with_timestamp(chrono::Utc::now().to_rfc3339())
        .with_metadata_value("source", json!("gemini_cli"));
    if let Some(search_metadata) = search_metadata {
        message = message.with_metadata_value("gemini_search", search_metadata);
    }
    message
}

fn tool_call_id(update: &Value) -> Option<&str> {
    update
        .get("toolCallId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_invalid_session_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("invalid session identifier")
        || lower.contains("session not found")
        || lower.contains("use --list-sessions")
}

async fn read_json_message(
    lines: &mut Lines<BufReader<ChildStdout>>,
    timeout: Duration,
) -> Result<Option<Value>, BridgeError> {
    let next = tokio::time::timeout(timeout, lines.next_line())
        .await
        .map_err(|_| BridgeError::Timeout)?
        .map_err(|error| BridgeError::RunFailed(format!("gemini stdout read failed: {error}")))?;
    let Some(line) = next else {
        return Ok(None);
    };
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(Some(Value::Null));
    }
    serde_json::from_str(trimmed)
        .map(Some)
        .map_err(|error| BridgeError::RunFailed(format!("invalid gemini ACP json: {error}")))
}

async fn send_json_request(
    stdin: &mut ChildStdin,
    id: u64,
    method: &str,
    params: Value,
) -> Result<(), BridgeError> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let mut serialized = serde_json::to_vec(&payload).map_err(|error| {
        BridgeError::Internal(format!("serialize gemini ACP request failed: {error}"))
    })?;
    serialized.push(b'\n');
    stdin
        .write_all(&serialized)
        .await
        .map_err(|error| BridgeError::RunFailed(format!("gemini stdin write failed: {error}")))?;
    stdin
        .flush()
        .await
        .map_err(|error| BridgeError::RunFailed(format!("gemini stdin flush failed: {error}")))
}

async fn read_until_response(
    lines: &mut Lines<BufReader<ChildStdout>>,
    id: u64,
    timeout: Duration,
) -> Result<Value, BridgeError> {
    loop {
        let Some(message) = read_json_message(lines, timeout).await? else {
            return Err(BridgeError::RunFailed(format!(
                "gemini ACP closed before responding to request {id}"
            )));
        };
        if message.is_null() {
            continue;
        }
        if message.get("id").and_then(Value::as_u64) == Some(id) {
            return Ok(message);
        }
    }
}

fn jsonrpc_error_message(message: &Value) -> Option<String> {
    let error = message.get("error")?;
    let base = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("ACP error");
    let detail = error.get("data").map(|value| match value {
        Value::Array(items) => items
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("; "),
        Value::Object(_) => value.to_string(),
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    });
    Some(match detail {
        Some(detail) if !detail.trim().is_empty() => format!("{base}: {detail}"),
        _ => base.to_owned(),
    })
}

fn append_stderr(message: impl Into<String>, stderr_output: &str) -> String {
    let message = message.into();
    let stderr_output = stderr_output.trim();
    if stderr_output.is_empty() {
        message
    } else {
        format!("{message} | stderr: {stderr_output}")
    }
}

fn extract_prompt_result_usage(message: &Value) -> (i64, i64) {
    let result = message.get("result");
    let usage = result.and_then(|value| value.get("usage"));
    if let Some(usage) = usage {
        return (
            usage
                .get("inputTokens")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            usage
                .get("outputTokens")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        );
    }

    let token_count = result
        .and_then(|value| value.get("_meta"))
        .and_then(|value| value.get("quota"))
        .and_then(|value| value.get("token_count"));
    (
        token_count
            .and_then(|value| value.get("input_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        token_count
            .and_then(|value| value.get("output_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
    )
}

fn extract_prompt_result_actual_model(message: &Value) -> Option<String> {
    message
        .get("result")
        .and_then(|value| value.get("_meta"))
        .and_then(|value| value.get("quota"))
        .and_then(|value| value.get("model_usage"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find_map(|item| {
                item.get("model")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            })
        })
}

fn resolve_session_id_from_response(
    session_response: &Value,
    requested_session_id: Option<&str>,
) -> Result<String, BridgeError> {
    session_response
        .get("result")
        .and_then(|value| value.get("sessionId"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| normalize_non_empty(requested_session_id))
        .ok_or_else(|| {
            BridgeError::RunFailed("gemini session response missing sessionId".to_owned())
        })
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

pub struct GeminiCliProvider {
    config: GeminiCliConfig,
    session_map: Mutex<HashMap<String, String>>,
    active_runs: Mutex<HashMap<String, Arc<Mutex<Child>>>>,
    run_session_map: Mutex<HashMap<String, String>>,
    ready: bool,
}

impl GeminiCliProvider {
    pub fn new(config: GeminiCliConfig) -> Self {
        Self {
            config,
            session_map: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            run_session_map: Mutex::new(HashMap::new()),
            ready: false,
        }
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
        run_id: &str,
        child: Option<Arc<Mutex<Child>>>,
        stderr_task: tokio::task::JoinHandle<String>,
    ) -> String {
        tokio::time::timeout(Duration::from_secs(2), async move {
            if let Some(child) = child {
                let mut child = child.lock().await;
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            tracing::debug!(run_id = %run_id, "cleaned up gemini ACP process");
            stderr_task.await.unwrap_or_default()
        })
        .await
        .unwrap_or_default()
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
            BridgeError::RunFailed("gemini workspace directory is unavailable".to_owned())
        })?;
        let timeout = request_timeout(&self.config);
        let active_tool_timeout = active_tool_idle_timeout(timeout);
        let mut command = Command::new(gemini_bin(&self.config));
        command.arg(GEMINI_ACP_ARG);
        command.arg(GEMINI_SKIP_TRUST_ARG);
        command.current_dir(cwd);
        command.stdin(std::process::Stdio::piped());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);
        command.envs(resolve_runtime_gemini_env(&self.config, &options.metadata));

        let mut child = command.spawn().map_err(|error| {
            BridgeError::Internal(format!("failed to spawn gemini CLI: {error}"))
        })?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| BridgeError::Internal("gemini stdin unavailable".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BridgeError::Internal("gemini stdout unavailable".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BridgeError::Internal("gemini stderr unavailable".to_owned()))?;

        let stderr_task = tokio::spawn(read_stream_to_string(stderr));
        let child = Arc::new(Mutex::new(child));
        self.register_run(run_id, &options.thread_id, child.clone())
            .await;
        let mut lines = BufReader::new(stdout).lines();
        let started = Instant::now();
        let mut next_request_id = 1_u64;

        send_json_request(
            &mut stdin,
            next_request_id,
            "initialize",
            json!({
                "protocolVersion": ACP_PROTOCOL_VERSION,
                "clientCapabilities": {},
                "clientInfo": {
                    "name": "garyx",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        )
        .await?;
        let initialize = match read_until_response(&mut lines, next_request_id, timeout).await {
            Ok(response) => response,
            Err(error) => {
                drop(stdin);
                let (child, _) = self.unregister_run(run_id).await;
                let stderr_output = self.cleanup_run_io(run_id, child, stderr_task).await;
                return Err(match error {
                    BridgeError::RunFailed(message) => {
                        BridgeError::RunFailed(append_stderr(message, &stderr_output))
                    }
                    other => other,
                });
            }
        };
        next_request_id += 1;
        if let Some(error) = jsonrpc_error_message(&initialize) {
            drop(stdin);
            let (child, _) = self.unregister_run(run_id).await;
            let stderr_output = self.cleanup_run_io(run_id, child, stderr_task).await;
            return Err(BridgeError::RunFailed(format!(
                "gemini initialize failed: {}",
                append_stderr(error, &stderr_output)
            )));
        }

        let mcp_servers =
            build_mcp_servers(&self.config, &options.thread_id, run_id, &options.metadata);
        let session_request = if let Some(session_id) = session_id {
            (
                "session/load",
                json!({
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy().to_string(),
                    "mcpServers": mcp_servers,
                }),
            )
        } else {
            (
                "session/new",
                json!({
                    "cwd": cwd.to_string_lossy().to_string(),
                    "mcpServers": mcp_servers,
                }),
            )
        };

        send_json_request(
            &mut stdin,
            next_request_id,
            session_request.0,
            session_request.1,
        )
        .await?;
        let session_response = read_until_response(&mut lines, next_request_id, timeout).await?;
        next_request_id += 1;
        if let Some(error) = jsonrpc_error_message(&session_response) {
            drop(stdin);
            let (child, _) = self.unregister_run(run_id).await;
            let stderr_output = self.cleanup_run_io(run_id, child, stderr_task).await;
            return Err(BridgeError::SessionError(format!(
                "{error}{}",
                if stderr_output.is_empty() {
                    String::new()
                } else {
                    format!(" | stderr: {stderr_output}")
                }
            )));
        }

        let resolved_session_id = resolve_session_id_from_response(&session_response, session_id)?;
        self.session_map
            .lock()
            .await
            .insert(options.thread_id.clone(), resolved_session_id.clone());

        let mode_id = approval_mode(&self.config, &options.metadata);
        if mode_id != "default" {
            send_json_request(
                &mut stdin,
                next_request_id,
                "session/set_mode",
                json!({
                    "sessionId": resolved_session_id,
                    "modeId": mode_id,
                }),
            )
            .await?;
            let mode_response = read_until_response(&mut lines, next_request_id, timeout).await?;
            next_request_id += 1;
            if let Some(error) = jsonrpc_error_message(&mode_response) {
                drop(stdin);
                let (child, _) = self.unregister_run(run_id).await;
                let stderr_output = self.cleanup_run_io(run_id, child, stderr_task).await;
                return Err(BridgeError::RunFailed(format!(
                    "gemini set_mode failed: {error}{}",
                    if stderr_output.is_empty() {
                        String::new()
                    } else {
                        format!(" | stderr: {stderr_output}")
                    }
                )));
            }
        }

        if let Some(model_id) = model_id(&self.config, &options.metadata) {
            send_json_request(
                &mut stdin,
                next_request_id,
                "session/set_model",
                json!({
                    "sessionId": resolved_session_id,
                    "modelId": model_id,
                }),
            )
            .await?;
            let model_response = read_until_response(&mut lines, next_request_id, timeout).await?;
            next_request_id += 1;
            if let Some(error) = jsonrpc_error_message(&model_response) {
                drop(stdin);
                let (child, _) = self.unregister_run(run_id).await;
                let stderr_output = self.cleanup_run_io(run_id, child, stderr_task).await;
                return Err(BridgeError::RunFailed(format!(
                    "gemini set_model failed: {error}{}",
                    if stderr_output.is_empty() {
                        String::new()
                    } else {
                        format!(" | stderr: {stderr_output}")
                    }
                )));
            }
        }

        let prompt_blocks = build_prompt_blocks(options, Some(cwd.as_path()), session_id.is_none());
        send_json_request(
            &mut stdin,
            next_request_id,
            "session/prompt",
            json!({
                "sessionId": resolved_session_id,
                "prompt": prompt_blocks,
            }),
        )
        .await?;

        let mut response = String::new();
        let mut session_messages = Vec::new();
        let mut input_tokens = 0_i64;
        let mut output_tokens = 0_i64;
        let mut cost = 0.0_f64;
        let mut actual_model = None;
        let mut success = true;
        let mut error = None;
        let mut active_tool_calls = HashSet::<String>::new();
        let mut has_unkeyed_tool_call = false;
        let mut thread_title: Option<String> = None;

        loop {
            let idle_timeout = if has_unkeyed_tool_call || !active_tool_calls.is_empty() {
                active_tool_timeout
            } else {
                timeout
            };
            let Some(message) = read_json_message(&mut lines, idle_timeout).await? else {
                success = false;
                error = Some("gemini ACP closed before prompt completed".to_owned());
                break;
            };
            if message.is_null() {
                continue;
            }

            if message.get("id").and_then(Value::as_u64) == Some(next_request_id) {
                if let Some(jsonrpc_error) = jsonrpc_error_message(&message) {
                    success = false;
                    error = Some(jsonrpc_error);
                } else {
                    let (resolved_input_tokens, resolved_output_tokens) =
                        extract_prompt_result_usage(&message);
                    input_tokens = resolved_input_tokens;
                    output_tokens = resolved_output_tokens;
                    actual_model = extract_prompt_result_actual_model(&message);
                }
                break;
            }

            if message.get("method").and_then(Value::as_str) != Some("session/update") {
                continue;
            }
            let Some(params) = message.get("params") else {
                continue;
            };
            if params.get("sessionId").and_then(Value::as_str) != Some(resolved_session_id.as_str())
            {
                continue;
            }
            let Some(update) = params.get("update") else {
                continue;
            };
            match update.get("sessionUpdate").and_then(Value::as_str) {
                Some("agent_message_chunk") => {
                    let text = update
                        .get("content")
                        .and_then(|content| content.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !text.is_empty() {
                        response.push_str(text);
                    }
                }
                Some("tool_call") => {
                    if let Some(title) = extract_gemini_thread_title(update) {
                        thread_title = Some(title);
                    }
                    if let Some(tool_call_id) = tool_call_id(update) {
                        active_tool_calls.insert(tool_call_id.to_owned());
                    } else {
                        has_unkeyed_tool_call = true;
                    }
                    let message = tool_message(update, false);
                    on_chunk(StreamEvent::ToolUse {
                        message: message.clone(),
                    });
                    session_messages.push(message);
                }
                Some("tool_call_update") => {
                    if let Some(title) = extract_gemini_thread_title(update) {
                        thread_title = Some(title);
                    }
                    let status = update
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if matches!(status, "completed" | "failed") {
                        if let Some(tool_call_id) = tool_call_id(update) {
                            active_tool_calls.remove(tool_call_id);
                        } else {
                            has_unkeyed_tool_call = false;
                        }
                        let message = tool_message(update, true);
                        on_chunk(StreamEvent::ToolResult {
                            message: message.clone(),
                        });
                        session_messages.push(message);
                    }
                }
                Some("usage_update") => {
                    if let Some(amount) = update
                        .get("cost")
                        .and_then(|value| value.get("amount"))
                        .and_then(Value::as_f64)
                    {
                        cost = amount;
                    }
                }
                _ => {}
            }
        }

        let duration_ms = started.elapsed().as_millis() as i64;
        drop(stdin);
        let (child, _) = self.unregister_run(run_id).await;
        let stderr_output = self.cleanup_run_io(run_id, child, stderr_task).await;
        if !success && error.is_none() && !stderr_output.is_empty() {
            error = Some(stderr_output.clone());
        }
        let response = strip_gemini_thought_output(&response);
        if !response.is_empty() {
            append_gemini_assistant_session_message(&mut session_messages, &response);
            on_chunk(StreamEvent::Delta {
                text: response.clone(),
            });
        }
        on_chunk(StreamEvent::Done);
        if let Some(model) = actual_model.as_ref()
            && let Some(message) = session_messages
                .iter_mut()
                .rev()
                .find(|message| message.role == ProviderMessageRole::Assistant)
        {
            message
                .metadata
                .insert("actual_model".to_owned(), Value::String(model.clone()));
        }

        Ok(ProviderRunResult {
            run_id: run_id.to_owned(),
            thread_id: options.thread_id.clone(),
            response,
            session_messages,
            sdk_session_id: Some(resolved_session_id),
            actual_model,
            thread_title,
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
impl AgentLoopProvider for GeminiCliProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GeminiCli
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        if self.ready {
            return Ok(());
        }
        let output = Command::new(gemini_bin(&self.config))
            .arg("--version")
            .output()
            .await
            .map_err(|error| {
                BridgeError::Internal(format!("failed to invoke gemini CLI: {error}"))
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

        let first_attempt = self
            .run_once(options, &run_id, session_id.as_deref(), &on_chunk)
            .await;
        let mut result = match first_attempt {
            Ok(result) => result,
            Err(error) if session_id.is_some() && is_invalid_session_error(&error.to_string()) => {
                self.session_map.lock().await.remove(&options.thread_id);
                self.run_once(options, &run_id, None, &on_chunk).await?
            }
            Err(error) => return Err(error),
        };

        if !result.success
            && let Some(error) = result.error.as_deref()
            && session_id.is_some()
            && is_invalid_session_error(error)
        {
            self.session_map.lock().await.remove(&options.thread_id);
            result = self.run_once(options, &run_id, None, &on_chunk).await?;
        }

        on_chunk(StreamEvent::Done);
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
mod tests;
