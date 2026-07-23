use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use garyx_models::provider::{
    GrokBuildConfig, PromptAttachment, ProviderMessage, ProviderMessageRole, ProviderRateLimit,
    ProviderRunOptions, ProviderRunResult, ProviderType, SDK_SESSION_FORK_METADATA_KEY,
    SDK_SESSION_ID_METADATA_KEY, StreamEvent, attachments_from_metadata,
    build_prompt_message_with_attachments, stage_image_payloads_for_prompt,
};
use grok_agent_sdk::{
    GrokCancellation, GrokClient, GrokClientConfig, GrokError, GrokEvent, GrokMcpEnvVariable,
    GrokMcpHeader, GrokMcpServer, GrokRunOutput, GrokRunRequest,
};
use serde_json::{Value, json};

use crate::gary_prompt::{compose_gary_instructions, prepend_initial_context_to_user_message};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_common::{
    PendingRateLimits, garyx_mcp_server, metadata_bool, metadata_string, normalize_non_empty,
    resolve_uuid_run_id, runtime_env,
};
use crate::provider_trait::{
    BridgeError, ClearSessionOutcome, ProviderModelDefaults, ProviderRuntime,
    ProviderRuntimeSelection, StreamCallback,
};

const DEFAULT_REQUEST_TIMEOUT_SECS: f64 = 300.0;
const CANCEL_ACK_TIMEOUT: Duration = Duration::from_secs(1);
const CANCEL_SETTLE_TIMEOUT: Duration = Duration::from_secs(1);
const GROK_NATIVE_MCP_GUIDANCE: &str = concat!(
    "Grok Build MCP routing:\n",
    "- Use the native `search_tool` first with the server name and action, then call ",
    "`use_tool` with the qualified tool name and exact schema returned by the search.\n",
    "- A `mcp__garyx__<name>` reference in general Garyx guidance is a logical tool ",
    "reference. In Grok Build, resolve the matching `garyx` MCP tool through ",
    "`search_tool` and invoke it through `use_tool`; do not call that reference as a ",
    "direct function name."
);

#[derive(Clone)]
struct ActiveGrokRun {
    thread_id: String,
    cancellation: GrokCancellation,
}

struct ActiveRunGuard {
    run_id: String,
    active_runs: Arc<Mutex<HashMap<String, ActiveGrokRun>>>,
}

impl Drop for ActiveRunGuard {
    fn drop(&mut self) {
        self.active_runs
            .lock()
            .expect("Grok active-run lock poisoned")
            .remove(&self.run_id);
    }
}

pub struct GrokBuildProvider {
    config: RwLock<GrokBuildConfig>,
    ready: AtomicBool,
    session_map: Mutex<HashMap<String, String>>,
    active_runs: Arc<Mutex<HashMap<String, ActiveGrokRun>>>,
    pending_rate_limits: PendingRateLimits,
}

impl GrokBuildProvider {
    pub fn new(config: GrokBuildConfig) -> Self {
        Self {
            config: RwLock::new(config),
            ready: AtomicBool::new(false),
            session_map: Mutex::new(HashMap::new()),
            active_runs: Arc::new(Mutex::new(HashMap::new())),
            pending_rate_limits: PendingRateLimits::default(),
        }
    }
}

fn request_timeout(config: &GrokBuildConfig) -> Duration {
    Duration::from_secs_f64(if config.timeout_seconds > 0.0 {
        config.timeout_seconds
    } else {
        DEFAULT_REQUEST_TIMEOUT_SECS
    })
}

fn resolve_workspace_dir(
    config: &GrokBuildConfig,
    options: &ProviderRunOptions,
) -> Result<PathBuf, BridgeError> {
    let configured = options
        .workspace_dir
        .as_ref()
        .or(config.workspace_dir.as_ref());
    if let Some(path) = configured {
        let expanded = PathBuf::from(shellexpand::tilde(path).as_ref());
        if !expanded.is_dir() {
            return Err(BridgeError::RunFailed(format!(
                "Grok workspace does not exist: {}",
                expanded.display()
            )));
        }
        return Ok(expanded);
    }
    std::env::current_dir().map_err(|error| BridgeError::Internal(error.to_string()))
}

fn resolve_model(config: &GrokBuildConfig, options: &ProviderRunOptions) -> Option<String> {
    metadata_string(&options.metadata, "model")
        .or_else(|| normalize_non_empty(Some(&config.model)))
        .or_else(|| normalize_non_empty(Some(&config.default_model)))
}

fn resolve_reasoning_effort(
    config: &GrokBuildConfig,
    options: &ProviderRunOptions,
) -> Option<String> {
    metadata_string(&options.metadata, "model_reasoning_effort")
        .or_else(|| normalize_non_empty(Some(&config.model_reasoning_effort)))
}

fn build_prompt_text(
    options: &ProviderRunOptions,
    include_context: bool,
) -> (String, Vec<PromptAttachment>) {
    let mut attachments = attachments_from_metadata(&options.metadata);
    if attachments.is_empty() {
        attachments.extend(stage_image_payloads_for_prompt(
            "garyx-grok",
            options.images.as_deref().unwrap_or_default(),
        ));
    }
    let message = build_native_skill_prompt(&options.message, &options.metadata)
        .unwrap_or_else(|| options.message.clone());
    let message =
        prepend_initial_context_to_user_message(&message, &options.metadata, include_context);
    let message = build_prompt_message_with_attachments(&message, &attachments);
    (message, attachments)
}

fn build_session_rules(options: &ProviderRunOptions) -> String {
    let mut rules = compose_gary_instructions(
        options
            .metadata
            .get("system_prompt")
            .and_then(Value::as_str),
    );
    rules.push_str("\n\n");
    rules.push_str(GROK_NATIVE_MCP_GUIDANCE);
    rules
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
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

fn string_map(value: Option<&Value>) -> Vec<(String, String)> {
    let Some(entries) = value.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut entries = entries
        .iter()
        .filter_map(|(key, value)| {
            let value = value.as_str()?.trim();
            (!key.trim().is_empty() && !value.is_empty()).then(|| (key.clone(), value.to_owned()))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn mcp_headers(
    server: &serde_json::Map<String, Value>,
    launch_environment: &HashMap<String, String>,
) -> Vec<GrokMcpHeader> {
    let mut headers = string_map(server.get("http_headers"));
    for (name, value) in string_map(server.get("headers")) {
        if let Some(existing) = headers
            .iter_mut()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(&name))
        {
            *existing = (name, value);
        } else {
            headers.push((name, value));
        }
    }
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        && let Some(env_name) = server
            .get("bearer_token_env")
            .or_else(|| server.get("bearerTokenEnv"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        && let Some(token) = launch_environment
            .get(env_name)
            .cloned()
            .or_else(|| std::env::var(env_name).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    {
        headers.push(("Authorization".to_owned(), format!("Bearer {token}")));
    }
    headers.sort_by(|left, right| {
        left.0
            .to_ascii_lowercase()
            .cmp(&right.0.to_ascii_lowercase())
    });
    headers
        .into_iter()
        .map(|(name, value)| GrokMcpHeader { name, value })
        .collect()
}

fn normalize_remote_mcp_server(
    name: &str,
    raw_server: &Value,
    launch_environment: &HashMap<String, String>,
) -> Option<GrokMcpServer> {
    let server = raw_server.as_object()?;
    if !server
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        return None;
    }
    let transport = server
        .get("type")
        .or_else(|| server.get("transport"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let url = server
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if matches!(
        transport.as_str(),
        "http" | "streamable_http" | "streamable-http" | "sse"
    ) || (transport.is_empty() && url.is_some())
    {
        let url = url?;
        let headers = mcp_headers(server, launch_environment);
        return if transport == "sse" {
            Some(GrokMcpServer::Sse {
                name: name.to_owned(),
                url: url.to_owned(),
                headers,
            })
        } else {
            Some(GrokMcpServer::Http {
                name: name.to_owned(),
                url: url.to_owned(),
                headers,
            })
        };
    }

    let command = server
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(GrokMcpServer::Stdio {
        name: name.to_owned(),
        command: command.to_owned(),
        args: string_array(server.get("args")),
        env: string_map(server.get("env"))
            .into_iter()
            .map(|(name, value)| GrokMcpEnvVariable { name, value })
            .collect(),
    })
}

fn build_mcp_servers(
    config: &GrokBuildConfig,
    options: &ProviderRunOptions,
    run_id: &str,
    launch_environment: &HashMap<String, String>,
) -> Vec<GrokMcpServer> {
    let mut servers = Vec::new();
    if let Some(runtime_servers) = options
        .metadata
        .get("remote_mcp_servers")
        .and_then(Value::as_object)
    {
        let mut names = runtime_servers.keys().collect::<Vec<_>>();
        names.sort();
        for name in names {
            if name.eq_ignore_ascii_case("garyx") {
                continue;
            }
            if let Some(server) =
                normalize_remote_mcp_server(name, &runtime_servers[name], launch_environment)
            {
                servers.push(server);
            }
        }
    }
    if let Some(server) = garyx_mcp_server(
        &config.mcp_base_url,
        &options.thread_id,
        run_id,
        &options.metadata,
    ) {
        let mut headers = server
            .headers
            .into_iter()
            .map(|(name, value)| GrokMcpHeader { name, value })
            .collect::<Vec<_>>();
        headers.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        });
        servers.push(GrokMcpServer::Http {
            name: "garyx".to_owned(),
            url: server.url,
            headers,
        });
    }
    servers
}

#[derive(Default)]
struct ToolState {
    name: String,
    title: Option<String>,
    input: Value,
    started: bool,
    finished: bool,
}

#[derive(Default)]
struct GrokEventMapper {
    response: String,
    session_messages: Vec<ProviderMessage>,
    tools: HashMap<String, ToolState>,
}

impl GrokEventMapper {
    fn apply(&mut self, event: GrokEvent, on_chunk: &StreamCallback) {
        match event {
            GrokEvent::SessionBound { session_id } => {
                on_chunk(StreamEvent::SessionBound {
                    sdk_session_id: session_id,
                });
            }
            GrokEvent::SessionUpdate { update } => self.apply_update(update, on_chunk),
        }
    }

    fn apply_update(&mut self, update: Value, on_chunk: &StreamCallback) {
        match update_kind(&update) {
            Some("agent_message_chunk") => {
                let Some(text) = update
                    .get("content")
                    .and_then(|content| content.get("text"))
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                else {
                    return;
                };
                self.response.push_str(text);
                on_chunk(StreamEvent::Delta {
                    text: text.to_owned(),
                });
                append_assistant_message(&mut self.session_messages, text);
            }
            Some("tool_call") => self.apply_tool_call(&update, on_chunk),
            Some("tool_call_update") => self.apply_tool_update(&update, on_chunk),
            // Grok thought chunks are deliberately not exposed as transcript
            // content. User echoes and plan notifications are also represented
            // elsewhere in Garyx's committed event ledger.
            _ => {}
        }
    }

    fn apply_tool_call(&mut self, update: &Value, on_chunk: &StreamCallback) {
        let Some(id) = tool_call_id(update) else {
            return;
        };
        let state = self.tools.entry(id.clone()).or_default();
        merge_tool_state(state, update);
        // ToolUse is append-only in Garyx's stream contract. If Grok defers
        // rawInput to the first tool_call_update, wait one ACP frame so the
        // single emitted row contains the authoritative input.
        if !state.input.is_null() {
            emit_tool_use(&id, state, &mut self.session_messages, on_chunk);
        }
    }

    fn apply_tool_update(&mut self, update: &Value, on_chunk: &StreamCallback) {
        let Some(id) = tool_call_id(update) else {
            return;
        };
        let state = self.tools.entry(id.clone()).or_default();
        merge_tool_state(state, update);
        let status = update.get("status").and_then(Value::as_str);
        let terminal = matches!(status, Some("completed" | "failed" | "cancelled"));
        if !state.input.is_null() || terminal {
            emit_tool_use(&id, state, &mut self.session_messages, on_chunk);
        }
        if !terminal || state.finished {
            return;
        }
        state.finished = true;
        let is_error = matches!(status, Some("failed" | "cancelled"));
        let content = json!({
            "type": "acpToolResult",
            "id": id,
            "name": state.name,
            "title": state.title,
            "status": status,
            "output": update.get("rawOutput").cloned().unwrap_or(Value::Null),
            "content": update.get("content").cloned().unwrap_or(Value::Null),
        });
        let message = ProviderMessage::tool_result(
            content,
            Some(id),
            normalize_non_empty(Some(&state.name)),
            is_error.then_some(true),
        )
        .with_timestamp(chrono::Utc::now().to_rfc3339())
        .with_metadata_value("source", json!("grok_acp"));
        on_chunk(StreamEvent::ToolResult {
            message: message.clone(),
        });
        self.session_messages.push(message);
    }
}

fn update_kind(update: &Value) -> Option<&str> {
    update
        .get("sessionUpdate")
        .or_else(|| update.get("session_update"))
        .and_then(Value::as_str)
}

fn tool_call_id(update: &Value) -> Option<String> {
    update
        .get("toolCallId")
        .or_else(|| update.get("tool_call_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned)
}

fn merge_tool_state(state: &mut ToolState, update: &Value) {
    if let Some(value) = update.get("rawInput").filter(|value| !value.is_null()) {
        state.input = value.clone();
    }
    if let Some(title) = update.get("title").and_then(Value::as_str) {
        state.title = normalize_non_empty(Some(title));
    }
    let tool_meta = update.get("_meta").and_then(|meta| meta.get("x.ai/tool"));
    if let Some(name) = tool_meta
        .and_then(|tool| tool.get("name"))
        .and_then(Value::as_str)
        .or_else(|| update.get("kind").and_then(Value::as_str))
    {
        state.name = name.to_owned();
    }
    if state.name.is_empty() {
        state.name = state
            .title
            .clone()
            .unwrap_or_else(|| "Grok tool".to_owned());
    }
}

fn emit_tool_use(
    id: &str,
    state: &mut ToolState,
    session_messages: &mut Vec<ProviderMessage>,
    on_chunk: &StreamCallback,
) {
    if state.started {
        return;
    }
    state.started = true;
    let message = ProviderMessage::tool_use(
        json!({
            "type": "acpToolCall",
            "id": id,
            "name": state.name,
            "title": state.title,
            "input": state.input,
        }),
        Some(id.to_owned()),
        normalize_non_empty(Some(&state.name)),
    )
    .with_timestamp(chrono::Utc::now().to_rfc3339())
    .with_metadata_value("source", json!("grok_acp"));
    on_chunk(StreamEvent::ToolUse {
        message: message.clone(),
    });
    session_messages.push(message);
}

fn append_assistant_message(messages: &mut Vec<ProviderMessage>, delta: &str) {
    if let Some(last) = messages.last_mut()
        && last.role == ProviderMessageRole::Assistant
        && last.metadata.get("source").and_then(Value::as_str) == Some("grok_acp")
    {
        let mut text = last.text.clone().unwrap_or_default();
        text.push_str(delta);
        last.text = Some(text.clone());
        last.content = Value::String(text);
        return;
    }
    messages.push(
        ProviderMessage::assistant_text(delta)
            .with_timestamp(chrono::Utc::now().to_rfc3339())
            .with_metadata_value("source", json!("grok_acp")),
    );
}

fn bridge_error(error: &GrokError) -> BridgeError {
    match error {
        GrokError::Timeout => BridgeError::Timeout,
        _ => BridgeError::RunFailed(error.to_string()),
    }
}

fn standard_rate_limit(error: &GrokError) -> Option<ProviderRateLimit> {
    Some(ProviderRateLimit {
        provider: "grok_build".to_owned(),
        reached_type: Some(error.rate_limit_kind()?.to_owned()),
        message: error.provider_message().map(ToOwned::to_owned),
        ..Default::default()
    })
}

fn completion_status(stop_reason: Option<&str>) -> (bool, Option<String>) {
    let reason = stop_reason.map(str::trim).filter(|value| !value.is_empty());
    let Some(reason) = reason else {
        return (true, None);
    };
    let normalized = reason.to_ascii_lowercase();
    match normalized.as_str() {
        "end_turn" => (true, None),
        "cancelled" | "canceled" => (true, Some("Grok Build stopped: cancelled".to_owned())),
        "max_tokens" | "max_turn_requests" | "refusal" => {
            (false, Some(format!("Grok Build stopped: {normalized}")))
        }
        _ => (true, Some(format!("Grok Build stopped: {reason}"))),
    }
}

async fn wait_for_cancel_settlement(cancellation: &GrokCancellation) {
    let _ = cancellation.wait_acknowledged(CANCEL_ACK_TIMEOUT).await;
    let _ = cancellation.wait_completed(CANCEL_SETTLE_TIMEOUT).await;
}

#[async_trait]
impl ProviderRuntime for GrokBuildProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GrokBuild
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        self.ready.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        let cancellations = self
            .active_runs
            .lock()
            .expect("Grok active-run lock poisoned")
            .values()
            .map(|run| run.cancellation.clone())
            .collect::<Vec<_>>();
        for cancellation in cancellations {
            cancellation.cancel();
        }
        self.ready.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        if !self.is_ready() {
            return Err(BridgeError::ProviderNotReady);
        }
        if metadata_bool(&options.metadata, SDK_SESSION_FORK_METADATA_KEY) {
            return Err(BridgeError::SessionError(
                "grok provider does not support sdk session fork".to_owned(),
            ));
        }
        let started_at = Instant::now();
        let run_id = resolve_uuid_run_id(&options.metadata);
        self.pending_rate_limits.clear(&options.thread_id).await;
        let config = self
            .config
            .read()
            .expect("Grok config lock poisoned")
            .clone();
        let workspace = resolve_workspace_dir(&config, options)?;
        let existing_session_id = metadata_string(&options.metadata, SDK_SESSION_ID_METADATA_KEY)
            .or_else(|| {
                self.session_map
                    .lock()
                    .expect("Grok session lock poisoned")
                    .get(&options.thread_id)
                    .cloned()
            });
        let include_initial_context = existing_session_id.is_none();
        let (prompt, _staged_attachments) = build_prompt_text(options, include_initial_context);
        let session_rules = build_session_rules(options);
        let launch_environment = runtime_env(&config.env, &options.metadata);
        let mcp_servers = build_mcp_servers(&config, options, &run_id, &launch_environment);
        let cancellation = GrokCancellation::default();
        self.active_runs
            .lock()
            .expect("Grok active-run lock poisoned")
            .insert(
                run_id.clone(),
                ActiveGrokRun {
                    thread_id: options.thread_id.clone(),
                    cancellation: cancellation.clone(),
                },
            );
        let _active_run_guard = ActiveRunGuard {
            run_id: run_id.clone(),
            active_runs: Arc::clone(&self.active_runs),
        };

        // This owned config is the run's immutable launch snapshot. A hot
        // reload only changes the provider lock used by subsequent runs.
        let client = GrokClient::new(GrokClientConfig {
            binary: normalize_non_empty(Some(&config.grok_bin))
                .unwrap_or_else(|| "grok".to_owned()),
            environment: launch_environment,
            max_turns: config.max_turns,
            startup_timeout: Duration::from_secs(30),
            request_timeout: request_timeout(&config),
        });
        let mut mapper = GrokEventMapper::default();
        let result = client
            .run(
                GrokRunRequest {
                    cwd: workspace,
                    prompt,
                    session_id: existing_session_id.clone(),
                    model: resolve_model(&config, options),
                    reasoning_effort: resolve_reasoning_effort(&config, options),
                    rules: Some(session_rules),
                    mcp_servers,
                },
                cancellation,
                |event| {
                    if let GrokEvent::SessionBound { session_id } = &event {
                        self.session_map
                            .lock()
                            .expect("Grok session lock poisoned")
                            .insert(options.thread_id.clone(), session_id.clone());
                    }
                    mapper.apply(event, &on_chunk);
                },
            )
            .await;

        let output = match result {
            Ok(output) => output,
            Err(GrokError::Cancelled) => GrokRunOutput {
                session_id: self
                    .session_map
                    .lock()
                    .expect("Grok session lock poisoned")
                    .get(&options.thread_id)
                    .cloned()
                    .or(existing_session_id)
                    .ok_or_else(|| {
                        BridgeError::SessionError(
                            "Grok cancellation completed before a native session was bound"
                                .to_owned(),
                        )
                    })?,
                stop_reason: Some("cancelled".to_owned()),
                ..Default::default()
            },
            Err(error) => {
                if let Some(rate_limit) = standard_rate_limit(&error) {
                    self.pending_rate_limits
                        .stage(options.thread_id.clone(), rate_limit)
                        .await;
                }
                return Err(bridge_error(&error));
            }
        };
        let (success, error) = completion_status(output.stop_reason.as_deref());
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id,
            thread_id: options.thread_id.clone(),
            response: mapper.response,
            session_messages: mapper.session_messages,
            sdk_session_id: Some(output.session_id),
            actual_model: output.actual_model,
            thread_title: None,
            success,
            error,
            input_tokens: output.input_tokens,
            output_tokens: output.output_tokens,
            cost: 0.0,
            duration_ms: started_at.elapsed().as_millis() as i64,
        })
    }

    fn resolve_runtime_selection(&self, options: &ProviderRunOptions) -> ProviderRuntimeSelection {
        let config = self.config.read().expect("Grok config lock poisoned");
        ProviderRuntimeSelection {
            model: resolve_model(&config, options),
            model_reasoning_effort: resolve_reasoning_effort(&config, options),
            model_service_tier: None,
        }
    }

    fn update_model_defaults(&self, defaults: &ProviderModelDefaults) {
        let mut config = self.config.write().expect("Grok config lock poisoned");
        config.model = defaults.model.clone();
        config.default_model = defaults.default_model.clone();
        config.model_reasoning_effort = defaults.model_reasoning_effort.clone();
    }

    fn update_launch_environment(&self, env: &HashMap<String, String>) {
        self.config
            .write()
            .expect("Grok config lock poisoned")
            .env
            .clone_from(env);
    }

    fn abort_before_task_cancel(&self) -> bool {
        true
    }

    async fn abort(&self, run_id: &str) -> bool {
        let cancellation = self
            .active_runs
            .lock()
            .expect("Grok active-run lock poisoned")
            .get(run_id)
            .map(|run| run.cancellation.clone());
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
            wait_for_cancel_settlement(&cancellation).await;
            true
        } else {
            false
        }
    }

    async fn interrupt_streaming_session(&self, thread_id: &str) -> bool {
        let cancellation = self
            .active_runs
            .lock()
            .expect("Grok active-run lock poisoned")
            .values()
            .find(|run| run.thread_id == thread_id)
            .map(|run| run.cancellation.clone());
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
            wait_for_cancel_settlement(&cancellation).await;
            true
        } else {
            false
        }
    }

    async fn take_rate_limit(&self, thread_id: &str) -> Option<ProviderRateLimit> {
        self.pending_rate_limits.take(thread_id).await
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(self
            .session_map
            .lock()
            .expect("Grok session lock poisoned")
            .get(thread_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn clear_session(&self, thread_id: &str) -> ClearSessionOutcome {
        if self
            .session_map
            .lock()
            .expect("Grok session lock poisoned")
            .remove(thread_id)
            .is_some()
        {
            ClearSessionOutcome::Cleared
        } else {
            ClearSessionOutcome::AlreadyAbsent
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tokio::sync::Notify;

    fn callback() -> StreamCallback {
        Box::new(|_| {})
    }

    fn fake_grok(script_body: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fake-grok");
        fs::write(&path, format!("#!/bin/sh\n{script_body}\n")).expect("write script");
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("permissions");
        (dir, path.to_string_lossy().into_owned())
    }

    async fn initialized_provider(
        binary: String,
        workspace_dir: &std::path::Path,
        max_turns: Option<i64>,
    ) -> GrokBuildProvider {
        let mut provider = GrokBuildProvider::new(GrokBuildConfig {
            grok_bin: binary,
            workspace_dir: Some(workspace_dir.to_string_lossy().into_owned()),
            timeout_seconds: 5.0,
            max_turns,
            ..Default::default()
        });
        provider.initialize().await.expect("initialize provider");
        provider
    }

    fn run_options(thread_id: &str, workspace_dir: &std::path::Path) -> ProviderRunOptions {
        ProviderRunOptions {
            thread_id: thread_id.to_owned(),
            message: "continue".to_owned(),
            workspace_dir: Some(workspace_dir.to_string_lossy().into_owned()),
            images: None,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn system_rules_and_user_context_use_distinct_acp_channels() {
        let workspace = tempfile::tempdir().expect("workspace");
        let mut options = run_options("thread::grok-context", workspace.path());
        options.message = "Do the requested work.".to_owned();
        options.metadata.insert(
            "system_prompt".to_owned(),
            Value::String("Follow the custom agent policy.".to_owned()),
        );
        options.metadata.insert(
            "runtime_context".to_owned(),
            json!({
                "thread_id": "thread::grok-context",
                "channel": "api"
            }),
        );

        let system_rules = build_session_rules(&options);
        let (fresh_prompt, _) = build_prompt_text(&options, true);
        let (resumed_prompt, _) = build_prompt_text(&options, false);

        assert!(system_rules.contains("Garyx runtime guidance:"));
        assert!(system_rules.contains("Follow the custom agent policy."));
        assert!(system_rules.contains("Grok Build MCP routing:"));
        assert!(!fresh_prompt.contains("<system_instructions>"));
        assert!(!fresh_prompt.contains("Garyx runtime guidance:"));
        assert!(fresh_prompt.contains("<garyx_thread_metadata>"));
        assert!(fresh_prompt.contains("Do the requested work."));
        assert_eq!(resumed_prompt, "Do the requested work.");
    }

    #[test]
    fn mcp_servers_are_normalized_for_acp_and_garyx_name_is_reserved() {
        let workspace = tempfile::tempdir().expect("workspace");
        let mut options = run_options("thread::grok-mcp", workspace.path());
        options.metadata.insert(
            "remote_mcp_servers".to_owned(),
            json!({
                "alpha-http": {
                    "transport": "streamable_http",
                    "url": "https://mcp.example.com/http",
                    "bearer_token_env": "TEST_MCP_TOKEN",
                    "headers": {"X-Test": "yes"}
                },
                "beta-stdio": {
                    "transport": "stdio",
                    "command": "/usr/bin/example-mcp",
                    "args": ["--stdio"],
                    "env": {"MODE": "test"}
                },
                "events": {
                    "type": "sse",
                    "url": "https://mcp.example.com/events"
                },
                "disabled": {
                    "enabled": false,
                    "command": "/usr/bin/disabled"
                },
                "Garyx": {
                    "type": "http",
                    "url": "https://untrusted.example.com/mcp"
                }
            }),
        );
        options.metadata.insert(
            "garyx_mcp_headers".to_owned(),
            json!({"X-Custom": "custom"}),
        );
        options.metadata.insert(
            "garyx_mcp_auth_token".to_owned(),
            Value::String("test-gateway-token".to_owned()),
        );
        let config = GrokBuildConfig {
            mcp_base_url: "http://127.0.0.1:31337".to_owned(),
            ..Default::default()
        };
        let launch_environment =
            HashMap::from([("TEST_MCP_TOKEN".to_owned(), "test-token".to_owned())]);

        let servers = build_mcp_servers(&config, &options, "run-1", &launch_environment);

        assert_eq!(
            servers,
            vec![
                GrokMcpServer::Http {
                    name: "alpha-http".to_owned(),
                    url: "https://mcp.example.com/http".to_owned(),
                    headers: vec![
                        GrokMcpHeader {
                            name: "Authorization".to_owned(),
                            value: "Bearer test-token".to_owned(),
                        },
                        GrokMcpHeader {
                            name: "X-Test".to_owned(),
                            value: "yes".to_owned(),
                        },
                    ],
                },
                GrokMcpServer::Stdio {
                    name: "beta-stdio".to_owned(),
                    command: "/usr/bin/example-mcp".to_owned(),
                    args: vec!["--stdio".to_owned()],
                    env: vec![GrokMcpEnvVariable {
                        name: "MODE".to_owned(),
                        value: "test".to_owned(),
                    }],
                },
                GrokMcpServer::Sse {
                    name: "events".to_owned(),
                    url: "https://mcp.example.com/events".to_owned(),
                    headers: Vec::new(),
                },
                GrokMcpServer::Http {
                    name: "garyx".to_owned(),
                    url: "http://127.0.0.1:31337/mcp/auth/test-gateway-token/thread%3A%3Agrok-mcp/run-1".to_owned(),
                    headers: vec![
                        GrokMcpHeader {
                            name: "X-Custom".to_owned(),
                            value: "custom".to_owned(),
                        },
                        GrokMcpHeader {
                            name: "X-Run-Id".to_owned(),
                            value: "run-1".to_owned(),
                        },
                        GrokMcpHeader {
                            name: "X-Session-Key".to_owned(),
                            value: "thread::grok-mcp".to_owned(),
                        },
                        GrokMcpHeader {
                            name: "X-Thread-Id".to_owned(),
                            value: "thread::grok-mcp".to_owned(),
                        },
                    ],
                },
            ]
        );
    }

    #[test]
    fn tool_updates_emit_one_start_and_one_terminal_result() {
        let mut mapper = GrokEventMapper::default();
        mapper.apply_update(
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-1",
                "title": "Run command",
                "rawInput": {"command": "pwd"},
                "_meta": {"x.ai/tool": {"name": "run_terminal_command"}}
            }),
            &callback(),
        );
        mapper.apply_update(
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tool-1",
                "status": "completed",
                "rawOutput": {"output_for_prompt": "/workspace"}
            }),
            &callback(),
        );
        mapper.apply_update(
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tool-1",
                "status": "completed"
            }),
            &callback(),
        );

        assert_eq!(mapper.session_messages.len(), 2);
        assert_eq!(
            mapper.session_messages[0].role,
            ProviderMessageRole::ToolUse
        );
        assert_eq!(
            mapper.session_messages[1].role,
            ProviderMessageRole::ToolResult
        );
        assert_eq!(
            mapper.session_messages[0].tool_use_id.as_deref(),
            Some("tool-1")
        );
    }

    #[test]
    fn assistant_chunks_coalesce_without_exposing_thought_chunks() {
        let mut mapper = GrokEventMapper::default();
        for update in [
            json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"private"}}),
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hel"}}),
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"lo"}}),
        ] {
            mapper.apply_update(update, &callback());
        }
        assert_eq!(mapper.response, "hello");
        assert_eq!(mapper.session_messages.len(), 1);
        assert_eq!(mapper.session_messages[0].text.as_deref(), Some("hello"));
    }

    #[test]
    fn tool_input_from_the_first_update_is_present_on_the_single_tool_use() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);
        let callback: StreamCallback = Box::new(move |event| {
            events_for_callback
                .lock()
                .expect("events lock poisoned")
                .push(event);
        });
        let mut mapper = GrokEventMapper::default();
        mapper.apply_update(
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-late-input",
                "title": "Run command",
                "_meta": {"x.ai/tool": {"name": "run_terminal_command"}}
            }),
            &callback,
        );
        mapper.apply_update(
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tool-late-input",
                "status": "in_progress"
            }),
            &callback,
        );
        mapper.apply_update(
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tool-late-input",
                "status": "completed",
                "rawInput": {"command": "pwd"},
                "rawOutput": {"output_for_prompt": "/workspace"}
            }),
            &callback,
        );

        let events = events.lock().expect("events lock poisoned");
        let tool_uses = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::ToolUse { message } => Some(message),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0].content["input"], json!({"command": "pwd"}));
        assert_eq!(
            mapper.session_messages[0].content["input"],
            json!({"command": "pwd"})
        );
    }

    #[tokio::test]
    async fn interrupt_returns_partial_success_after_native_cancel_settles() {
        let workspace = tempfile::tempdir().expect("workspace");
        let settled_marker = workspace.path().join("cancel-settled");
        let script = format!(
            r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{}}}}' ;;
    *'"method":"session/new"'*) printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"sessionId":"cancel-session"}}}}' ;;
    *'"method":"session/prompt"'*) printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"cancel-session","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"partial answer"}}}}}}}}' ;;
    *'"method":"session/cancel"'*)
      sleep 0.2
      printf '%s' settled > '{}'
      printf '%s\n' '{{"jsonrpc":"2.0","id":3,"result":{{"stopReason":"cancelled","_meta":{{"inputTokens":4,"outputTokens":2}}}}}}' ;;
  esac
done
"#,
            settled_marker.display()
        );
        let (_binary_dir, binary) = fake_grok(&script);
        let provider = Arc::new(initialized_provider(binary, workspace.path(), None).await);
        let options = run_options("thread::grok-cancel", workspace.path());
        let partial_seen = Arc::new(Notify::new());
        let partial_for_callback = Arc::clone(&partial_seen);
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);
        let callback: StreamCallback = Box::new(move |event| {
            if matches!(&event, StreamEvent::Delta { text } if text == "partial answer") {
                partial_for_callback.notify_one();
            }
            events_for_callback
                .lock()
                .expect("events lock poisoned")
                .push(event);
        });

        let run = provider.run_streaming(&options, callback);
        let interrupt = async {
            tokio::time::timeout(Duration::from_secs(2), partial_seen.notified())
                .await
                .expect("partial response");
            let accepted = provider
                .interrupt_streaming_session("thread::grok-cancel")
                .await;
            (accepted, settled_marker.exists())
        };
        let (result, (accepted, settled_before_return)) = tokio::join!(run, interrupt);

        assert!(accepted);
        assert!(
            settled_before_return,
            "interrupt must wait briefly for Grok to process session/cancel"
        );
        let result = result.expect("cancel is a clean partial completion");
        assert!(result.success);
        assert_eq!(
            result.error.as_deref(),
            Some("Grok Build stopped: cancelled")
        );
        assert_eq!(result.response, "partial answer");
        assert_eq!(result.sdk_session_id.as_deref(), Some("cancel-session"));
        assert_eq!(result.input_tokens, 4);
        assert_eq!(result.output_tokens, 2);
        assert!(
            provider
                .take_rate_limit("thread::grok-cancel")
                .await
                .is_none(),
            "cancellation must not stage rate-limit state"
        );
        assert!(
            events
                .lock()
                .expect("events lock poisoned")
                .iter()
                .any(|event| matches!(event, StreamEvent::Done))
        );
    }

    #[tokio::test]
    async fn non_terminal_stop_reason_is_a_soft_failure() {
        let workspace = tempfile::tempdir().expect("workspace");
        let (_binary_dir, binary) = fake_grok(
            r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;;
    *'"method":"session/new"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"refusal-session"}}' ;;
    *'"method":"session/prompt"'*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"refusal"}}' ;;
  esac
done
"#,
        );
        let provider = initialized_provider(binary, workspace.path(), None).await;
        let result = provider
            .run_streaming(
                &run_options("thread::grok-refusal", workspace.path()),
                callback(),
            )
            .await
            .expect("ACP completion remains a soft result");

        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("Grok Build stopped: refusal"));
        assert_eq!(result.sdk_session_id.as_deref(), Some("refusal-session"));
    }

    #[tokio::test]
    async fn sdk_session_fork_is_rejected_before_process_launch() {
        let workspace = tempfile::tempdir().expect("workspace");
        let provider = initialized_provider(
            "/definitely/missing/grok".to_owned(),
            workspace.path(),
            None,
        )
        .await;
        let mut options = run_options("thread::grok-fork", workspace.path());
        options
            .metadata
            .insert(SDK_SESSION_FORK_METADATA_KEY.to_owned(), Value::Bool(true));

        let error = provider
            .run_streaming(&options, callback())
            .await
            .expect_err("fork must not silently create an empty session");
        assert!(
            matches!(error, BridgeError::SessionError(message) if message.contains("does not support sdk session fork"))
        );
    }

    #[tokio::test]
    async fn configured_max_turns_reaches_the_grok_process() {
        let workspace = tempfile::tempdir().expect("workspace");
        let (_binary_dir, binary) = fake_grok(
            r#"
case " $* " in
  *' --max-turns 7 '*) ;;
  *) printf '%s\n' 'missing --max-turns 7' >&2; exit 17 ;;
esac
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;;
    *'"method":"session/new"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"max-turns-session"}}' ;;
    *'"method":"session/prompt"'*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}' ;;
  esac
done
"#,
        );
        let provider = initialized_provider(binary, workspace.path(), Some(7)).await;
        let result = provider
            .run_streaming(
                &run_options("thread::grok-max-turns", workspace.path()),
                callback(),
            )
            .await
            .expect("configured max turns reaches the child");

        assert!(result.success);
        assert_eq!(result.sdk_session_id.as_deref(), Some("max-turns-session"));
    }

    #[test]
    fn structured_rate_limit_maps_only_to_standard_provider_state() {
        let error = GrokError::Rpc {
            method: "session/prompt".to_owned(),
            code: -32000,
            message: "upstream capacity".to_owned(),
            data: Some(json!({"http_status": 503})),
        };
        let rate_limit = standard_rate_limit(&error).expect("structured capacity error");

        assert_eq!(rate_limit.provider, "grok_build");
        assert_eq!(rate_limit.reached_type.as_deref(), Some("capacity"));
        assert_eq!(rate_limit.message.as_deref(), Some("upstream capacity"));
        assert!(standard_rate_limit(&GrokError::Transport("HTTP 429".to_owned())).is_none());
    }

    #[test]
    fn stop_reasons_have_explicit_terminal_semantics() {
        assert_eq!(completion_status(None), (true, None));
        assert_eq!(completion_status(Some("end_turn")), (true, None));
        assert_eq!(
            completion_status(Some("cancelled")),
            (true, Some("Grok Build stopped: cancelled".to_owned()))
        );
        for reason in ["max_tokens", "max_turn_requests", "refusal"] {
            assert_eq!(
                completion_status(Some(reason)),
                (false, Some(format!("Grok Build stopped: {reason}")))
            );
        }
        assert_eq!(
            completion_status(Some("future_reason")),
            (true, Some("Grok Build stopped: future_reason".to_owned()))
        );
    }
}
