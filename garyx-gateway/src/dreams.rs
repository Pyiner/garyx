use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use claude_agent_sdk::{
    ClaudeAgentOptions, OutboundUserMessage, PermissionMode, run_streaming as run_claude_streaming,
};
use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::{ProviderMessage, ProviderMessageRole, ProviderType};
use garyx_router::{active_run_snapshot_messages, is_thread_key, workspace_dir_from_value};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::time::timeout;
use uuid::Uuid;

use crate::garyx_db::{DreamSpanDraft, DreamTopicDraft, GaryxDbError};
use crate::server::AppState;

const DEFAULT_LOOKBACK_HOURS: i64 = 24;
const MAX_LOOKBACK_HOURS: i64 = 24 * 31;
const DEFAULT_TOPIC_LIMIT: usize = 80;
const MAX_TOPIC_LIMIT: usize = 500;
const DEFAULT_MESSAGE_LIMIT: usize = 600;
const MAX_MESSAGE_LIMIT: usize = 2_000;
const CLAUDE_TIMEOUT_SECS: u64 = 120;
const MAX_CLAUDE_TIMEOUT_SECS: u64 = 170;
const MAX_PROMPT_TEXT_CHARS: usize = 1_000;
const CCTTY_BINARY_NAME: &str = "cctty";
const GARYX_CCTTY_PATH_ENV: &str = "GARYX_CCTTY_PATH";
const GARYX_CLAUDE_CLI_PATH_ENV: &str = "GARYX_CLAUDE_CLI_PATH";
const GARYX_CLAUDE_CLI_MODE_ENV: &str = "GARYX_CLAUDE_CLI_MODE";

#[derive(Debug, Clone, Deserialize)]
pub struct DreamListParams {
    pub from: Option<String>,
    pub to: Option<String>,
    pub since_hours: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DreamScanRequest {
    pub from: Option<String>,
    pub to: Option<String>,
    pub since_hours: Option<i64>,
    pub mode: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DreamScanMode {
    Auto,
    Claude,
    Heuristic,
}

#[derive(Debug, Clone)]
struct DreamWindow {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
struct CollectedDreamMessages {
    messages: Vec<DreamUserMessage>,
    threads_scanned: usize,
    matched_threads: usize,
}

#[derive(Debug, Clone, Serialize)]
struct DreamUserMessage {
    thread_id: String,
    workspace_dir: Option<String>,
    seq: u64,
    timestamp: DateTime<Utc>,
    text: String,
}

#[derive(Debug, Clone)]
struct ExtractedDreamTopic {
    title: String,
    summary: String,
    source: String,
    confidence: f64,
    spans: Vec<ExtractedDreamSpan>,
}

#[derive(Debug, Clone)]
struct ExtractedDreamSpan {
    thread_id: String,
    workspace_dir: Option<String>,
    start_seq: u64,
    end_seq: u64,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    excerpt: String,
    message_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeDreamResponse {
    #[serde(default)]
    topics: Vec<ClaudeDreamTopic>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeDreamTopic {
    title: Option<String>,
    summary: Option<String>,
    confidence: Option<f64>,
    #[serde(default)]
    spans: Vec<ClaudeDreamSpan>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeDreamSpan {
    thread_id: Option<String>,
    start_seq: Option<u64>,
    end_seq: Option<u64>,
    excerpt: Option<String>,
}

/// GET /api/dreams - list persisted dream topics.
pub async fn list_dreams(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DreamListParams>,
) -> impl IntoResponse {
    let window = match dream_window(
        params.from.as_deref(),
        params.to.as_deref(),
        params.since_hours,
    ) {
        Ok(window) => window,
        Err(message) => return bad_request(message).into_response(),
    };
    let limit = params
        .limit
        .unwrap_or(DEFAULT_TOPIC_LIMIT)
        .clamp(1, MAX_TOPIC_LIMIT);
    match state.ops.garyx_db.list_dream_topics(
        Some(&format_timestamp(window.from)),
        Some(&format_timestamp(window.to)),
        limit,
    ) {
        Ok(dreams) => {
            let latest_scan = state.ops.garyx_db.latest_dream_scan().ok().flatten();
            (
                StatusCode::OK,
                Json(json!({
                    "dreams": dreams,
                    "count": dreams.len(),
                    "from": format_timestamp(window.from),
                    "to": format_timestamp(window.to),
                    "latest_scan": latest_scan,
                })),
            )
                .into_response()
        }
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// GET /api/dreams/:dream_id - show one persisted dream topic and its thread spans.
pub async fn get_dream(
    State(state): State<Arc<AppState>>,
    AxumPath(dream_id): AxumPath<String>,
) -> impl IntoResponse {
    match state.ops.garyx_db.get_dream_topic(&dream_id) {
        Ok(Some(dream)) => (StatusCode::OK, Json(json!({ "dream": dream }))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "dream not found"})),
        )
            .into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// POST /api/dreams/scan - run a bounded dream scan and replace the scanned window.
pub async fn scan_dreams(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DreamScanRequest>,
) -> impl IntoResponse {
    let window = match dream_window(
        request.from.as_deref(),
        request.to.as_deref(),
        request.since_hours,
    ) {
        Ok(window) => window,
        Err(message) => return bad_request(message).into_response(),
    };
    let mode = match DreamScanMode::parse(request.mode.as_deref()) {
        Ok(mode) => mode,
        Err(message) => return bad_request(message).into_response(),
    };
    let message_limit = request
        .limit
        .unwrap_or(DEFAULT_MESSAGE_LIMIT)
        .clamp(1, MAX_MESSAGE_LIMIT);

    let collected =
        match collect_dream_messages(&state, window.from, window.to, message_limit).await {
            Ok(collected) => collected,
            Err(message) => return internal_error(message).into_response(),
        };

    let extraction = extract_dream_topics(&state, &collected, mode).await;
    let (topics, source, extraction_error) = match extraction {
        Ok(topics) => (topics, extraction_source(mode), None),
        Err(error) => {
            let topics = heuristic_topics(&collected.messages);
            (topics, "heuristic_fallback".to_owned(), Some(error))
        }
    };

    let drafts = dream_topic_drafts(topics);
    let scanned_from = format_timestamp(window.from);
    let scanned_to = format_timestamp(window.to);
    let scan = match state.ops.garyx_db.replace_dreams_in_window(
        &scanned_from,
        &scanned_to,
        &source,
        &drafts,
        extraction_error.as_deref(),
    ) {
        Ok(scan) => scan,
        Err(error) => return garyx_db_error_response(error).into_response(),
    };
    let dreams = match state.ops.garyx_db.list_dream_topics(
        Some(&scanned_from),
        Some(&scanned_to),
        DEFAULT_TOPIC_LIMIT,
    ) {
        Ok(dreams) => dreams,
        Err(error) => return garyx_db_error_response(error).into_response(),
    };

    (
        StatusCode::OK,
        Json(json!({
            "scan": scan,
            "dreams": dreams,
            "count": dreams.len(),
            "from": scanned_from,
            "to": scanned_to,
            "threads_scanned": collected.threads_scanned,
            "matched_threads": collected.matched_threads,
            "matched_messages": collected.messages.len(),
        })),
    )
        .into_response()
}

impl DreamScanMode {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value
            .map(str::trim)
            .filter(|candidate| !candidate.is_empty())
            .unwrap_or("auto")
            .to_ascii_lowercase()
            .as_str()
        {
            "auto" => Ok(Self::Auto),
            "claude" | "claude_code" | "llm" => Ok(Self::Claude),
            "heuristic" | "fast" | "local" => Ok(Self::Heuristic),
            other => Err(format!(
                "invalid dream scan mode `{other}`; use auto, claude, or heuristic"
            )),
        }
    }
}

fn extraction_source(mode: DreamScanMode) -> String {
    match mode {
        DreamScanMode::Auto | DreamScanMode::Claude => "claude".to_owned(),
        DreamScanMode::Heuristic => "heuristic".to_owned(),
    }
}

fn dream_window(
    raw_from: Option<&str>,
    raw_to: Option<&str>,
    since_hours: Option<i64>,
) -> Result<DreamWindow, String> {
    let to = match raw_to.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => parse_timestamp(value)?,
        None => Utc::now(),
    };
    let from = match raw_from.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => parse_timestamp(value)?,
        None => {
            let hours = since_hours
                .unwrap_or(DEFAULT_LOOKBACK_HOURS)
                .clamp(1, MAX_LOOKBACK_HOURS);
            to - Duration::hours(hours)
        }
    };
    if from > to {
        return Err("from must not be later than to".to_owned());
    }
    Ok(DreamWindow { from, to })
}

fn parse_timestamp(raw: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(raw.trim())
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| format!("invalid timestamp `{raw}`; use RFC3339"))
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Millis, true)
}

async fn collect_dream_messages(
    state: &Arc<AppState>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: usize,
) -> Result<CollectedDreamMessages, String> {
    let thread_keys = state.threads.thread_store.list_keys(Some("thread::")).await;
    let transcript_store = state.threads.history.transcript_store();
    let mut messages = Vec::new();
    let mut threads_scanned = 0usize;
    let mut matched_threads = 0usize;

    for thread_id in thread_keys {
        if !is_thread_key(&thread_id) {
            continue;
        }
        let Some(thread_data) = state.threads.thread_store.get(&thread_id).await else {
            continue;
        };
        if thread_last_updated(&thread_data)
            .map(|timestamp| timestamp < from)
            .unwrap_or(false)
        {
            continue;
        }
        threads_scanned += 1;

        let workspace_dir = workspace_dir_from_value(&thread_data);
        let mut thread_messages = Vec::new();
        let mut max_transcript_seq = 0u64;
        if transcript_store.exists(&thread_id).await {
            let records = transcript_store
                .records(&thread_id)
                .await
                .map_err(|error| format!("failed to load transcript for {thread_id}: {error}"))?;
            for record in records {
                max_transcript_seq = max_transcript_seq.max(record.seq);
                if let Some(message) = dream_user_message(
                    &thread_id,
                    workspace_dir.as_deref(),
                    record.seq,
                    &record.message,
                    Some(record.timestamp.as_str()),
                    from,
                    to,
                ) {
                    thread_messages.push(message);
                }
            }
        }

        let base_sequence = max_transcript_seq;
        for (idx, message) in active_run_snapshot_messages(&thread_data)
            .iter()
            .enumerate()
        {
            if let Some(entry) = dream_user_message(
                &thread_id,
                workspace_dir.as_deref(),
                base_sequence + idx as u64 + 1,
                message,
                None,
                from,
                to,
            ) {
                thread_messages.push(entry);
            }
        }

        if !thread_messages.is_empty() {
            matched_threads += 1;
            messages.extend(dedupe_thread_messages(thread_messages));
        }
    }

    messages.sort_by(|left, right| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left.thread_id.cmp(&right.thread_id))
            .then_with(|| left.seq.cmp(&right.seq))
    });
    if messages.len() > limit {
        let drop_count = messages.len() - limit;
        messages.drain(0..drop_count);
    }

    Ok(CollectedDreamMessages {
        messages,
        threads_scanned,
        matched_threads,
    })
}

fn thread_last_updated(thread_data: &Value) -> Option<DateTime<Utc>> {
    ["lastUpdatedAt", "updated_at", "last_updated_at"]
        .into_iter()
        .find_map(|field| thread_data.get(field).and_then(Value::as_str))
        .and_then(parse_stored_timestamp)
}

fn dedupe_thread_messages(messages: Vec<DreamUserMessage>) -> Vec<DreamUserMessage> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for message in messages {
        let key = (
            message.thread_id.clone(),
            format_timestamp(message.timestamp),
            message.text.clone(),
        );
        if seen.insert(key) {
            deduped.push(message);
        }
    }
    deduped
}

fn dream_user_message(
    thread_id: &str,
    workspace_dir: Option<&str>,
    seq: u64,
    message: &Value,
    timestamp_hint: Option<&str>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Option<DreamUserMessage> {
    if message_role(message).as_deref() != Some("user") {
        return None;
    }
    let text = extract_visible_text(message)?;
    let timestamp = timestamp_hint
        .or_else(|| message.get("timestamp").and_then(Value::as_str))
        .and_then(parse_stored_timestamp)
        .unwrap_or_else(Utc::now);
    if timestamp < from || timestamp > to {
        return None;
    }
    Some(DreamUserMessage {
        thread_id: thread_id.to_owned(),
        workspace_dir: workspace_dir.map(ToOwned::to_owned),
        seq,
        timestamp,
        text,
    })
}

fn message_role(message: &Value) -> Option<String> {
    if let Some(provider_message) = ProviderMessage::from_value(message) {
        return match provider_message.role {
            ProviderMessageRole::User => Some("user".to_owned()),
            ProviderMessageRole::Assistant => Some("assistant".to_owned()),
            ProviderMessageRole::System => Some("system".to_owned()),
            ProviderMessageRole::ToolUse => Some("tool_use".to_owned()),
            ProviderMessageRole::ToolResult => Some("tool_result".to_owned()),
        };
    }
    message
        .get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn extract_visible_text(message: &Value) -> Option<String> {
    if let Some(provider_message) = ProviderMessage::from_value(message) {
        if let Some(text) = provider_message
            .text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(text.to_owned());
        }
        let extracted = extract_text_from_content(&provider_message.content);
        if !extracted.is_empty() {
            return Some(extracted);
        }
    }

    if let Some(text) = garyx_router::message_text(message)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_owned());
    }

    let extracted = extract_text_from_content(message.get("content").unwrap_or(&Value::Null));
    if extracted.is_empty() {
        None
    } else {
        Some(extracted)
    }
}

fn extract_text_from_content(content: &Value) -> String {
    let mut parts = Vec::new();
    collect_content_text(content, &mut parts, 0);
    parts.join("\n")
}

fn collect_content_text(content: &Value, parts: &mut Vec<String>, depth: usize) {
    if depth > 32 {
        return;
    }
    match content {
        Value::String(text) => push_text_part(parts, text),
        Value::Array(items) => {
            for item in items {
                collect_content_text(item, parts, depth + 1);
            }
        }
        Value::Object(map) => collect_object_text(map, parts, depth + 1),
        _ => {}
    }
}

fn collect_object_text(map: &Map<String, Value>, parts: &mut Vec<String>, depth: usize) {
    if let Some(text) = map.get("text").and_then(Value::as_str) {
        push_text_part(parts, text);
    }
    if let Some(content) = map.get("content") {
        collect_content_text(content, parts, depth + 1);
    }
    if let Some(parts_value) = map.get("parts") {
        collect_content_text(parts_value, parts, depth + 1);
    }
    if let Some(items_value) = map.get("items") {
        collect_content_text(items_value, parts, depth + 1);
    }
}

fn push_text_part(parts: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_owned());
    }
}

fn parse_stored_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

async fn extract_dream_topics(
    state: &Arc<AppState>,
    collected: &CollectedDreamMessages,
    mode: DreamScanMode,
) -> Result<Vec<ExtractedDreamTopic>, String> {
    if collected.messages.is_empty() {
        return Ok(Vec::new());
    }
    if mode == DreamScanMode::Heuristic {
        return Ok(heuristic_topics(&collected.messages));
    }

    let config = state.config_snapshot();
    let prompt = build_claude_prompt(&collected.messages)?;
    let output = run_temporary_claude(&config, prompt).await?;
    let topics = parse_claude_topics(&output)?;
    let normalized = normalize_claude_topics(topics, &collected.messages);
    if normalized.is_empty() {
        return Err("Claude returned no usable dream topics".to_owned());
    }
    Ok(normalized)
}

fn build_claude_prompt(messages: &[DreamUserMessage]) -> Result<String, String> {
    let mut by_thread: BTreeMap<&str, (Option<String>, Vec<Value>)> = BTreeMap::new();
    for message in messages {
        let entry = by_thread
            .entry(message.thread_id.as_str())
            .or_insert_with(|| (message.workspace_dir.clone(), Vec::new()));
        if entry.0.is_none() {
            entry.0 = message.workspace_dir.clone();
        }
        entry.1.push(json!({
                "seq": message.seq,
                "timestamp": format_timestamp(message.timestamp),
                "text": truncate_chars(&message.text, MAX_PROMPT_TEXT_CHARS),
        }));
    }
    let threads = by_thread
        .into_iter()
        .map(|(thread_id, (workspace_dir, messages))| {
            json!({
                "thread_id": thread_id,
                "workspace_dir": workspace_dir,
                "messages": messages,
            })
        })
        .collect::<Vec<_>>();

    let payload = serde_json::to_string(&json!({ "threads": threads }))
        .map_err(|error| format!("failed to encode dream prompt payload: {error}"))?;
    Ok(format!(
        "Extract Garyx dream topics from recent user messages.\n\
         Rules:\n\
         - A topic is a user's coherent area of work or intent, not every single message.\n\
         - A thread may contain multiple topics when the user changes intent.\n\
         - A topic may contain spans from multiple threads if they clearly describe the same work.\n\
         - Prefer concise titles under 32 characters.\n\
         - Return JSON only, with this exact shape:\n\
         {{\"topics\":[{{\"title\":\"...\",\"summary\":\"...\",\"confidence\":0.0,\"spans\":[{{\"thread_id\":\"thread::...\",\"start_seq\":1,\"end_seq\":3,\"excerpt\":\"...\"}}]}}]}}\n\n\
         Input JSON:\n{payload}"
    ))
}

async fn run_temporary_claude(config: &GaryxConfig, prompt: String) -> Result<String, String> {
    let options = temporary_claude_options(config);
    let mut run = run_claude_streaming(options)
        .await
        .map_err(|error| format!("failed to start temporary Claude Code: {error}"))?;
    let control = run.control();
    control
        .send_user_message(OutboundUserMessage::text(prompt, ""))
        .await
        .map_err(|error| format!("failed to send dream prompt to Claude Code: {error}"))?;
    let result = match timeout(
        StdDuration::from_secs(claude_timeout_secs(&configured_claude_agent_config(config))),
        run.collect_until_result(),
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            let _ = run.finish().await;
            return Err(format!("temporary Claude Code failed: {error}"));
        }
        Err(_) => {
            let _ = run.finish().await;
            return Err("temporary Claude Code dream extraction timed out".to_owned());
        }
    };
    let _ = run.finish().await;

    if result.is_error {
        return Err(result
            .result
            .unwrap_or_else(|| "temporary Claude Code returned an error".to_owned()));
    }
    result
        .result
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "temporary Claude Code returned an empty result".to_owned())
}

fn temporary_claude_options(config: &GaryxConfig) -> ClaudeAgentOptions {
    let agent_cfg = configured_claude_agent_config(config);
    let mut options = ClaudeAgentOptions::default();
    options.system_prompt = Some(
        "You are a Garyx Dream extractor. You classify user activity into topic spans. Return JSON only and never modify files.".to_owned(),
    );
    options.permission_mode = Some(PermissionMode::Default);
    options.max_turns = Some(1);
    options.max_buffer_size = Some(10 * 1024 * 1024);
    options.setting_sources = Some(Vec::new());
    options.allowed_tools.clear();
    options.disallowed_tools.clear();
    options.extra_args.insert("bare".to_owned(), None);
    options
        .extra_args
        .insert("disable-slash-commands".to_owned(), None);
    options
        .extra_args
        .insert("strict-mcp-config".to_owned(), None);
    options
        .extra_args
        .insert("tools".to_owned(), Some(String::new()));
    if !agent_cfg.default_model.trim().is_empty() {
        options.model = Some(agent_cfg.default_model.trim().to_owned());
    }
    if let Some(path) = resolve_claude_cli_path(&agent_cfg) {
        options.cli_path = Some(path);
    }
    if let Some(workspace_dir) = agent_cfg
        .workspace_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        options.cwd = Some(PathBuf::from(workspace_dir));
    }
    options.env = agent_cfg.env.clone();
    options
}

fn claude_timeout_secs(agent_cfg: &AgentProviderConfig) -> u64 {
    if agent_cfg.timeout_seconds > 0.0 {
        agent_cfg
            .timeout_seconds
            .ceil()
            .clamp(10.0, MAX_CLAUDE_TIMEOUT_SECS as f64) as u64
    } else {
        CLAUDE_TIMEOUT_SECS
    }
}

fn configured_claude_agent_config(config: &GaryxConfig) -> AgentProviderConfig {
    for key in ["claude", "claude_code", "claude_tty"] {
        if let Some(value) = config.agents.get(key)
            && let Ok(mut agent_cfg) = serde_json::from_value::<AgentProviderConfig>(value.clone())
            && ProviderType::from_slug(&agent_cfg.provider_type) == Some(ProviderType::ClaudeCode)
        {
            agent_cfg.provider_type = ProviderType::ClaudeCode.as_slug().to_owned();
            return agent_cfg;
        }
    }
    AgentProviderConfig {
        provider_type: ProviderType::ClaudeCode.as_slug().to_owned(),
        ..Default::default()
    }
}

fn resolve_claude_cli_path(agent_cfg: &AgentProviderConfig) -> Option<PathBuf> {
    let configured_path = agent_cfg.claude_cli_path.trim();
    let explicit = (!configured_path.is_empty())
        .then(|| PathBuf::from(configured_path))
        .or_else(|| {
            agent_cfg
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
                .map(PathBuf::from)
                .filter(|value| !value.as_os_str().is_empty())
        });
    if explicit.is_some() {
        return explicit;
    }

    let mode = agent_cfg
        .env
        .get(GARYX_CLAUDE_CLI_MODE_ENV)
        .cloned()
        .or_else(|| std::env::var(GARYX_CLAUDE_CLI_MODE_ENV).ok())
        .unwrap_or_else(|| agent_cfg.claude_cli_mode.clone())
        .trim()
        .to_ascii_lowercase();
    if mode == "native" {
        return None;
    }
    bundled_cctty_path().or_else(|| executable_on_path(CCTTY_BINARY_NAME))
}

fn bundled_cctty_path() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let dir = current_exe.parent()?;
    let candidate = dir.join(CCTTY_BINARY_NAME);
    executable_file_exists(&candidate).then_some(candidate)
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    std::env::split_paths(&path_env)
        .map(|dir| dir.join(name))
        .find(|candidate| executable_file_exists(candidate))
}

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

fn parse_claude_topics(output: &str) -> Result<Vec<ClaudeDreamTopic>, String> {
    let json_text = extract_json_object(output)
        .ok_or_else(|| "temporary Claude Code did not return a JSON object".to_owned())?;
    let response: ClaudeDreamResponse = serde_json::from_str(json_text)
        .map_err(|error| format!("failed to parse Claude dream JSON: {error}"))?;
    Ok(response.topics)
}

fn extract_json_object(output: &str) -> Option<&str> {
    let trimmed = output.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (start < end).then_some(&trimmed[start..=end])
}

fn normalize_claude_topics(
    topics: Vec<ClaudeDreamTopic>,
    messages: &[DreamUserMessage],
) -> Vec<ExtractedDreamTopic> {
    let by_thread_seq = messages
        .iter()
        .map(|message| ((message.thread_id.as_str(), message.seq), message))
        .collect::<HashMap<_, _>>();
    let by_thread = messages_by_thread(messages);
    let mut normalized = Vec::new();

    for topic in topics {
        let mut spans = Vec::new();
        let mut seen_spans = HashSet::new();
        for raw_span in topic.spans {
            let Some(thread_id) = raw_span
                .thread_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(thread_messages) = by_thread.get(thread_id) else {
                continue;
            };
            let start_seq = raw_span.start_seq.unwrap_or_else(|| {
                thread_messages
                    .first()
                    .map(|message| message.seq)
                    .unwrap_or(1)
            });
            let end_seq = raw_span.end_seq.unwrap_or(start_seq);
            let span_messages = thread_messages
                .iter()
                .filter(|message| message.seq >= start_seq && message.seq <= end_seq)
                .copied()
                .collect::<Vec<_>>();
            let span_messages = if span_messages.is_empty() {
                by_thread_seq
                    .get(&(thread_id, start_seq))
                    .copied()
                    .into_iter()
                    .collect::<Vec<_>>()
            } else {
                span_messages
            };
            let Some(first) = span_messages.first().copied() else {
                continue;
            };
            let last = span_messages.last().copied().unwrap_or(first);
            if !seen_spans.insert((thread_id.to_owned(), first.seq, last.seq)) {
                continue;
            }
            spans.push(ExtractedDreamSpan {
                thread_id: thread_id.to_owned(),
                workspace_dir: first.workspace_dir.clone(),
                start_seq: first.seq,
                end_seq: last.seq,
                start_at: first.timestamp,
                end_at: last.timestamp,
                excerpt: raw_span
                    .excerpt
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| span_excerpt(&span_messages)),
                message_count: span_messages.len() as u32,
            });
        }
        if spans.is_empty() {
            continue;
        }
        let title = topic
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| truncate_chars(value, 64))
            .unwrap_or_else(|| title_from_text(&spans[0].excerpt));
        let summary = topic
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| truncate_chars(value, 240))
            .unwrap_or_else(|| spans[0].excerpt.clone());
        normalized.push(ExtractedDreamTopic {
            title,
            summary,
            source: "claude".to_owned(),
            confidence: topic.confidence.unwrap_or(0.82).clamp(0.0, 1.0),
            spans,
        });
    }
    normalized
}

fn heuristic_topics(messages: &[DreamUserMessage]) -> Vec<ExtractedDreamTopic> {
    let mut topics = Vec::new();
    for (_thread_id, thread_messages) in messages_by_thread(messages) {
        let mut current: Vec<&DreamUserMessage> = Vec::new();
        let mut last_timestamp: Option<DateTime<Utc>> = None;
        for message in thread_messages {
            let split = current
                .last()
                .and_then(|previous| {
                    (message.timestamp - previous.timestamp)
                        .to_std()
                        .ok()
                        .map(|gap| gap > StdDuration::from_secs(30 * 60))
                })
                .unwrap_or(false)
                || (!current.is_empty() && looks_like_topic_shift(&message.text));
            if split {
                if let Some(topic) = heuristic_topic_from_segment(&current) {
                    topics.push(topic);
                }
                current.clear();
            }
            last_timestamp = Some(message.timestamp);
            current.push(message);
        }
        if last_timestamp.is_some()
            && let Some(topic) = heuristic_topic_from_segment(&current)
        {
            topics.push(topic);
        }
    }
    topics.sort_by(|left, right| {
        right
            .spans
            .iter()
            .map(|span| span.end_at)
            .max()
            .cmp(&left.spans.iter().map(|span| span.end_at).max())
    });
    topics
}

fn messages_by_thread(messages: &[DreamUserMessage]) -> BTreeMap<&str, Vec<&DreamUserMessage>> {
    let mut by_thread: BTreeMap<&str, Vec<&DreamUserMessage>> = BTreeMap::new();
    for message in messages {
        by_thread
            .entry(message.thread_id.as_str())
            .or_default()
            .push(message);
    }
    by_thread
}

fn heuristic_topic_from_segment(segment: &[&DreamUserMessage]) -> Option<ExtractedDreamTopic> {
    let first = segment.first().copied()?;
    let last = segment.last().copied().unwrap_or(first);
    let excerpt = span_excerpt(segment);
    let title = title_from_text(&first.text);
    Some(ExtractedDreamTopic {
        title,
        summary: excerpt.clone(),
        source: "heuristic".to_owned(),
        confidence: if segment.len() > 1 { 0.58 } else { 0.42 },
        spans: vec![ExtractedDreamSpan {
            thread_id: first.thread_id.clone(),
            workspace_dir: first.workspace_dir.clone(),
            start_seq: first.seq,
            end_seq: last.seq,
            start_at: first.timestamp,
            end_at: last.timestamp,
            excerpt,
            message_count: segment.len() as u32,
        }],
    })
}

fn looks_like_topic_shift(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let english = [
        "another ",
        "next ",
        "new topic",
        "switching",
        "now let's",
        "also ",
        "separately",
    ];
    let chinese = [
        "另外",
        "还有",
        "接下来",
        "换个",
        "另一个",
        "第二个",
        "再看",
        "现在",
    ];
    english.iter().any(|prefix| lower.starts_with(prefix))
        || chinese.iter().any(|prefix| trimmed.starts_with(prefix))
}

fn span_excerpt(messages: &[&DreamUserMessage]) -> String {
    let joined = messages
        .iter()
        .take(3)
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    truncate_chars(&joined, 220)
}

fn title_from_text(text: &str) -> String {
    let cleaned = text
        .trim()
        .trim_start_matches('/')
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title = cleaned
        .split(['。', '，', '.', ',', '?', '？', '!', '！'])
        .next()
        .unwrap_or(cleaned.as_str())
        .trim();
    let title = if title.is_empty() {
        "Untitled Dream"
    } else {
        title
    };
    truncate_chars(title, 42)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= limit {
        return normalized;
    }
    normalized
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn dream_topic_drafts(topics: Vec<ExtractedDreamTopic>) -> Vec<DreamTopicDraft> {
    topics
        .into_iter()
        .filter_map(|topic| {
            let first_message_at = topic.spans.iter().map(|span| span.start_at).min()?;
            let last_message_at = topic.spans.iter().map(|span| span.end_at).max()?;
            let message_count = topic
                .spans
                .iter()
                .map(|span| span.message_count)
                .sum::<u32>();
            Some(DreamTopicDraft {
                dream_id: format!("dream::{}", Uuid::new_v4()),
                title: topic.title,
                summary: topic.summary,
                first_message_at: format_timestamp(first_message_at),
                last_message_at: format_timestamp(last_message_at),
                source: topic.source,
                confidence: topic.confidence,
                message_count,
                spans: topic
                    .spans
                    .into_iter()
                    .map(|span| DreamSpanDraft {
                        span_id: format!("dream_span::{}", Uuid::new_v4()),
                        thread_id: span.thread_id,
                        workspace_dir: span.workspace_dir,
                        start_seq: span.start_seq,
                        end_seq: span.end_seq,
                        start_at: format_timestamp(span.start_at),
                        end_at: format_timestamp(span.end_at),
                        excerpt: span.excerpt,
                        message_count: span.message_count,
                    })
                    .collect(),
            })
        })
        .collect()
}

fn bad_request(message: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": "BadRequest",
            "message": message,
        })),
    )
}

fn internal_error(message: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "InternalError",
            "message": message,
        })),
    )
}

fn garyx_db_error_response(error: GaryxDbError) -> (StatusCode, Json<Value>) {
    let (status, code) = match &error {
        GaryxDbError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BadRequest"),
        GaryxDbError::LockPoisoned | GaryxDbError::Io(_) | GaryxDbError::Sqlite(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "InternalError")
        }
    };
    (
        status,
        Json(json!({
            "error": code,
            "message": error.to_string(),
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use garyx_models::provider::ProviderMessage;

    fn user_message(text: &str, timestamp: &str) -> Value {
        let mut message = ProviderMessage::user_text(text);
        message.timestamp = Some(timestamp.to_owned());
        serde_json::to_value(message).expect("message serializes")
    }

    #[test]
    fn heuristic_splits_one_thread_into_multiple_topics() {
        let messages = vec![
            DreamUserMessage {
                thread_id: "thread::one".to_owned(),
                workspace_dir: None,
                seq: 1,
                timestamp: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
                text: "Review the pin API and gateway storage".to_owned(),
            },
            DreamUserMessage {
                thread_id: "thread::one".to_owned(),
                workspace_dir: None,
                seq: 2,
                timestamp: parse_timestamp("2026-05-21T10:04:00Z").unwrap(),
                text: "Check how mobile reads thread pins".to_owned(),
            },
            DreamUserMessage {
                thread_id: "thread::one".to_owned(),
                workspace_dir: None,
                seq: 3,
                timestamp: parse_timestamp("2026-05-21T10:08:00Z").unwrap(),
                text: "另外设计梦境的一天主题列表".to_owned(),
            },
        ];

        let topics = heuristic_topics(&messages);
        assert_eq!(topics.len(), 2);
        assert!(
            topics
                .iter()
                .any(|topic| topic.spans[0].start_seq == 1 && topic.spans[0].end_seq == 2)
        );
        assert!(topics.iter().any(|topic| topic.spans[0].start_seq == 3));
    }

    #[test]
    fn normalizes_claude_json_into_known_spans() {
        let messages = vec![
            DreamUserMessage {
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                seq: 1,
                timestamp: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
                text: "Implement dreams".to_owned(),
            },
            DreamUserMessage {
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                seq: 2,
                timestamp: parse_timestamp("2026-05-21T10:02:00Z").unwrap(),
                text: "Add CLI and mobile".to_owned(),
            },
        ];
        let raw = r#"{"topics":[{"title":"Dreams","summary":"Daily topic map","confidence":0.9,"spans":[{"thread_id":"thread::one","start_seq":1,"end_seq":2,"excerpt":"dreams work"}]}]}"#;
        let topics = normalize_claude_topics(parse_claude_topics(raw).unwrap(), &messages);
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].title, "Dreams");
        assert_eq!(
            topics[0].spans[0].workspace_dir.as_deref(),
            Some("/workspace/test")
        );
        assert_eq!(topics[0].spans[0].message_count, 2);
    }

    #[test]
    fn normalizes_claude_json_dedupes_duplicate_spans() {
        let messages = vec![DreamUserMessage {
            thread_id: "thread::one".to_owned(),
            workspace_dir: Some("/workspace/test".to_owned()),
            seq: 1,
            timestamp: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
            text: "Implement dream dedupe".to_owned(),
        }];
        let raw = r#"{
          "topics": [
            {
              "title": "Dream Dedupe",
              "spans": [
                {"thread_id": "thread::one", "start_seq": 1, "end_seq": 1},
                {"thread_id": "thread::one", "start_seq": 1, "end_seq": 1, "excerpt": "duplicate"}
              ]
            }
          ]
        }"#;

        let topics = normalize_claude_topics(parse_claude_topics(raw).unwrap(), &messages);

        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].spans.len(), 1);
        assert_eq!(topics[0].spans[0].start_seq, 1);
    }

    #[test]
    fn temporary_claude_options_disable_workspace_settings_and_tools() {
        let options = temporary_claude_options(&GaryxConfig::default());

        assert_eq!(options.setting_sources, Some(Vec::new()));
        assert_eq!(options.permission_mode, Some(PermissionMode::Default));
        assert!(options.allowed_tools.is_empty());
        assert!(options.disallowed_tools.is_empty());
        assert!(options.extra_args.contains_key("bare"));
        assert!(options.extra_args.contains_key("disable-slash-commands"));
        assert!(options.extra_args.contains_key("strict-mcp-config"));
        assert_eq!(
            options.extra_args.get("tools").and_then(Option::as_deref),
            Some("")
        );
        let args = options.to_cli_args();
        let setting_sources = args
            .iter()
            .position(|arg| arg == "--setting-sources")
            .expect("temporary Claude must explicitly override setting sources");
        assert_eq!(args[setting_sources + 1], "");
        let tools = args
            .iter()
            .position(|arg| arg == "--tools")
            .expect("temporary Claude must explicitly override built-in tools");
        assert_eq!(args[tools + 1], "");
    }

    #[test]
    fn dream_user_message_extracts_provider_user_text() {
        let message = user_message("A visible user request", "2026-05-21T10:00:00Z");
        let entry = dream_user_message(
            "thread::one",
            None,
            1,
            &message,
            None,
            parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
            parse_timestamp("2026-05-21T11:00:00Z").unwrap(),
        )
        .expect("user message is visible");
        assert_eq!(entry.text, "A visible user request");
    }
}
