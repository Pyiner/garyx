use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
#[cfg(test)]
use garyx_agent_loop::LlmOutput as NativeModelOutput;
use garyx_agent_loop::adapters::openai::OpenAiResponsesAdapter;
#[cfg(test)]
use garyx_agent_loop::adapters::openai::OpenAiResponsesAdapter as GptResponsesModelBackend;
#[cfg(test)]
use garyx_agent_loop::adapters::openai::ResponseStreamAccumulator;
use garyx_agent_loop::{
    AgentLoopError, AgentLoopEvent, AgentLoopRunRequest, AgentLoopSession, LlmAdapter,
    LlmToolCall as NativeToolCall, QueueMode, ToolExecution, ToolExecutor, run_agent_loop,
};
#[cfg(test)]
use garyx_agent_loop::{
    LlmRequest as NativeModelRequest, LlmResponse as NativeModelResponse, ModelVendor,
};
use garyx_models::provider::{
    GaryxNativeConfig, ProviderMessage, ProviderRunOptions, ProviderRunResult, ProviderType,
    QueuedUserInput, StreamBoundaryKind, StreamEvent, attachments_from_metadata,
    build_prompt_message_with_attachments,
};
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::gary_prompt::{
    compose_gary_instructions, prepend_initial_context_to_user_message, task_cli_env,
};
use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

pub(crate) const SESSION_MESSAGES_METADATA_KEY: &str = "garyx_session_messages";
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
    env.extend(metadata_string_map(metadata, "desktop_gpt_env"));
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

pub struct GaryxNativeProvider {
    config: GaryxNativeConfig,
    ready: Mutex<bool>,
    sessions: Mutex<HashMap<String, Arc<Mutex<AgentLoopSession>>>>,
    active_runs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    model_adapter: Arc<dyn LlmAdapter>,
}

impl GaryxNativeProvider {
    pub fn new(config: GaryxNativeConfig) -> Self {
        let model_adapter = Arc::new(OpenAiResponsesAdapter::new(config.clone()));
        Self::with_model_adapter(config, model_adapter)
    }

    pub(crate) fn with_model_adapter(
        config: GaryxNativeConfig,
        model_adapter: Arc<dyn LlmAdapter>,
    ) -> Self {
        Self {
            config,
            ready: Mutex::new(false),
            sessions: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            model_adapter,
        }
    }

    async fn ensure_session(&self, options: &ProviderRunOptions) -> Arc<Mutex<AgentLoopSession>> {
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
                Arc::new(Mutex::new(AgentLoopSession::new(
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

    async fn run_tool(
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

struct BridgeToolExecutor<'a> {
    provider: &'a GaryxNativeProvider,
    workspace_dir: PathBuf,
    metadata: &'a HashMap<String, Value>,
}

#[async_trait]
impl ToolExecutor for BridgeToolExecutor<'_> {
    async fn execute_tool(&self, call: &NativeToolCall) -> ToolExecution {
        let (content, is_error) = self
            .provider
            .run_tool(call, &self.workspace_dir, self.metadata)
            .await;
        ToolExecution {
            content,
            is_error,
            terminate: false,
        }
    }
}

fn map_loop_error(error: AgentLoopError) -> BridgeError {
    match error {
        AgentLoopError::Timeout => BridgeError::Timeout,
        AgentLoopError::Failed(message) => BridgeError::RunFailed(message),
    }
}

#[async_trait]
impl AgentLoopProvider for GaryxNativeProvider {
    fn provider_type(&self) -> ProviderType {
        // The provider type is the selected model backend. The in-process
        // native loop is the execution engine behind this backend.
        ProviderType::Gpt
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
        let request = AgentLoopRunRequest {
            model: model_id(&self.config, &options.metadata),
            instructions: self.instructions(options),
            tools: Self::tool_schemas(),
            reasoning_effort: normalize_non_empty(
                options
                    .metadata
                    .get("model_reasoning_effort")
                    .and_then(Value::as_str)
                    .or_else(|| Some(self.config.model_reasoning_effort.as_str())),
            ),
            service_tier: normalize_non_empty(
                options
                    .metadata
                    .get("model_service_tier")
                    .and_then(Value::as_str)
                    .or_else(|| Some(self.config.model_service_tier.as_str())),
            ),
            env: resolve_runtime_env(&self.config, &options.metadata),
            request_timeout: request_timeout(&self.config),
            max_tool_iterations: self.config.max_tool_iterations,
            max_turns: self
                .config
                .max_turns
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0),
            queue_mode: QueueMode::All,
            compaction: None,
        };
        let tool_executor = BridgeToolExecutor {
            provider: self,
            workspace_dir,
            metadata: &options.metadata,
        };
        let mut emitted_sdk_session_id = sdk_session_id.clone();
        let outcome = run_agent_loop(
            session,
            self.model_adapter.as_ref(),
            &tool_executor,
            request,
            cancel,
            |event| match event {
                AgentLoopEvent::SessionBound { sdk_session_id } => {
                    emitted_sdk_session_id = sdk_session_id.clone();
                    on_chunk(StreamEvent::SessionBound { sdk_session_id });
                }
                AgentLoopEvent::Delta { text } => on_chunk(StreamEvent::Delta { text }),
                AgentLoopEvent::ToolUse { message } => on_chunk(StreamEvent::ToolUse { message }),
                AgentLoopEvent::ToolResult { message } => {
                    on_chunk(StreamEvent::ToolResult { message })
                }
                AgentLoopEvent::UserAck { pending_input_id } => on_chunk(StreamEvent::Boundary {
                    kind: StreamBoundaryKind::UserAck,
                    pending_input_id,
                }),
                AgentLoopEvent::Done => on_chunk(StreamEvent::Done),
                _ => {}
            },
        )
        .await
        .map_err(map_loop_error);

        self.active_runs.lock().await.remove(&run_id);
        let outcome = outcome?;

        Ok(ProviderRunResult {
            run_id,
            thread_id: options.thread_id.clone(),
            response: outcome.response,
            session_messages: outcome.session_messages,
            sdk_session_id: Some(emitted_sdk_session_id),
            actual_model: outcome.actual_model,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: outcome.input_tokens,
            output_tokens: outcome.output_tokens,
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
                Arc::new(Mutex::new(AgentLoopSession::new(format!(
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
