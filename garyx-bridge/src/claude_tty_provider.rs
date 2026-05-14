use std::collections::{HashMap, VecDeque};
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use garyx_models::provider::{
    ClaudeCodeConfig, PromptAttachment, ProviderMessage, ProviderRunOptions, ProviderRunResult,
    ProviderType, QueuedUserInput, StreamBoundaryKind, StreamEvent, attachments_from_metadata,
    build_prompt_message_with_attachments, stage_image_payloads_for_prompt,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::gary_prompt::{
    compose_gary_instructions, prepend_initial_context_to_user_message, task_cli_env,
};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

const DEFAULT_COMPLETION_IDLE_MS: u64 = 1_500;
const TRANSCRIPT_POLL_MS: u64 = 120;
const ABORT_TIMEOUT_SECS: u64 = 5;
const DEFAULT_RUN_TIMEOUT_SECS: u64 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeTtyLaunchMode {
    NewSession,
    Resume,
}

#[derive(Debug, Clone)]
struct TtySpawnSpec {
    command: String,
    args: Vec<String>,
    cwd: PathBuf,
    env: HashMap<String, String>,
}

trait TtyProcess: Send {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn interrupt(&mut self) -> io::Result<()> {
        self.write_all(b"\x03")
    }
    fn kill(&mut self);
}

trait TtyBackend: Send + Sync {
    fn spawn(&self, spec: &TtySpawnSpec) -> Result<Box<dyn TtyProcess>, BridgeError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingAckMarker {
    RootUserMessage,
    QueuedInput(String),
}

struct ClaudeTtySession {
    session_id: String,
    transcript_path: PathBuf,
    transcript_offset: u64,
    process: Box<dyn TtyProcess>,
}

/// Claude Code provider that drives the interactive CLI through a real PTY.
///
/// The PTY is only a control transport. Garyx still uses Claude's persisted
/// JSONL transcript as the structured event source so desktop/channel surfaces
/// do not need to render or parse a terminal UI.
pub struct ClaudeTtyProvider {
    config: ClaudeCodeConfig,
    backend: Arc<dyn TtyBackend>,
    claude_config_dir: Option<PathBuf>,
    sessions: Mutex<HashMap<String, Arc<Mutex<ClaudeTtySession>>>>,
    session_ids: Mutex<HashMap<String, String>>,
    active_runs: Mutex<HashMap<String, String>>,
    run_pending_inputs: Mutex<HashMap<String, VecDeque<PendingAckMarker>>>,
    ready: bool,
    completion_idle: Duration,
    poll_interval: Duration,
}

impl ClaudeTtyProvider {
    pub fn new(config: ClaudeCodeConfig) -> Self {
        Self::with_backend(config, Arc::new(default_tty_backend()))
    }

    fn with_backend(config: ClaudeCodeConfig, backend: Arc<dyn TtyBackend>) -> Self {
        Self {
            config,
            backend,
            claude_config_dir: None,
            sessions: Mutex::new(HashMap::new()),
            session_ids: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            run_pending_inputs: Mutex::new(HashMap::new()),
            ready: false,
            completion_idle: Duration::from_millis(DEFAULT_COMPLETION_IDLE_MS),
            poll_interval: Duration::from_millis(TRANSCRIPT_POLL_MS),
        }
    }

    #[cfg(test)]
    fn with_backend_for_test(
        config: ClaudeCodeConfig,
        backend: Arc<dyn TtyBackend>,
        completion_idle: Duration,
    ) -> Self {
        Self {
            completion_idle,
            poll_interval: Duration::from_millis(10),
            ..Self::with_backend(config, backend)
        }
    }

    #[cfg(test)]
    fn with_backend_and_config_dir_for_test(
        config: ClaudeCodeConfig,
        backend: Arc<dyn TtyBackend>,
        completion_idle: Duration,
        claude_config_dir: PathBuf,
    ) -> Self {
        Self {
            claude_config_dir: Some(claude_config_dir),
            ..Self::with_backend_for_test(config, backend, completion_idle)
        }
    }

    fn run_timeout(&self) -> Duration {
        let configured = self.config.timeout_seconds;
        if configured.is_finite() && configured > 0.0 {
            Duration::from_secs_f64(configured)
        } else {
            Duration::from_secs(DEFAULT_RUN_TIMEOUT_SECS)
        }
    }

    async fn ensure_session(
        &self,
        options: &ProviderRunOptions,
        run_id: &str,
    ) -> Result<Arc<Mutex<ClaudeTtySession>>, BridgeError> {
        if let Some(existing) = self.sessions.lock().await.get(&options.thread_id).cloned() {
            return Ok(existing);
        }

        let (session_id, had_session_id) = self.resolve_or_create_session_id(options).await;
        let cwd = resolve_tty_cwd(&self.config, options)?;
        let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
        let env = self.build_env(options);
        let config_dir = match self.claude_config_dir.as_ref() {
            Some(path) => path.clone(),
            None => claude_config_dir(&env)?,
        };
        let transcript_path = claude_transcript_path(&config_dir, &cwd, &session_id);
        let launch_mode = if had_session_id || transcript_path.exists() {
            ClaudeTtyLaunchMode::Resume
        } else {
            ClaudeTtyLaunchMode::NewSession
        };
        let args = self.build_claude_args(options, run_id, &session_id, launch_mode, &cwd);
        let spec = TtySpawnSpec {
            command: "claude".to_owned(),
            args,
            cwd: cwd.clone(),
            env,
        };
        let process = self.backend.spawn(&spec)?;
        let session = Arc::new(Mutex::new(ClaudeTtySession {
            session_id,
            transcript_path,
            transcript_offset: 0,
            process,
        }));
        self.sessions
            .lock()
            .await
            .insert(options.thread_id.clone(), session.clone());
        Ok(session)
    }

    async fn resolve_or_create_session_id(&self, options: &ProviderRunOptions) -> (String, bool) {
        if let Some(existing) = self
            .session_ids
            .lock()
            .await
            .get(&options.thread_id)
            .cloned()
        {
            return (existing, true);
        }
        if let Some(from_metadata) = options
            .metadata
            .get("sdk_session_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let session_id = from_metadata.to_owned();
            self.session_ids
                .lock()
                .await
                .insert(options.thread_id.clone(), session_id.clone());
            return (session_id, true);
        }
        let session_id = Uuid::new_v4().to_string();
        self.session_ids
            .lock()
            .await
            .insert(options.thread_id.clone(), session_id.clone());
        (session_id, false)
    }

    fn build_claude_args(
        &self,
        options: &ProviderRunOptions,
        run_id: &str,
        session_id: &str,
        launch_mode: ClaudeTtyLaunchMode,
        cwd: &Path,
    ) -> Vec<String> {
        let mut args = Vec::new();

        match launch_mode {
            ClaudeTtyLaunchMode::NewSession => {
                args.push("--session-id".to_owned());
                args.push(session_id.to_owned());
            }
            ClaudeTtyLaunchMode::Resume => {
                args.push("--resume".to_owned());
                args.push(session_id.to_owned());
            }
        }

        if !self.config.permission_mode.trim().is_empty() {
            args.push("--permission-mode".to_owned());
            args.push(self.config.permission_mode.clone());
        }
        if !self.config.disallowed_tools.is_empty() {
            args.push("--disallowedTools".to_owned());
            args.push(self.config.disallowed_tools.join(","));
        }
        if let Some(model) = resolve_requested_model(&self.config, &options.metadata) {
            args.push("--model".to_owned());
            args.push(model);
        }
        if !self.config.setting_sources.is_empty() {
            args.push("--setting-sources".to_owned());
            args.push(self.config.setting_sources.join(","));
        }

        let mcp_config = self.build_mcp_config(options, run_id);
        if let Some(mcp_config) = mcp_config {
            args.push("--mcp-config".to_owned());
            args.push(mcp_config.to_string());
        }

        let runtime_system_prompt = options
            .metadata
            .get("system_prompt")
            .and_then(Value::as_str)
            .or(self.config.system_prompt.as_deref());
        let session_agent_id = options
            .metadata
            .get("agent_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let session_agent_name = options
            .metadata
            .get("agent_display_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Garyx custom agent");
        if let Some(agent_id) = session_agent_id {
            args.push("--agents".to_owned());
            args.push(
                json!({
                    agent_id: {
                        "description": format!("Garyx custom agent: {session_agent_name}"),
                        "prompt": runtime_system_prompt.unwrap_or(""),
                    }
                })
                .to_string(),
            );
            args.push("--agent".to_owned());
            args.push(agent_id.to_owned());
        } else {
            args.push("--append-system-prompt".to_owned());
            args.push(compose_gary_instructions(
                runtime_system_prompt,
                Some(cwd),
                options
                    .metadata
                    .get("automation_id")
                    .and_then(Value::as_str),
            ));
        }

        args
    }

    fn build_mcp_config(&self, options: &ProviderRunOptions, run_id: &str) -> Option<Value> {
        if self.config.mcp_base_url.trim().is_empty() {
            return None;
        }
        let mut headers = serde_json::Map::new();
        headers.insert("X-Run-Id".to_owned(), Value::String(run_id.to_owned()));
        headers.insert(
            "X-Thread-Id".to_owned(),
            Value::String(options.thread_id.clone()),
        );
        headers.insert(
            "X-Session-Key".to_owned(),
            Value::String(options.thread_id.clone()),
        );
        if let Some(extra) = options
            .metadata
            .get("garyx_mcp_headers")
            .and_then(Value::as_object)
        {
            for (key, value) in extra {
                if let Some(value) = value.as_str() {
                    headers.insert(key.clone(), Value::String(value.to_owned()));
                }
            }
        }
        let encoded_thread = urlencoding::encode(&options.thread_id);
        let encoded_run = urlencoding::encode(run_id);
        let url = options
            .metadata
            .get("garyx_mcp_auth_token")
            .and_then(Value::as_str)
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
        Some(json!({
            "mcpServers": {
                "garyx": {
                    "type": "http",
                    "url": url,
                    "headers": headers,
                }
            }
        }))
    }

    fn build_env(&self, options: &ProviderRunOptions) -> HashMap<String, String> {
        let mut env = self.config.env.clone();
        env.extend(task_cli_env(&options.metadata));
        env.extend(metadata_string_map(&options.metadata, "desktop_claude_env"));
        env
    }

    async fn prepare_transcript_offset(&self, session: &Arc<Mutex<ClaudeTtySession>>) {
        let mut guard = session.lock().await;
        guard.transcript_offset = std::fs::metadata(&guard.transcript_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
    }

    async fn write_prompt_to_session(
        &self,
        session: &Arc<Mutex<ClaudeTtySession>>,
        prompt: &str,
    ) -> Result<(), BridgeError> {
        let bytes = bracketed_paste_input(prompt);
        let mut guard = session.lock().await;
        guard.process.write_all(&bytes).map_err(|error| {
            BridgeError::RunFailed(format!("failed to write claude tty input: {error}"))
        })
    }

    async fn initialize_pending_inputs(&self, run_id: &str) {
        self.run_pending_inputs.lock().await.insert(
            run_id.to_owned(),
            VecDeque::from([PendingAckMarker::RootUserMessage]),
        );
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
            })
        {
            queue.remove(index);
        }
    }

    async fn acknowledge_next_pending_input(&self, run_id: &str) -> Option<String> {
        self.run_pending_inputs
            .lock()
            .await
            .get_mut(run_id)
            .and_then(|queue| match queue.pop_front() {
                Some(PendingAckMarker::QueuedInput(pending_input_id)) => Some(pending_input_id),
                Some(PendingAckMarker::RootUserMessage) | None => None,
            })
    }

    async fn resolve_active_run_id_for_session(&self, thread_id: &str) -> Option<String> {
        self.active_runs
            .lock()
            .await
            .iter()
            .find_map(|(run_id, mapped_thread_id)| {
                (mapped_thread_id == thread_id).then(|| run_id.clone())
            })
    }

    async fn tail_run_until_idle(
        &self,
        run_id: &str,
        session: &Arc<Mutex<ClaudeTtySession>>,
        on_chunk: &StreamCallback,
    ) -> Result<TtyRunOutcome, BridgeError> {
        let started = Instant::now();
        let timeout = self.run_timeout();
        let mut state = TranscriptRunState::default();
        let mut last_activity = Instant::now();

        loop {
            if started.elapsed() > timeout {
                return Err(BridgeError::Timeout);
            }

            let (path, mut offset) = {
                let guard = session.lock().await;
                (guard.transcript_path.clone(), guard.transcript_offset)
            };

            match read_complete_transcript_lines(&path, offset).await {
                Ok((lines, consumed)) if consumed > 0 => {
                    offset += consumed;
                    session.lock().await.transcript_offset = offset;
                    for line in lines {
                        if apply_transcript_line(run_id, &line, &mut state, self, on_chunk).await {
                            last_activity = Instant::now();
                        }
                    }
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(BridgeError::RunFailed(format!(
                        "failed to read claude transcript: {error}"
                    )));
                }
            }

            if state.result_seen {
                break;
            }
            if state.assistant_seen && last_activity.elapsed() >= self.completion_idle {
                break;
            }
            tokio::time::sleep(self.poll_interval).await;
        }

        Ok(TtyRunOutcome {
            session_id: state.session_id,
            response_text: state.response_text,
            session_messages: state.session_messages,
            is_error: state.is_error,
            error_message: state.error_message,
            input_tokens: state.input_tokens,
            output_tokens: state.output_tokens,
            cost_usd: state.cost_usd,
            actual_model: state.actual_model,
            thread_title: state.thread_title,
        })
    }
}

#[async_trait]
impl AgentLoopProvider for ClaudeTtyProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeTty
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        let output = tokio::process::Command::new("claude")
            .arg("--version")
            .output()
            .await;
        match output {
            Ok(output) if output.status.success() => {
                tracing::info!("Claude TTY provider initialized");
                self.ready = true;
                Ok(())
            }
            _ => Err(BridgeError::Internal(
                "claude binary not found in PATH".to_owned(),
            )),
        }
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        let sessions = self.sessions.lock().await.drain().collect::<Vec<_>>();
        for (_thread_id, session) in sessions {
            session.lock().await.process.kill();
        }
        self.session_ids.lock().await.clear();
        self.active_runs.lock().await.clear();
        self.run_pending_inputs.lock().await.clear();
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
        let started = Instant::now();
        let session = self.ensure_session(options, &run_id).await?;
        let (sdk_session_id, include_context) = {
            let guard = session.lock().await;
            let known_history = guard.transcript_path.exists()
                && std::fs::metadata(&guard.transcript_path)
                    .map(|metadata| metadata.len() > 0)
                    .unwrap_or(false);
            (guard.session_id.clone(), !known_history)
        };
        on_chunk(StreamEvent::SessionBound {
            sdk_session_id: sdk_session_id.clone(),
        });

        self.prepare_transcript_offset(&session).await;
        self.active_runs
            .lock()
            .await
            .insert(run_id.clone(), options.thread_id.clone());
        self.initialize_pending_inputs(&run_id).await;

        let prompt = build_tty_user_prompt(options, include_context);
        let send_result = self.write_prompt_to_session(&session, &prompt).await;
        if let Err(error) = send_result {
            self.active_runs.lock().await.remove(&run_id);
            self.run_pending_inputs.lock().await.remove(&run_id);
            return Err(error);
        }

        let outcome = match self.tail_run_until_idle(&run_id, &session, &on_chunk).await {
            Ok(outcome) => outcome,
            Err(error) => {
                self.active_runs.lock().await.remove(&run_id);
                self.run_pending_inputs.lock().await.remove(&run_id);
                return Err(error);
            }
        };
        self.active_runs.lock().await.remove(&run_id);
        self.run_pending_inputs.lock().await.remove(&run_id);
        on_chunk(StreamEvent::Done);

        let duration_ms = started.elapsed().as_millis() as i64;
        Ok(ProviderRunResult {
            run_id,
            thread_id: options.thread_id.clone(),
            response: outcome.response_text,
            session_messages: outcome.session_messages,
            sdk_session_id: Some(outcome.session_id.unwrap_or(sdk_session_id)),
            actual_model: outcome
                .actual_model
                .or_else(|| resolve_requested_model(&self.config, &options.metadata)),
            thread_title: outcome.thread_title,
            success: !outcome.is_error,
            error: if outcome.is_error {
                outcome
                    .error_message
                    .or_else(|| Some("claude tty transcript reported error".to_owned()))
            } else {
                None
            },
            input_tokens: outcome.input_tokens,
            output_tokens: outcome.output_tokens,
            cost: outcome.cost_usd,
            duration_ms,
        })
    }

    async fn abort(&self, run_id: &str) -> bool {
        let thread_id = self.active_runs.lock().await.remove(run_id);
        self.run_pending_inputs.lock().await.remove(run_id);
        let Some(thread_id) = thread_id else {
            return false;
        };
        let session = self.sessions.lock().await.get(&thread_id).cloned();
        let Some(session) = session else {
            return false;
        };
        let interrupt = async {
            session
                .lock()
                .await
                .process
                .interrupt()
                .map_err(|error| error.to_string())
        };
        match tokio::time::timeout(Duration::from_secs(ABORT_TIMEOUT_SECS), interrupt).await {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(run_id = %run_id, error = %error, "failed to interrupt claude tty run");
                false
            }
            Err(_) => {
                tracing::warn!(run_id = %run_id, "timed out interrupting claude tty run");
                false
            }
        }
    }

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn add_streaming_input(&self, thread_id: &str, input: QueuedUserInput) -> bool {
        let Some(run_id) = self.resolve_active_run_id_for_session(thread_id).await else {
            return false;
        };
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
        let Some(session) = self.sessions.lock().await.get(thread_id).cloned() else {
            self.rollback_pending_input(&run_id, &pending_input_id)
                .await;
            return false;
        };
        let prompt = build_tty_prompt_from_parts(&input.message, &input.attachments);
        match self.write_prompt_to_session(&session, &prompt).await {
            Ok(()) => true,
            Err(error) => {
                self.rollback_pending_input(&run_id, &pending_input_id)
                    .await;
                tracing::warn!(thread_id = %thread_id, error = %error, "failed to write queued claude tty input");
                false
            }
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
        let mut sessions = self.session_ids.lock().await;
        if let Some(existing) = sessions.get(thread_id) {
            return Ok(existing.clone());
        }
        let session_id = Uuid::new_v4().to_string();
        sessions.insert(thread_id.to_owned(), session_id.clone());
        Ok(session_id)
    }

    async fn clear_session(&self, thread_id: &str) -> bool {
        let removed_session = self.sessions.lock().await.remove(thread_id);
        if let Some(session) = removed_session.as_ref() {
            session.lock().await.process.kill();
        }
        let removed_id = self.session_ids.lock().await.remove(thread_id);
        let stale_run_ids = self
            .active_runs
            .lock()
            .await
            .iter()
            .filter_map(|(run_id, mapped_thread_id)| {
                (mapped_thread_id == thread_id).then(|| run_id.clone())
            })
            .collect::<Vec<_>>();
        for run_id in stale_run_ids {
            self.active_runs.lock().await.remove(&run_id);
            self.run_pending_inputs.lock().await.remove(&run_id);
        }
        removed_id.is_some() || removed_session.is_some()
    }
}

#[derive(Default)]
struct TranscriptRunState {
    session_id: Option<String>,
    response_text: String,
    session_messages: Vec<ProviderMessage>,
    assistant_seen: bool,
    result_seen: bool,
    is_error: bool,
    error_message: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cost_usd: f64,
    actual_model: Option<String>,
    thread_title: Option<String>,
}

struct TtyRunOutcome {
    session_id: Option<String>,
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

async fn apply_transcript_line(
    run_id: &str,
    line: &str,
    state: &mut TranscriptRunState,
    provider: &ClaudeTtyProvider,
    on_chunk: &StreamCallback,
) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "system" => {
            if state.session_id.is_none() {
                state.session_id = value
                    .get("session_id")
                    .or_else(|| value.get("sessionId"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned);
                if let Some(session_id) = state.session_id.clone() {
                    on_chunk(StreamEvent::SessionBound {
                        sdk_session_id: session_id,
                    });
                }
            }
            if state.thread_title.is_none() {
                state.thread_title = extract_claude_thread_title(&value);
            }
            true
        }
        "ai-title" => {
            state.thread_title = value
                .get("aiTitle")
                .and_then(Value::as_str)
                .map(normalize_thread_title)
                .filter(|value| !value.is_empty())
                .or_else(|| state.thread_title.take());
            true
        }
        "user" => {
            let content = value
                .get("message")
                .and_then(|message| message.get("content"))
                .unwrap_or(&Value::Null);
            let tool_result_messages = extract_tool_results_from_user_content(content);
            if !tool_result_messages.is_empty() {
                for message in tool_result_messages {
                    on_chunk(StreamEvent::ToolResult {
                        message: message.clone(),
                    });
                    state.session_messages.push(message);
                }
                return true;
            }
            if transcript_user_content_has_text(content) {
                on_chunk(StreamEvent::Boundary {
                    kind: StreamBoundaryKind::UserAck,
                    pending_input_id: provider.acknowledge_next_pending_input(run_id).await,
                });
            }
            true
        }
        "assistant" => {
            let message = value.get("message").unwrap_or(&value);
            if state.actual_model.is_none() {
                state.actual_model = message
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned);
            }
            let content = message.get("content").unwrap_or(&Value::Null);
            for tool in extract_tool_uses_from_assistant_content(content) {
                on_chunk(StreamEvent::ToolUse {
                    message: tool.clone(),
                });
                state.session_messages.push(tool);
            }
            let text = transcript_content_text(content);
            if !text.is_empty() {
                state.assistant_seen = true;
                state.response_text.push_str(&text);
                let entry = ProviderMessage::assistant_text(text.clone())
                    .with_timestamp(chrono::Utc::now().to_rfc3339())
                    .with_metadata_value("source", json!("claude_tty"));
                state.session_messages.push(entry);
                on_chunk(StreamEvent::Delta { text });
            }
            true
        }
        "result" => {
            state.result_seen = true;
            state.session_id = value
                .get("session_id")
                .or_else(|| value.get("sessionId"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .or_else(|| state.session_id.take());
            state.is_error = value
                .get("is_error")
                .or_else(|| value.get("isError"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state.error_message = value
                .get("error")
                .or_else(|| value.get("error_message"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            state.cost_usd = value
                .get("total_cost_usd")
                .or_else(|| value.get("totalCostUsd"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            if let Some(usage) = value.get("usage").and_then(Value::as_object) {
                state.input_tokens = usage
                    .get("input")
                    .or_else(|| usage.get("input_tokens"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                state.output_tokens = usage
                    .get("output")
                    .or_else(|| usage.get("output_tokens"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
            }
            true
        }
        _ => false,
    }
}

async fn read_complete_transcript_lines(
    path: &Path,
    offset: u64,
) -> io::Result<(Vec<String>, u64)> {
    let mut file = tokio::fs::File::open(path).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).await?;
    if buf.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let text = String::from_utf8_lossy(&buf);
    let Some(last_newline) = text.rfind('\n') else {
        return Ok((Vec::new(), 0));
    };
    let complete = &text[..last_newline];
    let consumed = complete.as_bytes().len() as u64 + 1;
    let lines = complete.lines().map(ToOwned::to_owned).collect();
    Ok((lines, consumed))
}

fn build_tty_user_prompt(options: &ProviderRunOptions, include_context: bool) -> String {
    let attachments = merged_prompt_attachments(options);
    let message = build_native_skill_prompt(&options.message, &options.metadata)
        .unwrap_or_else(|| options.message.clone());
    let message =
        prepend_initial_context_to_user_message(&message, &options.metadata, include_context);
    build_tty_prompt_from_parts(&message, &attachments)
}

fn merged_prompt_attachments(options: &ProviderRunOptions) -> Vec<PromptAttachment> {
    let mut attachments = attachments_from_metadata(&options.metadata);
    if attachments.is_empty()
        && let Some(images) = options.images.as_deref()
    {
        attachments.extend(stage_image_payloads_for_prompt("garyx-claude-tty", images));
    }
    attachments
}

fn build_tty_prompt_from_parts(message: &str, attachments: &[PromptAttachment]) -> String {
    build_prompt_message_with_attachments(message, attachments)
}

fn bracketed_paste_input(prompt: &str) -> Vec<u8> {
    let normalized = prompt.replace("\r\n", "\n").replace('\r', "\n");
    let mut bytes = Vec::with_capacity(normalized.len() + 16);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(normalized.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~\r");
    bytes
}

fn resolve_run_id(metadata: &HashMap<String, Value>) -> String {
    metadata
        .get("bridge_run_id")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("client_run_id").and_then(Value::as_str))
        .or_else(|| metadata.get("run_id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("run_{}", Uuid::new_v4()))
}

fn resolve_requested_model(
    config: &ClaudeCodeConfig,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    metadata
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            let default = config.default_model.trim();
            (!default.is_empty()).then(|| default.to_owned())
        })
}

fn metadata_string_map(metadata: &HashMap<String, Value>, key: &str) -> HashMap<String, String> {
    metadata
        .get(key)
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn claude_config_dir(env: &HashMap<String, String>) -> Result<PathBuf, BridgeError> {
    env.get("CLAUDE_CONFIG_DIR")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from))
        .or_else(|| {
            env.get("HOME")
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".claude"))
        })
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
        .ok_or_else(|| {
            BridgeError::Internal("HOME is not set; cannot locate Claude config".to_owned())
        })
}

fn claude_project_dir_name(cwd: &Path) -> String {
    let mapped = cwd
        .to_string_lossy()
        .chars()
        .map(|ch| if ch == '/' { '-' } else { ch })
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

fn resolve_tty_cwd(
    config: &ClaudeCodeConfig,
    options: &ProviderRunOptions,
) -> Result<PathBuf, BridgeError> {
    options
        .workspace_dir
        .as_ref()
        .or(config.workspace_dir.as_ref())
        .map(|ws| PathBuf::from(shellexpand::tilde(ws).as_ref()))
        .filter(|path| path.exists())
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| BridgeError::Internal("failed to resolve Claude TTY cwd".to_owned()))
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

fn transcript_user_content_has_text(content: &Value) -> bool {
    match content {
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(items) => items.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("text")
                && item
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
        }),
        _ => false,
    }
}

fn transcript_content_text(content: &Value) -> String {
    match content {
        Value::String(value) => value.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                (item.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| item.get("text").and_then(Value::as_str))
                    .flatten()
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn extract_tool_uses_from_assistant_content(content: &Value) -> Vec<ProviderMessage> {
    let Some(items) = content.as_array() else {
        return Vec::new();
    };
    let now = chrono::Utc::now().to_rfc3339();
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(|item| {
            ProviderMessage::tool_use(
                json!({
                    "tool": item.get("name").cloned().unwrap_or(Value::Null),
                    "input": item.get("input").cloned().unwrap_or(Value::Null),
                }),
                item.get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                item.get("name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            )
            .with_timestamp(now.clone())
            .with_metadata_value("source", json!("claude_tty"))
        })
        .collect()
}

fn extract_tool_results_from_user_content(content: &Value) -> Vec<ProviderMessage> {
    let Some(items) = content.as_array() else {
        return Vec::new();
    };
    let now = chrono::Utc::now().to_rfc3339();
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_result"))
        .map(|item| {
            let tool_text = item
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let mut message = ProviderMessage::tool_result(
                json!({
                    "result": item.get("content").cloned().unwrap_or(Value::Null),
                    "text": tool_text,
                }),
                item.get("tool_use_id")
                    .or_else(|| item.get("toolUseId"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                None,
                item.get("is_error")
                    .or_else(|| item.get("isError"))
                    .and_then(Value::as_bool),
            )
            .with_timestamp(now.clone())
            .with_metadata_value("source", json!("claude_tty"));
            if !tool_text.is_empty() {
                message.text = Some(tool_text);
            }
            message
        })
        .collect()
}

#[cfg(unix)]
fn default_tty_backend() -> UnixPtyBackend {
    UnixPtyBackend
}

#[cfg(not(unix))]
fn default_tty_backend() -> UnsupportedPtyBackend {
    UnsupportedPtyBackend
}

#[cfg(unix)]
struct UnixPtyBackend;

#[cfg(unix)]
struct UnixPtyProcess {
    pid: libc::pid_t,
    writer: File,
    _reader: std::thread::JoinHandle<()>,
}

#[cfg(unix)]
impl TtyBackend for UnixPtyBackend {
    fn spawn(&self, spec: &TtySpawnSpec) -> Result<Box<dyn TtyProcess>, BridgeError> {
        spawn_unix_pty(spec).map(|process| Box::new(process) as Box<dyn TtyProcess>)
    }
}

#[cfg(unix)]
impl TtyProcess for UnixPtyProcess {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    fn kill(&mut self) {
        unsafe {
            libc::kill(self.pid, libc::SIGTERM);
        }
    }
}

#[cfg(unix)]
fn spawn_unix_pty(spec: &TtySpawnSpec) -> Result<UnixPtyProcess, BridgeError> {
    use std::os::fd::FromRawFd;

    let command = CString::new(spec.command.as_str())
        .map_err(|_| BridgeError::Internal("claude command contains NUL byte".to_owned()))?;
    let mut c_args = Vec::with_capacity(spec.args.len() + 1);
    c_args.push(command.clone());
    for arg in &spec.args {
        c_args.push(
            CString::new(arg.as_str()).map_err(|_| {
                BridgeError::Internal("claude argument contains NUL byte".to_owned())
            })?,
        );
    }
    let mut argv = c_args.iter().map(|arg| arg.as_ptr()).collect::<Vec<_>>();
    argv.push(std::ptr::null());

    let cwd = CString::new(spec.cwd.to_string_lossy().as_bytes())
        .map_err(|_| BridgeError::Internal("cwd contains NUL byte".to_owned()))?;
    let env = spec
        .env
        .iter()
        .map(|(key, value)| {
            Ok((
                CString::new(key.as_str()).map_err(|_| {
                    BridgeError::Internal("environment key contains NUL byte".to_owned())
                })?,
                CString::new(value.as_str()).map_err(|_| {
                    BridgeError::Internal("environment value contains NUL byte".to_owned())
                })?,
            ))
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    let mut master_fd: libc::c_int = -1;
    let mut winsize = libc::winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let pid = unsafe {
        libc::forkpty(
            &mut master_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut winsize,
        )
    };
    if pid < 0 {
        return Err(BridgeError::Internal(format!(
            "forkpty failed: {}",
            io::Error::last_os_error()
        )));
    }
    if pid == 0 {
        unsafe {
            libc::chdir(cwd.as_ptr());
            for (key, value) in &env {
                libc::setenv(key.as_ptr(), value.as_ptr(), 1);
            }
            libc::execvp(command.as_ptr(), argv.as_ptr());
            libc::_exit(127);
        }
    }

    let master = unsafe { File::from_raw_fd(master_fd) };
    let mut reader = master
        .try_clone()
        .map_err(|error| BridgeError::Internal(format!("failed to clone pty master: {error}")))?;
    let reader_thread = std::thread::spawn(move || {
        let mut buf = [0_u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    Ok(UnixPtyProcess {
        pid,
        writer: master,
        _reader: reader_thread,
    })
}

#[cfg(not(unix))]
struct UnsupportedPtyBackend;

#[cfg(not(unix))]
impl TtyBackend for UnsupportedPtyBackend {
    fn spawn(&self, _spec: &TtySpawnSpec) -> Result<Box<dyn TtyProcess>, BridgeError> {
        Err(BridgeError::Internal(
            "claude_tty provider currently requires Unix PTY support".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use garyx_models::provider::StreamEvent;
    use std::sync::Mutex as StdMutex;

    struct FakeTtyBackend {
        transcript_path: PathBuf,
        spawns: StdMutex<Vec<TtySpawnSpec>>,
        writes: Arc<StdMutex<Vec<Vec<u8>>>>,
    }

    struct FakeTtyProcess {
        transcript_path: PathBuf,
        writes: Arc<StdMutex<Vec<Vec<u8>>>>,
    }

    struct SilentTtyBackend {
        writes: Arc<StdMutex<Vec<Vec<u8>>>>,
    }

    struct SilentTtyProcess {
        writes: Arc<StdMutex<Vec<Vec<u8>>>>,
    }

    impl TtyBackend for FakeTtyBackend {
        fn spawn(&self, spec: &TtySpawnSpec) -> Result<Box<dyn TtyProcess>, BridgeError> {
            self.spawns.lock().unwrap().push(spec.clone());
            Ok(Box::new(FakeTtyProcess {
                transcript_path: self.transcript_path.clone(),
                writes: self.writes.clone(),
            }))
        }
    }

    impl TtyProcess for FakeTtyProcess {
        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            if let Some(parent) = self.transcript_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(
                &self.transcript_path,
                concat!(
                    r#"{"type":"system","subtype":"init","session_id":"session-1"}"#,
                    "\n",
                    r#"{"type":"user","message":{"content":"hello"}}"#,
                    "\n",
                    r#"{"type":"assistant","message":{"model":"claude-test","content":[{"type":"text","text":"hi from tty"}]}}"#,
                    "\n",
                ),
            )
        }

        fn kill(&mut self) {}
    }

    impl TtyBackend for SilentTtyBackend {
        fn spawn(&self, _spec: &TtySpawnSpec) -> Result<Box<dyn TtyProcess>, BridgeError> {
            Ok(Box::new(SilentTtyProcess {
                writes: self.writes.clone(),
            }))
        }
    }

    impl TtyProcess for SilentTtyProcess {
        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }

        fn kill(&mut self) {}
    }

    fn options(temp: &tempfile::TempDir) -> ProviderRunOptions {
        ProviderRunOptions {
            thread_id: "thread::tty-test".to_owned(),
            message: "hello".to_owned(),
            workspace_dir: Some(temp.path().to_string_lossy().into_owned()),
            images: None,
            metadata: HashMap::from([("client_run_id".to_owned(), json!("run-1"))]),
        }
    }

    #[test]
    fn bracketed_paste_wraps_prompt() {
        let bytes = bracketed_paste_input("hello\nworld");
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "\u{1b}[200~hello\nworld\u{1b}[201~\r"
        );
    }

    #[test]
    fn claude_config_dir_prefers_provider_env() {
        let env = HashMap::from([
            (
                "CLAUDE_CONFIG_DIR".to_owned(),
                "/tmp/garyx-test-claude".to_owned(),
            ),
            ("HOME".to_owned(), "/tmp/ignored-home".to_owned()),
        ]);

        assert_eq!(
            claude_config_dir(&env).unwrap(),
            PathBuf::from("/tmp/garyx-test-claude")
        );
    }

    #[test]
    fn transcript_parser_extracts_assistant_text_and_tool_use() {
        let mut state = TranscriptRunState::default();
        let line = r#"{"type":"assistant","message":{"model":"claude-test","content":[{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"README.md"}},{"type":"text","text":"done"}]}}"#;
        let provider = ClaudeTtyProvider::with_backend_for_test(
            ClaudeCodeConfig::default(),
            Arc::new(FakeTtyBackend {
                transcript_path: PathBuf::from("/tmp/unused.jsonl"),
                spawns: StdMutex::new(Vec::new()),
                writes: Arc::new(StdMutex::new(Vec::new())),
            }),
            Duration::from_millis(1),
        );
        let events = Arc::new(StdMutex::new(Vec::new()));
        let captured = events.clone();
        let callback: StreamCallback = Box::new(move |event| {
            captured.lock().unwrap().push(event);
        });
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            assert!(apply_transcript_line("run-1", line, &mut state, &provider, &callback).await);
        });
        assert_eq!(state.response_text, "done");
        assert_eq!(state.actual_model.as_deref(), Some("claude-test"));
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, StreamEvent::ToolUse { .. }))
        );
    }

    #[tokio::test]
    async fn run_streaming_drives_tty_and_reads_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join(".claude");
        let transcript_path = claude_transcript_path(
            &config_dir,
            &std::fs::canonicalize(temp.path()).unwrap(),
            "session-1",
        );
        let writes = Arc::new(StdMutex::new(Vec::new()));
        let backend = Arc::new(FakeTtyBackend {
            transcript_path,
            spawns: StdMutex::new(Vec::new()),
            writes: writes.clone(),
        });
        let mut provider = ClaudeTtyProvider::with_backend_and_config_dir_for_test(
            ClaudeCodeConfig {
                mcp_base_url: String::new(),
                ..Default::default()
            },
            backend.clone(),
            Duration::from_millis(20),
            config_dir,
        );
        provider.ready = true;
        provider
            .session_ids
            .lock()
            .await
            .insert("thread::tty-test".to_owned(), "session-1".to_owned());

        let events = Arc::new(StdMutex::new(Vec::new()));
        let captured = events.clone();
        let result = provider
            .run_streaming(
                &options(&temp),
                Box::new(move |event| {
                    captured.lock().unwrap().push(event);
                }),
            )
            .await
            .unwrap();

        assert_eq!(result.response, "hi from tty");
        assert_eq!(result.sdk_session_id.as_deref(), Some("session-1"));
        assert!(
            String::from_utf8(writes.lock().unwrap()[0].clone())
                .unwrap()
                .contains("hello")
        );
        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                ..
            }
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, StreamEvent::Done))
        );
    }

    #[tokio::test]
    async fn run_streaming_error_clears_active_run_tracking() {
        let temp = tempfile::tempdir().unwrap();
        let mut provider = ClaudeTtyProvider::with_backend_and_config_dir_for_test(
            ClaudeCodeConfig {
                mcp_base_url: String::new(),
                timeout_seconds: 0.05,
                ..Default::default()
            },
            Arc::new(SilentTtyBackend {
                writes: Arc::new(StdMutex::new(Vec::new())),
            }),
            Duration::from_millis(20),
            temp.path().join(".claude"),
        );
        provider.ready = true;

        let result = provider
            .run_streaming(&options(&temp), Box::new(|_| {}))
            .await;

        assert!(matches!(result, Err(BridgeError::Timeout)));
        assert!(provider.active_runs.lock().await.is_empty());
        assert!(provider.run_pending_inputs.lock().await.is_empty());
    }

    #[test]
    fn build_args_use_interactive_flags_without_print_mode() {
        let temp = tempfile::tempdir().unwrap();
        let provider = ClaudeTtyProvider::with_backend_for_test(
            ClaudeCodeConfig {
                default_model: "sonnet".to_owned(),
                mcp_base_url: "http://127.0.0.1:31337".to_owned(),
                ..Default::default()
            },
            Arc::new(FakeTtyBackend {
                transcript_path: temp.path().join("unused.jsonl"),
                spawns: StdMutex::new(Vec::new()),
                writes: Arc::new(StdMutex::new(Vec::new())),
            }),
            Duration::from_millis(1),
        );
        let args = provider.build_claude_args(
            &options(&temp),
            "run-1",
            "session-1",
            ClaudeTtyLaunchMode::NewSession,
            temp.path(),
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--session-id", "session-1"])
        );
        assert!(args.windows(2).any(|pair| pair == ["--model", "sonnet"]));
        assert!(!args.iter().any(|arg| arg == "--print" || arg == "-p"));
        assert!(!args.iter().any(|arg| arg == "--input-format"));
        assert!(!args.iter().any(|arg| arg == "--output-format"));
    }
}
