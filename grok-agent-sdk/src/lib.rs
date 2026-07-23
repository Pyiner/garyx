//! Minimal native client for Grok Build's Agent Client Protocol stdio mode.
//!
//! This crate intentionally owns transport concerns only. Callers provide one
//! immutable copy of the ordinary provider environment for each launched
//! process; the transport never inspects or mutates credential values.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};

const ACP_PROTOCOL_VERSION: u64 = 1;
const RATE_LIMITED_RPC_CODE: i64 = -32003;
const STDERR_LIMIT_BYTES: usize = 16 * 1024;
const CANCEL_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub struct GrokClientConfig {
    pub binary: String,
    /// Owned copy of Garyx's ordinary provider environment for this process.
    pub environment: HashMap<String, String>,
    pub max_turns: Option<i64>,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for GrokClientConfig {
    fn default() -> Self {
        Self {
            binary: "grok".to_owned(),
            environment: HashMap::new(),
            max_turns: None,
            startup_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(300),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GrokCancellation {
    inner: Arc<GrokCancellationInner>,
}

#[derive(Debug, Default)]
struct GrokCancellationInner {
    cancelled: AtomicBool,
    notify: Notify,
    acknowledged: AtomicBool,
    acknowledged_notify: Notify,
    completed: AtomicBool,
    completed_notify: Notify,
}

impl GrokCancellation {
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_one();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Wait until the ACP loop has written `session/cancel` to the child.
    pub async fn wait_acknowledged(&self, wait: Duration) -> bool {
        if self.inner.acknowledged.load(Ordering::SeqCst) {
            return true;
        }
        timeout(wait, self.inner.acknowledged_notify.notified())
            .await
            .is_ok()
    }

    /// Wait until the ACP run has consumed the cancellation response, so the
    /// child has processed the request before a caller may tear it down.
    pub async fn wait_completed(&self, wait: Duration) -> bool {
        if self.inner.completed.load(Ordering::SeqCst) {
            return true;
        }
        timeout(wait, self.inner.completed_notify.notified())
            .await
            .is_ok()
    }

    fn acknowledge(&self) {
        self.inner.acknowledged.store(true, Ordering::SeqCst);
        self.inner.acknowledged_notify.notify_one();
    }

    fn complete(&self) {
        self.inner.completed.store(true, Ordering::SeqCst);
        self.inner.completed_notify.notify_waiters();
        // Retain a permit for a waiter that observed `completed == false`
        // immediately before the store but had not registered with Notify yet.
        self.inner.completed_notify.notify_one();
    }

    async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.inner.notify.notified().await;
    }
}

#[derive(Debug, Clone)]
pub struct GrokRunRequest {
    pub cwd: PathBuf,
    pub prompt: String,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    /// Client rules carried by Grok Build's session-scoped ACP `rules`
    /// extension. Grok folds these into its native system prompt.
    pub rules: Option<String>,
    /// MCP servers serialized into the standard ACP `mcpServers` array on
    /// both `session/new` and `session/load`.
    pub mcp_servers: Vec<GrokMcpServer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokMcpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokMcpEnvVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrokMcpServer {
    Http {
        name: String,
        url: String,
        headers: Vec<GrokMcpHeader>,
    },
    Sse {
        name: String,
        url: String,
        headers: Vec<GrokMcpHeader>,
    },
    Stdio {
        name: String,
        command: String,
        args: Vec<String>,
        env: Vec<GrokMcpEnvVariable>,
    },
}

impl GrokMcpServer {
    fn to_acp_value(&self) -> Value {
        match self {
            Self::Http { name, url, headers } => json!({
                "type": "http",
                "name": name,
                "url": url,
                "headers": headers.iter().map(|header| json!({
                    "name": header.name,
                    "value": header.value,
                })).collect::<Vec<_>>(),
            }),
            Self::Sse { name, url, headers } => json!({
                "type": "sse",
                "name": name,
                "url": url,
                "headers": headers.iter().map(|header| json!({
                    "name": header.name,
                    "value": header.value,
                })).collect::<Vec<_>>(),
            }),
            Self::Stdio {
                name,
                command,
                args,
                env,
            } => json!({
                "name": name,
                "command": command,
                "args": args,
                "env": env.iter().map(|variable| json!({
                    "name": variable.name,
                    "value": variable.value,
                })).collect::<Vec<_>>(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GrokEvent {
    SessionBound { session_id: String },
    SessionUpdate { update: Value },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrokRunOutput {
    pub session_id: String,
    pub actual_model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrokReasoningEffort {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub recommended: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrokModel {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub recommended: bool,
    pub default_reasoning_effort: Option<String>,
    pub reasoning_efforts: Vec<GrokReasoningEffort>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrokModelCatalog {
    pub current_model_id: Option<String>,
    pub models: Vec<GrokModel>,
}

#[derive(thiserror::Error, Debug)]
pub enum GrokError {
    #[error("failed to start Grok Build: {0}")]
    Spawn(String),
    #[error("Grok Build stdio transport failed: {0}")]
    Transport(String),
    #[error("Grok Build returned invalid ACP data: {0}")]
    Protocol(String),
    #[error("Grok Build authentication is unavailable: {0}")]
    Authentication(String),
    #[error("Grok Build ACP request `{method}` failed ({code}): {message}")]
    Rpc {
        method: String,
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("Grok Build request timed out")]
    Timeout,
    #[error("Grok Build request was cancelled")]
    Cancelled,
}

impl GrokError {
    pub fn rate_limit_kind(&self) -> Option<&'static str> {
        let Self::Rpc { code, data, .. } = self else {
            return None;
        };
        if *code == RATE_LIMITED_RPC_CODE
            || rpc_http_status(data.as_ref()) == Some(429)
            || structured_rate_limit_kind(data.as_ref()) == Some("rate_limited")
        {
            return Some("rate_limited");
        }
        if matches!(rpc_http_status(data.as_ref()), Some(503 | 529))
            || structured_rate_limit_kind(data.as_ref()) == Some("capacity")
        {
            return Some("capacity");
        }
        None
    }

    pub fn provider_message(&self) -> Option<&str> {
        match self {
            Self::Rpc { message, .. } => Some(message),
            _ => None,
        }
    }
}

fn rpc_http_status(data: Option<&Value>) -> Option<i64> {
    structured_i64(
        data?,
        &[
            "http_status",
            "httpStatus",
            "http_status_code",
            "httpStatusCode",
            "status_code",
            "statusCode",
            "status",
        ],
        3,
    )
}

fn structured_rate_limit_kind(data: Option<&Value>) -> Option<&'static str> {
    let kind = structured_string(
        data?,
        &["error_type", "errorType", "kind", "type", "reason", "code"],
        3,
    )?;
    let normalized = kind
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_");
    match normalized.as_str() {
        "rate_limit"
        | "rate_limited"
        | "usage_pool_exhausted"
        | "usage_limit_reached"
        | "global_rate_limit"
        | "concurrency_limit" => Some("rate_limited"),
        "capacity" | "service_unavailable" | "overloaded" => Some("capacity"),
        _ => None,
    }
}

fn structured_i64(value: &Value, keys: &[&str], depth: usize) -> Option<i64> {
    if depth == 0 {
        return None;
    }
    if let Some(text) = value.as_str() {
        let parsed = serde_json::from_str::<Value>(text).ok()?;
        return structured_i64(&parsed, keys, depth - 1);
    }
    let object = value.as_object()?;
    for key in keys {
        if let Some(candidate) = object.get(*key) {
            if let Some(number) = candidate.as_i64() {
                return Some(number);
            }
            if let Some(number) = candidate
                .as_str()
                .and_then(|number| number.parse::<i64>().ok())
            {
                return Some(number);
            }
        }
    }
    ["error", "details", "cause"]
        .into_iter()
        .filter_map(|key| object.get(key))
        .find_map(|nested| structured_i64(nested, keys, depth - 1))
}

fn structured_string(value: &Value, keys: &[&str], depth: usize) -> Option<String> {
    if depth == 0 {
        return None;
    }
    if let Some(text) = value.as_str() {
        let parsed = serde_json::from_str::<Value>(text).ok()?;
        return structured_string(&parsed, keys, depth - 1);
    }
    let object = value.as_object()?;
    for key in keys {
        if let Some(candidate) = object.get(*key).and_then(Value::as_str) {
            return Some(candidate.to_owned());
        }
    }
    ["error", "details", "cause"]
        .into_iter()
        .filter_map(|key| object.get(key))
        .find_map(|nested| structured_string(nested, keys, depth - 1))
}

#[derive(Clone, Debug)]
pub struct GrokClient {
    config: GrokClientConfig,
}

impl GrokClient {
    pub fn new(config: GrokClientConfig) -> Self {
        Self { config }
    }

    pub async fn discover_models(&self, cwd: &Path) -> Result<GrokModelCatalog, GrokError> {
        let mut transport = Transport::spawn(&self.config, cwd, None, None).await?;
        let initialized = transport
            .initialize(self.config.startup_timeout, false)
            .await;
        let result = initialized.map(|value| parse_model_catalog(&value));
        transport.finish().await;
        result
    }

    pub async fn run<F>(
        &self,
        request: GrokRunRequest,
        cancellation: GrokCancellation,
        mut on_event: F,
    ) -> Result<GrokRunOutput, GrokError>
    where
        F: FnMut(GrokEvent) + Send,
    {
        let mut transport = Transport::spawn(
            &self.config,
            &request.cwd,
            request.model.as_deref(),
            request.reasoning_effort.as_deref(),
        )
        .await?;

        let result = async {
            let initialized = transport
                .initialize(self.config.startup_timeout, true)
                .await?;
            let current_model = parse_model_catalog(&initialized).current_model_id;
            let mcp_servers = request
                .mcp_servers
                .iter()
                .map(GrokMcpServer::to_acp_value)
                .collect::<Vec<_>>();
            let rules = request
                .rules
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());

            let session_id = if let Some(session_id) = request
                .session_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let value = transport
                    .request(
                        "session/load",
                        json!({
                            "sessionId": session_id,
                            "cwd": request.cwd,
                            "mcpServers": mcp_servers,
                            "_meta": session_meta(rules, true),
                        }),
                        self.config.request_timeout,
                    )
                    .await?;
                response_session_id(&value).unwrap_or_else(|| session_id.to_owned())
            } else {
                let value = transport
                    .request(
                        "session/new",
                        json!({
                            "cwd": request.cwd,
                            "mcpServers": mcp_servers,
                            "_meta": session_meta(rules, false),
                        }),
                        self.config.request_timeout,
                    )
                    .await?;
                response_session_id(&value).ok_or_else(|| {
                    GrokError::Protocol("session/new returned no sessionId".to_owned())
                })?
            };
            on_event(GrokEvent::SessionBound {
                session_id: session_id.clone(),
            });

            if let Some(model) = request
                .model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let set_model = transport
                    .request(
                        "session/set_model",
                        json!({
                            "sessionId": session_id,
                            "modelId": model,
                            "_meta": request.reasoning_effort.as_deref().map(|effort| {
                                json!({ "reasoningEffort": effort })
                            }).unwrap_or_else(|| json!({})),
                        }),
                        self.config.request_timeout,
                    )
                    .await;
                if !matches!(&set_model, Err(GrokError::Rpc { code: -32601, .. })) {
                    set_model?;
                }
            }

            let prompt_id = transport
                .send_request(
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{ "type": "text", "text": request.prompt }],
                    }),
                )
                .await?;
            let mut deadline = Instant::now() + self.config.request_timeout;
            let mut cancellation_sent = false;
            let prompt_response = loop {
                if Instant::now() >= deadline {
                    break Err(GrokError::Timeout);
                }
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled(), if !cancellation_sent => {
                        transport.send_notification(
                            "session/cancel",
                            json!({ "sessionId": session_id }),
                        ).await?;
                        cancellation.acknowledge();
                        cancellation_sent = true;
                        deadline = Instant::now()
                            + self.config.request_timeout.min(CANCEL_SETTLEMENT_TIMEOUT);
                    }
                    message = transport.recv_until(deadline) => {
                        let message = match message {
                            Ok(message) => message,
                            Err(_) if cancellation_sent => break Ok(Value::Null),
                            Err(error) => break Err(error),
                        };
                        if !cancellation_sent {
                            // Prompt streams use inactivity semantics: any ACP
                            // frame proves that the child is still making
                            // progress. One-shot requests keep their fixed
                            // deadline in `Transport::request`.
                            deadline = Instant::now() + self.config.request_timeout;
                        }
                        if is_server_request(&message) {
                            transport.reject_server_request(&message).await?;
                            continue;
                        }
                        if message.get("method").and_then(Value::as_str) == Some("session/update") {
                            if let Some(update) = message.get("params").and_then(|params| params.get("update")) {
                                on_event(GrokEvent::SessionUpdate { update: update.clone() });
                            }
                            continue;
                        }
                        if response_id(&message) == Some(prompt_id) {
                            break Transport::response_result("session/prompt", message);
                        }
                    }
                }
            };
            let prompt_response = match prompt_response {
                Ok(value) => value,
                Err(_) if cancellation_sent => Value::Null,
                Err(error) => return Err(error),
            };
            let meta = prompt_response.get("_meta").unwrap_or(&Value::Null);
            Ok(GrokRunOutput {
                session_id,
                actual_model: value_string(meta, &["modelId", "model_id"])
                    .or(request.model)
                    .or(current_model),
                input_tokens: value_i64(meta, &["inputTokens", "input_tokens"]).unwrap_or(0),
                output_tokens: value_i64(meta, &["outputTokens", "output_tokens"]).unwrap_or(0),
                stop_reason: if cancellation_sent {
                    // The local cancellation signal is authoritative even if
                    // the child races it with a natural-looking end_turn.
                    Some("cancelled".to_owned())
                } else {
                    value_string(&prompt_response, &["stopReason", "stop_reason"])
                },
            })
        }
        .await;

        cancellation.complete();
        transport.finish().await;
        result
    }
}

struct Transport {
    child: Child,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    incoming: mpsc::Receiver<Result<Value, GrokError>>,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    stderr: Arc<std::sync::Mutex<String>>,
    next_id: u64,
}

impl Transport {
    async fn spawn(
        config: &GrokClientConfig,
        cwd: &Path,
        model: Option<&str>,
        reasoning_effort: Option<&str>,
    ) -> Result<Self, GrokError> {
        let binary = config.binary.trim();
        let binary = if binary.is_empty() { "grok" } else { binary };
        let mut command = Command::new(binary);
        command.arg("--no-auto-update");
        if let Some(max_turns) = config.max_turns.filter(|value| *value > 0) {
            command.arg("--max-turns").arg(max_turns.to_string());
        }
        command
            .arg("agent")
            .arg("--always-approve")
            .arg("--no-leader");
        if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
            command.arg("--model").arg(model);
        }
        if let Some(effort) = reasoning_effort
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            command.arg("--reasoning-effort").arg(effort);
        }
        command
            .arg("stdio")
            .current_dir(cwd)
            .envs(&config.environment)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| GrokError::Spawn(format!("`{binary}`: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| GrokError::Transport("child stdin unavailable".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| GrokError::Transport("child stdout unavailable".to_owned()))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| GrokError::Transport("child stderr unavailable".to_owned()))?;

        let (incoming_tx, incoming) = mpsc::channel(64);
        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) if line.trim().is_empty() => continue,
                    Ok(Some(line)) => {
                        let parsed = serde_json::from_str::<Value>(&line).map_err(|error| {
                            GrokError::Protocol(format!("invalid JSON-RPC frame: {error}"))
                        });
                        if incoming_tx.send(parsed).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = incoming_tx
                            .send(Err(GrokError::Transport(format!(
                                "failed reading stdout: {error}"
                            ))))
                            .await;
                        break;
                    }
                }
            }
        });

        let stderr = Arc::new(std::sync::Mutex::new(String::new()));
        let stderr_copy = Arc::clone(&stderr);
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr_pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut captured = stderr_copy.lock().expect("Grok stderr lock poisoned");
                if captured.len() >= STDERR_LIMIT_BYTES {
                    continue;
                }
                if !captured.is_empty() {
                    captured.push('\n');
                }
                let remaining = STDERR_LIMIT_BYTES.saturating_sub(captured.len());
                let end = if line.len() <= remaining {
                    line.len()
                } else {
                    line.char_indices()
                        .map(|(index, _)| index)
                        .take_while(|index| *index <= remaining)
                        .last()
                        .unwrap_or(0)
                };
                captured.push_str(&line[..end]);
            }
        });

        Ok(Self {
            child,
            stdin: Arc::new(Mutex::new(Some(stdin))),
            incoming,
            stdout_task,
            stderr_task,
            stderr,
            next_id: 1,
        })
    }

    async fn initialize(
        &mut self,
        request_timeout: Duration,
        authenticate: bool,
    ) -> Result<Value, GrokError> {
        let initialized = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": ACP_PROTOCOL_VERSION,
                    "clientCapabilities": {},
                    "clientInfo": {
                        "name": "garyx",
                        "title": "Garyx",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
                request_timeout,
            )
            .await?;
        if !authenticate {
            return Ok(initialized);
        }
        if let Some(method_id) = advertised_auth_method(&initialized)? {
            self.request(
                "authenticate",
                json!({ "methodId": method_id }),
                request_timeout,
            )
            .await?;
        }
        Ok(initialized)
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        request_timeout: Duration,
    ) -> Result<Value, GrokError> {
        let id = self.send_request(method, params).await?;
        let deadline = Instant::now() + request_timeout;
        loop {
            let message = self.recv_until(deadline).await?;
            if is_server_request(&message) {
                self.reject_server_request(&message).await?;
                continue;
            }
            if response_id(&message) == Some(id) {
                return Self::response_result(method, message);
            }
        }
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<u64, GrokError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;
        Ok(id)
    }

    async fn send_notification(&self, method: &str, params: Value) -> Result<(), GrokError> {
        self.send(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn send(&self, value: Value) -> Result<(), GrokError> {
        let mut line =
            serde_json::to_vec(&value).map_err(|error| GrokError::Protocol(error.to_string()))?;
        line.push(b'\n');
        let mut guard = self.stdin.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| GrokError::Transport("child stdin is closed".to_owned()))?;
        stdin
            .write_all(&line)
            .await
            .map_err(|error| GrokError::Transport(format!("failed writing stdin: {error}")))?;
        stdin
            .flush()
            .await
            .map_err(|error| GrokError::Transport(format!("failed flushing stdin: {error}")))
    }

    async fn recv_until(&mut self, deadline: Instant) -> Result<Value, GrokError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        timeout(remaining, self.incoming.recv())
            .await
            .map_err(|_| GrokError::Timeout)?
            .ok_or_else(|| {
                let captured = self.stderr.lock().expect("Grok stderr lock poisoned");
                let detail = captured.trim();
                if detail.is_empty() {
                    GrokError::Transport("Grok Build closed stdout".to_owned())
                } else {
                    GrokError::Transport(format!("Grok Build closed stdout: {detail}"))
                }
            })?
    }

    async fn reject_server_request(&self, request: &Value) -> Result<(), GrokError> {
        let Some(id) = request.get("id") else {
            return Ok(());
        };
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": "Garyx does not implement this ACP client method",
            },
        }))
        .await
    }

    fn response_result(method: &str, response: Value) -> Result<Value, GrokError> {
        if let Some(error) = response.get("error") {
            return Err(GrokError::Rpc {
                method: method.to_owned(),
                code: error.get("code").and_then(Value::as_i64).unwrap_or(-32000),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown ACP error")
                    .to_owned(),
                data: error.get("data").cloned(),
            });
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| GrokError::Protocol(format!("`{method}` returned no result")))
    }

    async fn finish(mut self) {
        self.stdin.lock().await.take();
        if timeout(Duration::from_secs(2), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
            let _ = self.child.wait().await;
        }
        self.stdout_task.abort();
        self.stderr_task.abort();
    }
}

fn is_server_request(value: &Value) -> bool {
    value.get("id").is_some() && value.get("method").is_some()
}

fn response_id(value: &Value) -> Option<u64> {
    value.get("id").and_then(Value::as_u64)
}

fn response_session_id(value: &Value) -> Option<String> {
    value_string(value, &["sessionId", "session_id"])
}

fn session_meta(rules: Option<&str>, no_replay: bool) -> Value {
    let mut meta = serde_json::Map::new();
    if no_replay {
        meta.insert("noReplay".to_owned(), Value::Bool(true));
    }
    if let Some(rules) = rules {
        meta.insert("rules".to_owned(), Value::String(rules.to_owned()));
    }
    Value::Object(meta)
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn value_i64(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_i64))
}

fn advertised_auth_method(initialized: &Value) -> Result<Option<String>, GrokError> {
    let methods = initialized
        .get("authMethods")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if methods.is_empty() {
        return Ok(None);
    }
    let first_method_id = methods[0]
        .as_str()
        .or_else(|| methods[0].get("id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            GrokError::Authentication(
                "Grok advertised an authentication method without an id".to_owned(),
            )
        })?;
    // The ordinary Grok process owns its authentication choice. ACP metadata
    // carries that choice separately because authMethods ordering preserves
    // compatibility (for example xai.api_key can remain first while a cached
    // OAuth session is the declared default). Garyx must follow the declared
    // method without inspecting credentials or applying its own preference.
    let declared_default = initialized
        .get("_meta")
        .and_then(|meta| value_string(meta, &["defaultAuthMethodId", "default_auth_method_id"]))
        .or_else(|| {
            value_string(
                initialized,
                &["defaultAuthMethodId", "default_auth_method_id"],
            )
        });
    if let Some(default) = declared_default
        && methods.iter().any(|method| {
            method
                .as_str()
                .or_else(|| method.get("id").and_then(Value::as_str))
                == Some(default.as_str())
        })
    {
        return Ok(Some(default));
    }
    Ok(Some(first_method_id))
}

pub fn parse_model_catalog(initialized: &Value) -> GrokModelCatalog {
    let state = initialized
        .get("_meta")
        .and_then(|meta| meta.get("modelState"))
        .or_else(|| initialized.get("modelState"))
        .unwrap_or(&Value::Null);
    let current_model_id = value_string(state, &["currentModelId", "current_model_id"]);
    let entries = state
        .get("availableModels")
        .or_else(|| state.get("available_models"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let models = entries
        .into_iter()
        .filter_map(|entry| {
            let id = value_string(&entry, &["modelId", "model_id", "id"])?;
            let model_meta = entry.get("_meta").unwrap_or(&Value::Null);
            let default_reasoning_effort = value_string(
                model_meta,
                &[
                    "reasoningEffort",
                    "reasoning_effort",
                    "defaultReasoningEffort",
                ],
            );
            let reasoning_entries = model_meta
                .get("reasoningEfforts")
                .or_else(|| model_meta.get("reasoning_efforts"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let reasoning_efforts = reasoning_entries
                .into_iter()
                .filter_map(|effort| {
                    let id = value_string(&effort, &["id", "value"])?;
                    Some(GrokReasoningEffort {
                        label: value_string(&effort, &["label", "name"])
                            .unwrap_or_else(|| id.clone()),
                        description: value_string(&effort, &["description"]),
                        recommended: effort
                            .get("default")
                            .or_else(|| effort.get("recommended"))
                            .and_then(Value::as_bool)
                            .unwrap_or_else(|| default_reasoning_effort.as_deref() == Some(&id)),
                        id,
                    })
                })
                .collect();
            Some(GrokModel {
                label: value_string(&entry, &["name", "label"]).unwrap_or_else(|| id.clone()),
                description: value_string(&entry, &["description"]),
                recommended: current_model_id.as_deref() == Some(&id),
                default_reasoning_effort,
                reasoning_efforts,
                id,
            })
        })
        .collect();
    GrokModelCatalog {
        current_model_id,
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parses_models_and_reasoning_efforts_from_initialize_metadata() {
        let catalog = parse_model_catalog(&json!({
            "_meta": {
                "modelState": {
                    "currentModelId": "grok-code-fast-1",
                    "availableModels": [{
                        "modelId": "grok-code-fast-1",
                        "name": "Grok Code Fast 1",
                        "description": "Fast coding model",
                        "_meta": {
                            "reasoningEffort": "high",
                            "reasoningEfforts": [
                                {"value": "low", "label": "Low"},
                                {"value": "high", "label": "High", "default": true}
                            ]
                        }
                    }]
                }
            }
        }));

        assert_eq!(
            catalog.current_model_id.as_deref(),
            Some("grok-code-fast-1")
        );
        assert_eq!(catalog.models.len(), 1);
        assert!(catalog.models[0].recommended);
        assert_eq!(catalog.models[0].reasoning_efforts.len(), 2);
        assert!(catalog.models[0].reasoning_efforts[1].recommended);
    }

    #[test]
    fn rate_limits_are_classified_only_from_structured_rpc_data() {
        let coded = GrokError::Rpc {
            method: "session/prompt".to_owned(),
            code: RATE_LIMITED_RPC_CODE,
            message: "Rate limited".to_owned(),
            data: None,
        };
        let capacity = GrokError::Rpc {
            method: "session/prompt".to_owned(),
            code: -32000,
            message: "upstream unavailable".to_owned(),
            data: Some(json!({"http_status": 529})),
        };
        let structured_usage = GrokError::Rpc {
            method: "session/prompt".to_owned(),
            code: -32000,
            message: "request failed".to_owned(),
            data: Some(json!({"details": {"error_type": "usage_limit_reached"}})),
        };
        let structured_capacity = GrokError::Rpc {
            method: "session/prompt".to_owned(),
            code: -32000,
            message: "request failed".to_owned(),
            data: Some(json!("{\"error\":{\"type\":\"service_unavailable\"}}")),
        };
        let rpc_text_only = GrokError::Rpc {
            method: "session/prompt".to_owned(),
            code: -32000,
            message: "HTTP 429 rate limited".to_owned(),
            data: None,
        };
        let text_only = GrokError::Transport("HTTP 429 rate limited".to_owned());

        assert_eq!(coded.rate_limit_kind(), Some("rate_limited"));
        assert_eq!(capacity.rate_limit_kind(), Some("capacity"));
        assert_eq!(structured_usage.rate_limit_kind(), Some("rate_limited"));
        assert_eq!(structured_capacity.rate_limit_kind(), Some("capacity"));
        assert_eq!(rpc_text_only.rate_limit_kind(), None);
        assert_eq!(text_only.rate_limit_kind(), None);
    }

    #[test]
    fn authentication_uses_groks_declared_default_acp_method() {
        let initialized = json!({
            "authMethods": [
                {"id": "xai.api_key"},
                {"id": "cached_token"}
            ],
            "_meta": {"defaultAuthMethodId": "cached_token"}
        });

        assert_eq!(
            advertised_auth_method(&initialized).expect("valid auth method"),
            Some("cached_token".to_owned())
        );
    }

    #[test]
    fn authentication_falls_back_to_first_method_without_a_declared_default() {
        let initialized = json!({
            "authMethods": [
                {"id": "xai.api_key"},
                {"id": "cached_token"}
            ]
        });

        assert_eq!(
            advertised_auth_method(&initialized).expect("valid auth method"),
            Some("xai.api_key".to_owned())
        );
    }

    fn fake_grok(script_body: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fake-grok");
        std::fs::write(&path, format!("#!/bin/sh\n{script_body}\n")).expect("write script");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("permissions");
        (dir, path.to_string_lossy().into_owned())
    }

    #[tokio::test]
    async fn streams_new_native_session_and_preserves_environment_snapshot() {
        let (_dir, binary) = fake_grok(
            r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"authMethods":[{"id":"xai.api_key"}],"_meta":{"modelState":{"currentModelId":"grok-test","availableModels":[]}}}}' ;;
    *'"method":"authenticate"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{}}' ;;
    *'"method":"session/new"'*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"sessionId":"native-session-1"}}' ;;
    *'"method":"session/prompt"'*)
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"native-session-1\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"$GROK_TEST_MARKER\"}}}}"
      printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{"stopReason":"end_turn","_meta":{"sessionId":"native-session-1","modelId":"grok-test","inputTokens":2,"outputTokens":3}}}' ;;
  esac
done
"#,
        );
        let client = GrokClient::new(GrokClientConfig {
            binary,
            environment: HashMap::from([(
                "GROK_TEST_MARKER".to_owned(),
                "snapshot-value".to_owned(),
            )]),
            max_turns: None,
            startup_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
        });
        let mut events = Vec::new();
        let output = client
            .run(
                GrokRunRequest {
                    cwd: std::env::current_dir().expect("cwd"),
                    prompt: "hello".to_owned(),
                    session_id: None,
                    model: None,
                    reasoning_effort: None,
                    rules: None,
                    mcp_servers: Vec::new(),
                },
                GrokCancellation::default(),
                |event| events.push(event),
            )
            .await
            .expect("run succeeds");

        assert_eq!(output.session_id, "native-session-1");
        assert_eq!(output.actual_model.as_deref(), Some("grok-test"));
        assert_eq!(output.input_tokens, 2);
        assert_eq!(output.output_tokens, 3);
        assert_eq!(
            events,
            vec![
                GrokEvent::SessionBound {
                    session_id: "native-session-1".to_owned(),
                },
                GrokEvent::SessionUpdate {
                    update: json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "snapshot-value"}
                    }),
                },
            ]
        );
    }

    #[tokio::test]
    async fn session_requests_carry_native_system_rules_and_mcp_servers() {
        let capture_dir = tempfile::tempdir().expect("capture dir");
        let capture_path = capture_dir.path().join("acp-requests.jsonl");
        let (_binary_dir, binary) = fake_grok(
            r#"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$GROK_ACP_CAPTURE"
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;;
    *'"method":"session/new"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"native-context-session"}}' ;;
    *'"method":"session/load"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{}}' ;;
    *'"method":"session/prompt"'*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}' ;;
  esac
done
"#,
        );
        let client = GrokClient::new(GrokClientConfig {
            binary,
            environment: HashMap::from([(
                "GROK_ACP_CAPTURE".to_owned(),
                capture_path.to_string_lossy().into_owned(),
            )]),
            max_turns: None,
            startup_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
        });
        let mcp_servers = vec![
            GrokMcpServer::Http {
                name: "garyx".to_owned(),
                url: "http://127.0.0.1:31337/mcp/thread/run".to_owned(),
                headers: vec![GrokMcpHeader {
                    name: "X-Run-Id".to_owned(),
                    value: "run".to_owned(),
                }],
            },
            GrokMcpServer::Sse {
                name: "events".to_owned(),
                url: "https://mcp.example.com/events".to_owned(),
                headers: Vec::new(),
            },
            GrokMcpServer::Stdio {
                name: "local".to_owned(),
                command: "/usr/bin/example-mcp".to_owned(),
                args: vec!["--stdio".to_owned()],
                env: vec![GrokMcpEnvVariable {
                    name: "MODE".to_owned(),
                    value: "test".to_owned(),
                }],
            },
        ];

        for session_id in [None, Some("native-context-session".to_owned())] {
            client
                .run(
                    GrokRunRequest {
                        cwd: std::env::current_dir().expect("cwd"),
                        prompt: "ordinary user message".to_owned(),
                        session_id,
                        model: None,
                        reasoning_effort: None,
                        rules: Some("native system instructions".to_owned()),
                        mcp_servers: mcp_servers.clone(),
                    },
                    GrokCancellation::default(),
                    |_| {},
                )
                .await
                .expect("native context run succeeds");
        }

        let requests = std::fs::read_to_string(&capture_path)
            .expect("capture")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("valid JSON-RPC capture"))
            .collect::<Vec<_>>();
        let new_request = requests
            .iter()
            .find(|request| request.get("method") == Some(&json!("session/new")))
            .expect("session/new request");
        let load_request = requests
            .iter()
            .find(|request| request.get("method") == Some(&json!("session/load")))
            .expect("session/load request");
        for request in [new_request, load_request] {
            assert_eq!(
                request["params"]["_meta"]["rules"],
                "native system instructions"
            );
            assert_eq!(
                request["params"]["mcpServers"],
                json!([
                    {
                        "type": "http",
                        "name": "garyx",
                        "url": "http://127.0.0.1:31337/mcp/thread/run",
                        "headers": [{"name": "X-Run-Id", "value": "run"}]
                    },
                    {
                        "type": "sse",
                        "name": "events",
                        "url": "https://mcp.example.com/events",
                        "headers": []
                    },
                    {
                        "name": "local",
                        "command": "/usr/bin/example-mcp",
                        "args": ["--stdio"],
                        "env": [{"name": "MODE", "value": "test"}]
                    }
                ])
            );
        }
        assert_eq!(load_request["params"]["_meta"]["noReplay"], true);
        assert!(new_request["params"]["_meta"].get("noReplay").is_none());
        let prompt_requests = requests
            .iter()
            .filter(|request| request.get("method") == Some(&json!("session/prompt")))
            .collect::<Vec<_>>();
        assert_eq!(prompt_requests.len(), 2);
        assert!(
            prompt_requests.iter().all(|request| {
                request["params"]["prompt"][0]["text"] == "ordinary user message"
            })
        );
    }

    #[tokio::test]
    async fn active_prompt_streams_may_outlive_the_inactivity_timeout() {
        let (_dir, binary) = fake_grok(
            r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;;
    *'"method":"session/new"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"long-session"}}' ;;
    *'"method":"session/prompt"'*)
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"long-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"still "}}}}'
      sleep 1.2
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"long-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}}}'
      sleep 1.2
      printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}' ;;
  esac
done
"#,
        );
        let client = GrokClient::new(GrokClientConfig {
            binary,
            environment: HashMap::new(),
            max_turns: None,
            startup_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(2),
        });
        let mut chunks = String::new();
        let output = client
            .run(
                GrokRunRequest {
                    cwd: std::env::current_dir().expect("cwd"),
                    prompt: "keep working".to_owned(),
                    session_id: None,
                    model: None,
                    reasoning_effort: None,
                    rules: None,
                    mcp_servers: Vec::new(),
                },
                GrokCancellation::default(),
                |event| {
                    if let GrokEvent::SessionUpdate { update } = event
                        && let Some(text) = update
                            .get("content")
                            .and_then(|content| content.get("text"))
                            .and_then(Value::as_str)
                    {
                        chunks.push_str(text);
                    }
                },
            )
            .await
            .expect("active updates keep the prompt alive");

        assert_eq!(output.session_id, "long-session");
        assert_eq!(chunks, "still working");
    }

    #[tokio::test]
    async fn one_shot_requests_keep_a_fixed_deadline_despite_notifications() {
        let (_dir, binary) = fake_grok(
            r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      i=0
      while [ "$i" -lt 5 ]; do
        printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk"}}}'
        sleep 0.3
        i=$((i + 1))
      done ;;
  esac
done
"#,
        );
        let client = GrokClient::new(GrokClientConfig {
            binary,
            environment: HashMap::new(),
            max_turns: None,
            startup_timeout: Duration::from_secs(1),
            request_timeout: Duration::from_secs(5),
        });

        let error = client
            .discover_models(&std::env::current_dir().expect("cwd"))
            .await
            .expect_err("initialize notifications do not extend its deadline");
        assert!(matches!(error, GrokError::Timeout));
    }

    #[tokio::test]
    async fn loads_exact_native_session_and_sends_acp_cancel() {
        let (_dir, binary) = fake_grok(
            r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"authMethods":[{"id":"cached_token"}]}}' ;;
    *'"method":"authenticate"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{}}' ;;
    *'"method":"session/load"'*)
      case "$line" in *'native-resume-id'*) ;; *) exit 3 ;; esac
      case "$line" in *'"noReplay":true'*) ;; *) exit 4 ;; esac
      printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{}}' ;;
    *'"method":"session/cancel"'*'native-resume-id'*) printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{"stopReason":"end_turn"}}' ;;
  esac
done
"#,
        );
        let client = GrokClient::new(GrokClientConfig {
            binary,
            environment: HashMap::new(),
            max_turns: None,
            startup_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
        });
        let cancellation = GrokCancellation::default();
        let cancellation_observer = cancellation.clone();
        let cancellation_for_task = cancellation.clone();
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancellation_for_task.cancel();
        });
        let mut events = Vec::new();
        let output = client
            .run(
                GrokRunRequest {
                    cwd: std::env::current_dir().expect("cwd"),
                    prompt: "wait".to_owned(),
                    session_id: Some("native-resume-id".to_owned()),
                    model: None,
                    reasoning_effort: None,
                    rules: None,
                    mcp_servers: Vec::new(),
                },
                cancellation,
                |event| events.push(event),
            )
            .await
            .expect("cancelled prompt returns its partial native result");
        cancel_task.await.expect("cancel task");

        assert_eq!(output.session_id, "native-resume-id");
        assert_eq!(output.stop_reason.as_deref(), Some("cancelled"));
        assert!(
            cancellation_observer
                .wait_acknowledged(Duration::from_millis(10))
                .await
        );
        assert!(
            cancellation_observer
                .wait_completed(Duration::from_millis(10))
                .await
        );
        assert_eq!(
            events,
            vec![GrokEvent::SessionBound {
                session_id: "native-resume-id".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn failed_native_resume_never_falls_back_to_a_fresh_session() {
        let (_dir, binary) = fake_grok(
            r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"authMethods":[{"id":"cached_token"}]}}' ;;
    *'"method":"authenticate"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{}}' ;;
    *'"method":"session/load"'*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"error":{"code":-32010,"message":"session not found"}}' ;;
    *'"method":"session/new"'*) exit 9 ;;
  esac
done
"#,
        );
        let client = GrokClient::new(GrokClientConfig {
            binary,
            environment: HashMap::new(),
            max_turns: None,
            startup_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
        });
        let mut events = Vec::new();
        let result = client
            .run(
                GrokRunRequest {
                    cwd: std::env::current_dir().expect("cwd"),
                    prompt: "resume".to_owned(),
                    session_id: Some("missing-native-session".to_owned()),
                    model: None,
                    reasoning_effort: None,
                    rules: None,
                    mcp_servers: Vec::new(),
                },
                GrokCancellation::default(),
                |event| events.push(event),
            )
            .await;

        assert!(matches!(
            result,
            Err(GrokError::Rpc {
                ref method,
                code: -32010,
                ..
            }) if method == "session/load"
        ));
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn multibyte_stderr_is_capped_by_bytes() {
        let (_dir, binary) = fake_grok(
            r#"
i=0
while [ "$i" -lt 20000 ]; do
  printf 'é' >&2
  i=$((i + 1))
done
printf '\n' >&2
exit 8
"#,
        );
        let client = GrokClient::new(GrokClientConfig {
            binary,
            environment: HashMap::new(),
            max_turns: None,
            startup_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
        });

        let error = client
            .discover_models(&std::env::current_dir().expect("cwd"))
            .await
            .expect_err("closed stdout includes captured stderr");
        let GrokError::Transport(message) = error else {
            panic!("unexpected error: {error}");
        };
        assert!(
            message.len() <= "Grok Build closed stdout: ".len() + STDERR_LIMIT_BYTES,
            "captured stderr exceeded its byte cap: {} bytes",
            message.len()
        );
    }

    #[tokio::test]
    async fn applies_model_and_reasoning_to_the_acp_session() {
        let (_dir, binary) = fake_grok(
            r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"authMethods":[{"id":"cached_token"}]}}' ;;
    *'"method":"authenticate"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{}}' ;;
    *'"method":"session/new"'*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"sessionId":"model-session"}}' ;;
    *'"method":"session/set_model"'*)
      case "$line" in *'"modelId":"grok-test-model"'*) ;; *) exit 3 ;; esac
      case "$line" in *'"reasoningEffort":"high"'*) ;; *) exit 4 ;; esac
      printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{}}' ;;
    *'"method":"session/prompt"'*) printf '%s\n' '{"jsonrpc":"2.0","id":5,"result":{"stopReason":"end_turn","_meta":{"modelId":"grok-test-model"}}}' ;;
  esac
done
"#,
        );
        let client = GrokClient::new(GrokClientConfig {
            binary,
            environment: HashMap::new(),
            max_turns: None,
            startup_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(5),
        });
        let output = client
            .run(
                GrokRunRequest {
                    cwd: std::env::current_dir().expect("cwd"),
                    prompt: "hello".to_owned(),
                    session_id: None,
                    model: Some("grok-test-model".to_owned()),
                    reasoning_effort: Some("high".to_owned()),
                    rules: None,
                    mcp_servers: Vec::new(),
                },
                GrokCancellation::default(),
                |_| {},
            )
            .await
            .expect("configured session succeeds");
        assert_eq!(output.actual_model.as_deref(), Some("grok-test-model"));
    }

    #[tokio::test]
    #[ignore = "requires an installed Grok Build CLI"]
    async fn real_grok_build_exposes_models_over_acp_stdio() {
        let client = GrokClient::new(GrokClientConfig::default());
        let catalog = client
            .discover_models(&std::env::current_dir().expect("cwd"))
            .await
            .expect("Grok ACP initialize succeeds");
        assert!(!catalog.models.is_empty(), "Grok advertised no models");
        assert!(catalog.current_model_id.is_some());
    }
}
