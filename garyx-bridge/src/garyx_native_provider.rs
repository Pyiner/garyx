use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use garyx_models::provider::{
    GaryxNativeConfig, ProviderMessage, ProviderRunOptions, ProviderRunResult, ProviderType,
    QueuedUserInput, StreamBoundaryKind, StreamEvent, attachments_from_metadata,
    build_prompt_message_with_attachments,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::gary_prompt::{
    compose_gary_instructions, prepend_initial_context_to_user_message, task_cli_env,
};
use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

pub(crate) const SESSION_MESSAGES_METADATA_KEY: &str = "garyx_session_messages";
const OPENAI_RESPONSES_BASE_URL: &str = "https://api.openai.com/v1";
const CHATGPT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const DEFAULT_REQUEST_TIMEOUT_SECS: f64 = 300.0;
const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;

fn resolve_run_id(metadata: &HashMap<String, Value>) -> String {
    metadata
        .get("bridge_run_id")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("client_run_id").and_then(Value::as_str))
        .or_else(|| metadata.get("run_id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("run_{}", Uuid::new_v4()))
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn request_timeout(config: &GaryxNativeConfig) -> Duration {
    let timeout = if config.request_timeout_seconds > 0.0 {
        config.request_timeout_seconds
    } else if config.timeout_seconds > 0.0 {
        config.timeout_seconds
    } else {
        DEFAULT_REQUEST_TIMEOUT_SECS
    };
    Duration::from_secs_f64(timeout)
}

fn model_id(config: &GaryxNativeConfig, metadata: &HashMap<String, Value>) -> String {
    normalize_non_empty(metadata.get("model").and_then(Value::as_str))
        .or_else(|| normalize_non_empty(Some(config.model.as_str())))
        .or_else(|| normalize_non_empty(Some(config.default_model.as_str())))
        .unwrap_or_else(|| "gpt-5.5".to_owned())
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

fn resolve_runtime_env(
    config: &GaryxNativeConfig,
    metadata: &HashMap<String, Value>,
) -> HashMap<String, String> {
    let mut env = config.env.clone();
    env.extend(task_cli_env(metadata));
    env.extend(metadata_string_map(metadata, "desktop_codex_env"));
    env.extend(metadata_string_map(metadata, "desktop_garyx_native_env"));
    env
}

fn resolve_workspace_dir(config: &GaryxNativeConfig, options: &ProviderRunOptions) -> PathBuf {
    options
        .workspace_dir
        .as_ref()
        .or(config.workspace_dir.as_ref())
        .map(|value| PathBuf::from(shellexpand::tilde(value).as_ref()))
        .filter(|value| value.exists())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn truncate_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut clipped = value
        .chars()
        .take(limit.saturating_sub(20))
        .collect::<String>();
    clipped.push_str("\n[truncated]");
    clipped
}

fn provider_message_text(message: &ProviderMessage) -> Option<String> {
    message
        .text
        .clone()
        .or_else(|| message.content.as_str().map(ToOwned::to_owned))
        .or_else(|| {
            (!message.content.is_null())
                .then(|| serde_json::to_string(&message.content).ok())
                .flatten()
        })
}

fn persisted_session_messages(metadata: &HashMap<String, Value>) -> Vec<ProviderMessage> {
    metadata
        .get(SESSION_MESSAGES_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<ProviderMessage>>(value).ok())
        .unwrap_or_default()
}

fn goal_context(metadata: &HashMap<String, Value>) -> Option<String> {
    let goal = metadata.get("goal")?.as_object()?;
    let objective = goal
        .get("objective")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let status = goal
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("active");
    if status != "active" {
        return None;
    }
    Some(format!(
        "Current durable Garyx goal: {objective}\nUse get_goal to inspect it and update_goal with status=completed when the objective is genuinely done."
    ))
}

#[derive(Debug, Clone)]
pub(crate) struct NativeToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone)]
pub(crate) enum NativeModelOutput {
    Text(String),
    ToolCall(NativeToolCall),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NativeModelResponse {
    pub outputs: Vec<NativeModelOutput>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub actual_model: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct NativeModelRequest {
    pub model: String,
    pub instructions: String,
    pub messages: Vec<ProviderMessage>,
    pub tools: Vec<Value>,
    pub reasoning_effort: Option<String>,
    pub env: HashMap<String, String>,
}

#[async_trait]
pub(crate) trait NativeModelClient: Send + Sync {
    async fn sample(&self, request: NativeModelRequest)
    -> Result<NativeModelResponse, BridgeError>;
}

#[derive(Debug, Clone)]
struct NativeAuth {
    bearer_token: String,
    base_url: String,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthDotJson {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    tokens: Option<AuthTokens>,
    agent_identity: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthTokens {
    access_token: String,
    account_id: Option<String>,
    #[serde(default)]
    id_token: Value,
}

fn env_value(env: &HashMap<String, String>, name: &str) -> Option<String> {
    env.get(name)
        .cloned()
        .or_else(|| std::env::var(name).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn codex_home(config: &GaryxNativeConfig, env: &HashMap<String, String>) -> Option<PathBuf> {
    normalize_non_empty(Some(config.codex_home.as_str()))
        .or_else(|| env_value(env, "CODEX_HOME"))
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| format!("{home}/.codex"))
        })
        .map(|value| PathBuf::from(shellexpand::tilde(&value).as_ref()))
}

fn response_base_url(default_base_url: &str, config: &GaryxNativeConfig) -> String {
    normalize_non_empty(Some(config.base_url.as_str()))
        .unwrap_or_else(|| default_base_url.to_owned())
}

fn resolve_native_auth(
    config: &GaryxNativeConfig,
    env: &HashMap<String, String>,
) -> Result<NativeAuth, BridgeError> {
    if config.auth_source.trim().is_empty() || config.auth_source.trim() == "codex" {
        if let Some(api_key) =
            env_value(env, "CODEX_API_KEY").or_else(|| env_value(env, "OPENAI_API_KEY"))
        {
            return Ok(NativeAuth {
                bearer_token: api_key,
                base_url: response_base_url(OPENAI_RESPONSES_BASE_URL, config),
                account_id: None,
            });
        }

        let home = codex_home(config, env).ok_or_else(|| {
            BridgeError::RunFailed("Codex auth not found: CODEX_HOME/HOME is unset".to_owned())
        })?;
        let auth_path = home.join("auth.json");
        let contents = std::fs::read_to_string(&auth_path).map_err(|error| {
            BridgeError::RunFailed(format!(
                "Codex auth not found at {}: {error}",
                auth_path.display()
            ))
        })?;
        let auth: AuthDotJson = serde_json::from_str(&contents).map_err(|error| {
            BridgeError::RunFailed(format!(
                "Codex auth file {} is invalid: {error}",
                auth_path.display()
            ))
        })?;
        if let Some(api_key) = auth
            .openai_api_key
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            return Ok(NativeAuth {
                bearer_token: api_key,
                base_url: response_base_url(OPENAI_RESPONSES_BASE_URL, config),
                account_id: None,
            });
        }
        if let Some(tokens) = auth.tokens
            && !tokens.access_token.trim().is_empty()
        {
            let account_id = tokens.account_id.or_else(|| {
                tokens
                    .id_token
                    .get("chatgpt_account_id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
            return Ok(NativeAuth {
                bearer_token: tokens.access_token,
                base_url: response_base_url(CHATGPT_CODEX_BASE_URL, config),
                account_id,
            });
        }
        if auth.agent_identity.is_some() {
            return Err(BridgeError::RunFailed(
                "Codex auth contains only agent_identity; Garyx native currently supports CODEX_API_KEY, OPENAI_API_KEY, auth.json OPENAI_API_KEY, or auth.json tokens.access_token".to_owned(),
            ));
        }
        return Err(BridgeError::RunFailed(
            "Codex auth file does not contain a supported credential".to_owned(),
        ));
    }

    Err(BridgeError::RunFailed(format!(
        "unsupported Garyx native auth_source '{}'",
        config.auth_source
    )))
}

fn responses_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/responses")
    }
}

struct HttpNativeModelClient {
    config: GaryxNativeConfig,
    http: reqwest::Client,
}

impl HttpNativeModelClient {
    fn new(config: GaryxNativeConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    fn message_input(message: &ProviderMessage) -> Option<Value> {
        match message.role_str() {
            "user" => Some(json!({
                "role": "user",
                "content": provider_message_text(message).unwrap_or_default(),
            })),
            "assistant" => Some(json!({
                "role": "assistant",
                "content": provider_message_text(message).unwrap_or_default(),
            })),
            "system" => Some(json!({
                "role": "system",
                "content": provider_message_text(message).unwrap_or_default(),
            })),
            "tool_use" => {
                let call_id = message
                    .tool_use_id
                    .clone()
                    .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
                Some(json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": message.tool_name.clone().unwrap_or_default(),
                    "arguments": message.content.to_string(),
                }))
            }
            "tool_result" => {
                let call_id = message
                    .tool_use_id
                    .clone()
                    .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
                Some(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": provider_message_text(message).unwrap_or_else(|| message.content.to_string()),
                }))
            }
            _ => None,
        }
    }

    fn parse_response(value: Value) -> NativeModelResponse {
        let mut outputs = Vec::new();
        if let Some(items) = value.get("output").and_then(Value::as_array) {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("message") => {
                        if let Some(content) = item.get("content").and_then(Value::as_array) {
                            for block in content {
                                if let Some(text) = block
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .or_else(|| block.get("output_text").and_then(Value::as_str))
                                    && !text.is_empty()
                                {
                                    outputs.push(NativeModelOutput::Text(text.to_owned()));
                                }
                            }
                        }
                    }
                    Some("function_call") => {
                        let name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if name.is_empty() {
                            continue;
                        }
                        let id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .or_else(|| item.get("id").and_then(Value::as_str))
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
                        let arguments = item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .and_then(|text| serde_json::from_str::<Value>(text).ok())
                            .unwrap_or_else(|| {
                                item.get("arguments").cloned().unwrap_or(Value::Null)
                            });
                        outputs.push(NativeModelOutput::ToolCall(NativeToolCall {
                            id,
                            name,
                            arguments,
                        }));
                    }
                    _ => {}
                }
            }
        }

        let usage = value.get("usage").unwrap_or(&Value::Null);
        NativeModelResponse {
            outputs,
            input_tokens: usage
                .get("input_tokens")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            output_tokens: usage
                .get("output_tokens")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            actual_model: value
                .get("model")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        }
    }
}

#[async_trait]
impl NativeModelClient for HttpNativeModelClient {
    async fn sample(
        &self,
        request: NativeModelRequest,
    ) -> Result<NativeModelResponse, BridgeError> {
        let auth = resolve_native_auth(&self.config, &request.env)?;
        let mut input = Vec::new();
        for message in &request.messages {
            if let Some(item) = Self::message_input(message) {
                input.push(item);
            }
        }

        let mut body = json!({
            "model": request.model,
            "instructions": request.instructions,
            "input": input,
            "tools": request.tools,
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "stream": false,
            "store": false,
        });
        if let Some(effort) = request.reasoning_effort.as_deref()
            && !effort.trim().is_empty()
        {
            body["reasoning"] = json!({ "effort": effort.trim() });
        }

        let mut builder = self
            .http
            .post(responses_endpoint(&auth.base_url))
            .bearer_auth(auth.bearer_token)
            .json(&body);
        if let Some(account_id) = auth.account_id.as_deref()
            && !account_id.trim().is_empty()
        {
            builder = builder.header("ChatGPT-Account-ID", account_id);
        }
        let response = builder.send().await.map_err(|error| {
            BridgeError::RunFailed(format!("Garyx native model request failed: {error}"))
        })?;
        let status = response.status();
        let value = response.json::<Value>().await.map_err(|error| {
            BridgeError::RunFailed(format!(
                "Garyx native model response was invalid JSON: {error}"
            ))
        })?;
        if !status.is_success() {
            return Err(BridgeError::RunFailed(format!(
                "Garyx native model request failed with {status}: {value}"
            )));
        }
        Ok(Self::parse_response(value))
    }
}

#[derive(Debug, Clone)]
struct NativeSession {
    sdk_session_id: String,
    messages: Vec<ProviderMessage>,
    pending_inputs: VecDeque<QueuedUserInput>,
    interrupted: bool,
}

impl NativeSession {
    fn new(sdk_session_id: String) -> Self {
        Self {
            sdk_session_id,
            messages: Vec::new(),
            pending_inputs: VecDeque::new(),
            interrupted: false,
        }
    }
}

pub struct GaryxNativeProvider {
    config: GaryxNativeConfig,
    ready: Mutex<bool>,
    sessions: Mutex<HashMap<String, Arc<Mutex<NativeSession>>>>,
    active_runs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    model_client: Arc<dyn NativeModelClient>,
}

impl GaryxNativeProvider {
    pub fn new(config: GaryxNativeConfig) -> Self {
        let model_client = Arc::new(HttpNativeModelClient::new(config.clone()));
        Self::with_model_client(config, model_client)
    }

    pub(crate) fn with_model_client(
        config: GaryxNativeConfig,
        model_client: Arc<dyn NativeModelClient>,
    ) -> Self {
        Self {
            config,
            ready: Mutex::new(false),
            sessions: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            model_client,
        }
    }

    async fn ensure_session(&self, options: &ProviderRunOptions) -> Arc<Mutex<NativeSession>> {
        let restored_sid = options
            .metadata
            .get("sdk_session_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .entry(options.thread_id.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(NativeSession::new(
                    restored_sid
                        .clone()
                        .unwrap_or_else(|| format!("garyx-native-{}", Uuid::new_v4())),
                )))
            })
            .clone();
        drop(sessions);

        let persisted = persisted_session_messages(&options.metadata);
        if !persisted.is_empty() {
            let mut state = session.lock().await;
            if state.messages.is_empty() {
                state.messages = persisted;
            }
            if let Some(sid) = restored_sid
                && state.sdk_session_id != sid
            {
                state.sdk_session_id = sid;
            }
        }
        session
    }

    fn instructions(&self, options: &ProviderRunOptions) -> String {
        let mut parts = Vec::new();
        let runtime_system_prompt = options
            .metadata
            .get("system_prompt")
            .and_then(Value::as_str);
        parts.push(compose_gary_instructions(
            runtime_system_prompt,
            options.workspace_dir.as_deref().map(Path::new),
            options
                .metadata
                .get("automation_id")
                .and_then(Value::as_str),
        ));
        if let Some(goal) = goal_context(&options.metadata) {
            parts.push(goal);
        }
        parts.join("\n\n")
    }

    fn tool_schemas() -> Vec<Value> {
        vec![
            json!({
                "type": "function",
                "name": "exec_command",
                "description": "Run a shell command in the active workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "cmd": { "type": "string" },
                        "timeout_seconds": { "type": "number" }
                    },
                    "required": ["cmd"],
                    "additionalProperties": false
                }
            }),
            json!({
                "type": "function",
                "name": "read_file",
                "description": "Read a UTF-8 text file.",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                    "additionalProperties": false
                }
            }),
            json!({
                "type": "function",
                "name": "write_file",
                "description": "Write a UTF-8 text file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }
            }),
            json!({
                "type": "function",
                "name": "list_dir",
                "description": "List files and directories.",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "additionalProperties": false
                }
            }),
            json!({
                "type": "function",
                "name": "get_goal",
                "description": "Return the current durable Garyx goal for this thread.",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            }),
            json!({
                "type": "function",
                "name": "update_goal",
                "description": "Update the current durable Garyx goal status.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "status": { "type": "string", "enum": ["active", "paused", "completed"] },
                        "note": { "type": "string" }
                    },
                    "required": ["status"],
                    "additionalProperties": false
                }
            }),
        ]
    }

    async fn execute_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
        metadata: &HashMap<String, Value>,
    ) -> (Value, bool) {
        let result = match call.name.as_str() {
            "exec_command" => self.exec_command_tool(call, workspace_dir).await,
            "read_file" => self.read_file_tool(call, workspace_dir).await,
            "write_file" => self.write_file_tool(call, workspace_dir).await,
            "list_dir" => self.list_dir_tool(call, workspace_dir).await,
            "get_goal" => Ok(metadata.get("goal").cloned().unwrap_or(Value::Null)),
            "update_goal" => Ok(json!({
                "status": call.arguments.get("status").and_then(Value::as_str).unwrap_or("active"),
                "note": call.arguments.get("note").and_then(Value::as_str).unwrap_or(""),
            })),
            _ => Err(format!("unknown tool '{}'", call.name)),
        };
        match result {
            Ok(value) => (value, false),
            Err(error) => (json!({ "error": error }), true),
        }
    }

    async fn exec_command_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
    ) -> Result<Value, String> {
        let cmd = call
            .arguments
            .get("cmd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "missing cmd".to_owned())?;
        let timeout = call
            .arguments
            .get("timeout_seconds")
            .and_then(Value::as_f64)
            .filter(|value| *value > 0.0)
            .unwrap_or(60.0)
            .min(900.0);
        let output = tokio::time::timeout(
            Duration::from_secs_f64(timeout),
            Command::new("zsh")
                .arg("-lc")
                .arg(cmd)
                .current_dir(workspace_dir)
                .output(),
        )
        .await
        .map_err(|_| format!("command timed out after {timeout:.0}s"))?
        .map_err(|error| format!("failed to run command: {error}"))?;
        Ok(json!({
            "status": output.status.code(),
            "success": output.status.success(),
            "stdout": truncate_text(&String::from_utf8_lossy(&output.stdout), MAX_TOOL_OUTPUT_CHARS),
            "stderr": truncate_text(&String::from_utf8_lossy(&output.stderr), MAX_TOOL_OUTPUT_CHARS),
        }))
    }

    async fn read_file_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
    ) -> Result<Value, String> {
        let path = resolve_tool_path(workspace_dir, &call.arguments)?;
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        Ok(json!({
            "path": path.display().to_string(),
            "content": truncate_text(&contents, MAX_TOOL_OUTPUT_CHARS),
        }))
    }

    async fn write_file_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
    ) -> Result<Value, String> {
        let path = resolve_tool_path(workspace_dir, &call.arguments)?;
        let content = call
            .arguments
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing content".to_owned())?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        tokio::fs::write(&path, content)
            .await
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        Ok(json!({
            "path": path.display().to_string(),
            "bytes": content.len(),
        }))
    }

    async fn list_dir_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
    ) -> Result<Value, String> {
        let path = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .map(|value| path_from_arg(workspace_dir, value))
            .unwrap_or_else(|| workspace_dir.to_path_buf());
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(|error| format!("failed to list {}: {error}", path.display()))?;
        let mut values = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?
        {
            let file_type = entry.file_type().await.ok();
            values.push(json!({
                "name": entry.file_name().to_string_lossy(),
                "kind": if file_type.as_ref().is_some_and(|value| value.is_dir()) { "dir" } else { "file" },
            }));
        }
        Ok(json!({
            "path": path.display().to_string(),
            "entries": values,
        }))
    }
}

fn resolve_tool_path(workspace_dir: &Path, arguments: &Value) -> Result<PathBuf, String> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing path".to_owned())?;
    Ok(path_from_arg(workspace_dir, path))
}

fn path_from_arg(workspace_dir: &Path, path: &str) -> PathBuf {
    let expanded = PathBuf::from(shellexpand::tilde(path).as_ref());
    if expanded.is_absolute() {
        expanded
    } else {
        workspace_dir.join(expanded)
    }
}

#[async_trait]
impl AgentLoopProvider for GaryxNativeProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GaryxNative
    }

    fn is_ready(&self) -> bool {
        self.ready.try_lock().map(|value| *value).unwrap_or(false)
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        *self.ready.lock().await = true;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        self.sessions.lock().await.clear();
        self.active_runs.lock().await.clear();
        *self.ready.lock().await = false;
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let start = Instant::now();
        let run_id = resolve_run_id(&options.metadata);
        let workspace_dir = resolve_workspace_dir(&self.config, options);
        let session = self.ensure_session(options).await;
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_runs
            .lock()
            .await
            .insert(run_id.clone(), cancel.clone());

        let sdk_session_id = {
            let mut state = session.lock().await;
            state.interrupted = false;
            if !options.message.trim().is_empty() {
                let attachments = attachments_from_metadata(&options.metadata);
                let message = build_prompt_message_with_attachments(&options.message, &attachments);
                let message =
                    prepend_initial_context_to_user_message(&message, &options.metadata, true);
                state.messages.push(ProviderMessage::user_text(message));
            }
            state.sdk_session_id.clone()
        };
        on_chunk(StreamEvent::SessionBound {
            sdk_session_id: sdk_session_id.clone(),
        });

        let mut response_text = String::new();
        let mut run_session_messages = Vec::<ProviderMessage>::new();
        let mut input_tokens = 0i64;
        let mut output_tokens = 0i64;
        let mut actual_model = None;
        let mut iterations = 0u32;
        let max_iterations = self.config.max_tool_iterations.max(1);
        let instructions = self.instructions(options);
        let tools = Self::tool_schemas();

        loop {
            if cancel.load(Ordering::Relaxed) || session.lock().await.interrupted {
                self.active_runs.lock().await.remove(&run_id);
                return Err(BridgeError::RunFailed(
                    "Garyx native run interrupted".to_owned(),
                ));
            }

            let messages = { session.lock().await.messages.clone() };
            let request = NativeModelRequest {
                model: model_id(&self.config, &options.metadata),
                instructions: instructions.clone(),
                messages,
                tools: tools.clone(),
                reasoning_effort: normalize_non_empty(
                    options
                        .metadata
                        .get("model_reasoning_effort")
                        .and_then(Value::as_str)
                        .or_else(|| Some(self.config.model_reasoning_effort.as_str())),
                ),
                env: resolve_runtime_env(&self.config, &options.metadata),
            };
            let model_response = match tokio::time::timeout(
                request_timeout(&self.config),
                self.model_client.sample(request),
            )
            .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    self.active_runs.lock().await.remove(&run_id);
                    return Err(error);
                }
                Err(_) => {
                    self.active_runs.lock().await.remove(&run_id);
                    return Err(BridgeError::Timeout);
                }
            };

            input_tokens += model_response.input_tokens;
            output_tokens += model_response.output_tokens;
            if actual_model.is_none() {
                actual_model = model_response.actual_model.clone();
            }

            let mut needs_follow_up = false;
            for output in model_response.outputs {
                match output {
                    NativeModelOutput::Text(text) => {
                        if text.is_empty() {
                            continue;
                        }
                        response_text.push_str(&text);
                        on_chunk(StreamEvent::Delta { text: text.clone() });
                        let message = ProviderMessage::assistant_text(text);
                        session.lock().await.messages.push(message.clone());
                        run_session_messages.push(message);
                    }
                    NativeModelOutput::ToolCall(call) => {
                        iterations += 1;
                        if iterations > max_iterations {
                            self.active_runs.lock().await.remove(&run_id);
                            return Err(BridgeError::RunFailed(format!(
                                "Garyx native exceeded max_tool_iterations={max_iterations}"
                            )));
                        }
                        let tool_use = ProviderMessage::tool_use(
                            json!({
                                "name": call.name,
                                "arguments": call.arguments,
                            }),
                            Some(call.id.clone()),
                            Some(call.name.clone()),
                        );
                        on_chunk(StreamEvent::ToolUse {
                            message: tool_use.clone(),
                        });
                        session.lock().await.messages.push(tool_use.clone());
                        run_session_messages.push(tool_use);

                        let (result, is_error) = self
                            .execute_tool(&call, &workspace_dir, &options.metadata)
                            .await;
                        let tool_result = ProviderMessage::tool_result(
                            result,
                            Some(call.id),
                            Some(call.name),
                            Some(is_error),
                        );
                        on_chunk(StreamEvent::ToolResult {
                            message: tool_result.clone(),
                        });
                        session.lock().await.messages.push(tool_result.clone());
                        run_session_messages.push(tool_result);
                        needs_follow_up = true;
                    }
                }
            }

            let mut accepted_pending_input = false;
            loop {
                let pending = { session.lock().await.pending_inputs.pop_front() };
                let Some(pending) = pending else {
                    break;
                };
                on_chunk(StreamEvent::Boundary {
                    kind: StreamBoundaryKind::UserAck,
                    pending_input_id: pending.pending_input_id.clone(),
                });
                session
                    .lock()
                    .await
                    .messages
                    .push(ProviderMessage::user_text(pending.message));
                accepted_pending_input = true;
            }

            if !needs_follow_up && !accepted_pending_input {
                break;
            }
        }

        on_chunk(StreamEvent::Done);
        self.active_runs.lock().await.remove(&run_id);

        Ok(ProviderRunResult {
            run_id,
            thread_id: options.thread_id.clone(),
            response: response_text,
            session_messages: run_session_messages,
            sdk_session_id: Some(sdk_session_id),
            actual_model,
            thread_title: None,
            success: true,
            error: None,
            input_tokens,
            output_tokens,
            cost: 0.0,
            duration_ms: start.elapsed().as_millis() as i64,
        })
    }

    async fn abort(&self, run_id: &str) -> bool {
        self.active_runs
            .lock()
            .await
            .get(run_id)
            .map(|flag| {
                flag.store(true, Ordering::Relaxed);
                true
            })
            .unwrap_or(false)
    }

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn add_streaming_input(&self, thread_id: &str, input: QueuedUserInput) -> bool {
        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(thread_id).cloned()
        };
        let Some(session) = session else {
            return false;
        };
        session.lock().await.pending_inputs.push_back(input);
        true
    }

    async fn interrupt_streaming_session(&self, thread_id: &str) -> bool {
        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(thread_id).cloned()
        };
        let Some(session) = session else {
            return false;
        };
        session.lock().await.interrupted = true;
        true
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .entry(thread_id.to_owned())
            .or_insert_with(|| {
                Arc::new(Mutex::new(NativeSession::new(format!(
                    "garyx-native-{}",
                    Uuid::new_v4()
                ))))
            })
            .clone();
        Ok(session.lock().await.sdk_session_id.clone())
    }

    async fn clear_session(&self, thread_id: &str) -> bool {
        self.sessions.lock().await.remove(thread_id).is_some()
    }
}

#[cfg(test)]
mod tests;
