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
    GrokCancellation, GrokClient, GrokClientConfig, GrokError, GrokEvent, GrokRunRequest,
};
use serde_json::{Value, json};

use crate::gary_prompt::{compose_gary_instructions, prepend_initial_context_to_user_message};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_common::{
    PendingRateLimits, metadata_bool, metadata_string, normalize_non_empty, resolve_uuid_run_id,
    runtime_env,
};
use crate::provider_trait::{
    BridgeError, ClearSessionOutcome, ProviderModelDefaults, ProviderRuntime,
    ProviderRuntimeSelection, StreamCallback,
};

const DEFAULT_REQUEST_TIMEOUT_SECS: f64 = 300.0;

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
    include_instructions: bool,
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
        prepend_initial_context_to_user_message(&message, &options.metadata, include_instructions);
    let message = build_prompt_message_with_attachments(&message, &attachments);
    if !include_instructions {
        return (message, attachments);
    }
    let instructions = compose_gary_instructions(
        options
            .metadata
            .get("system_prompt")
            .and_then(Value::as_str),
    );
    let prompt = if message.trim().is_empty() {
        format!("<system_instructions>\n{instructions}\n</system_instructions>")
    } else {
        format!(
            "<system_instructions>\n{instructions}\n</system_instructions>\n\n<user_request>\n{message}\n</user_request>"
        )
    };
    (prompt, attachments)
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
        emit_tool_use(&id, state, &mut self.session_messages, on_chunk);
    }

    fn apply_tool_update(&mut self, update: &Value, on_chunk: &StreamCallback) {
        let Some(id) = tool_call_id(update) else {
            return;
        };
        let state = self.tools.entry(id.clone()).or_default();
        merge_tool_state(state, update);
        emit_tool_use(&id, state, &mut self.session_messages, on_chunk);
        let status = update.get("status").and_then(Value::as_str);
        if !matches!(status, Some("completed" | "failed" | "cancelled")) || state.finished {
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
        let started_at = Instant::now();
        let run_id = resolve_uuid_run_id(&options.metadata);
        self.pending_rate_limits.clear(&options.thread_id).await;
        let config = self
            .config
            .read()
            .expect("Grok config lock poisoned")
            .clone();
        let workspace = resolve_workspace_dir(&config, options)?;
        let existing_session_id = if metadata_bool(&options.metadata, SDK_SESSION_FORK_METADATA_KEY)
        {
            None
        } else {
            metadata_string(&options.metadata, SDK_SESSION_ID_METADATA_KEY).or_else(|| {
                self.session_map
                    .lock()
                    .expect("Grok session lock poisoned")
                    .get(&options.thread_id)
                    .cloned()
            })
        };
        let include_instructions = existing_session_id.is_none();
        let (prompt, _staged_attachments) = build_prompt_text(options, include_instructions);
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
            environment: runtime_env(&config.env, &options.metadata),
            startup_timeout: Duration::from_secs(30),
            request_timeout: request_timeout(&config),
        });
        let mut mapper = GrokEventMapper::default();
        let result = client
            .run(
                GrokRunRequest {
                    cwd: workspace,
                    prompt,
                    session_id: existing_session_id,
                    model: resolve_model(&config, options),
                    reasoning_effort: resolve_reasoning_effort(&config, options),
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
            Err(error) => {
                if let Some(rate_limit) = standard_rate_limit(&error) {
                    self.pending_rate_limits
                        .stage(options.thread_id.clone(), rate_limit)
                        .await;
                }
                return Err(bridge_error(&error));
            }
        };
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id,
            thread_id: options.thread_id.clone(),
            response: mapper.response,
            session_messages: mapper.session_messages,
            sdk_session_id: Some(output.session_id),
            actual_model: output.actual_model,
            thread_title: None,
            success: true,
            error: None,
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
            let _ = cancellation.wait_acknowledged(Duration::from_secs(1)).await;
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
            let _ = cancellation.wait_acknowledged(Duration::from_secs(1)).await;
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

    fn callback() -> StreamCallback {
        Box::new(|_| {})
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
}
