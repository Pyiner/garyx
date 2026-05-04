//! Codex app-server transport (JSON-RPC over stdio).
//!
//! Spawns a `codex app-server --listen stdio://` child process and communicates
//! via line-delimited JSON-RPC messages on stdin/stdout.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::error::CodexError;
use crate::types::{JsonRpcError, JsonRpcNotification, JsonRpcServerResponse};

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

type PendingMap = HashMap<u64, oneshot::Sender<Value>>;

/// Handler for server-initiated requests.
///
/// Receives `(method, params)` and returns:
/// - `Ok(Some(result))` to respond with a result
/// - `Ok(None)` to fall through to default handling
/// - `Err((code, message, data))` to respond with an error
pub type ServerRequestHandler = Arc<
    dyn Fn(
            &str,
            &Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<Option<Value>, (i64, String, Option<Value>)>,
                    > + Send,
            >,
        > + Send
        + Sync,
>;

// ---------------------------------------------------------------------------
// CodexTransport
// ---------------------------------------------------------------------------

/// Stdio transport for `codex app-server --listen stdio://`.
///
/// Manages the subprocess lifecycle, JSON-RPC request/response matching,
/// notification broadcasting, and server-initiated request handling.
pub struct CodexTransport {
    codex_bin: String,
    extra_args: Vec<String>,
    env: HashMap<String, String>,
    startup_timeout: Duration,
    request_timeout: Duration,

    child: Mutex<Option<Child>>,
    stdin_writer: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    pending: Arc<Mutex<PendingMap>>,
    next_id: AtomicU64,
    notification_tx: broadcast::Sender<JsonRpcNotification>,

    server_request_handler: Arc<Mutex<Option<ServerRequestHandler>>>,

    reader_task: Mutex<Option<JoinHandle<()>>>,
    stderr_task: Mutex<Option<JoinHandle<()>>>,
    wait_task: Mutex<Option<JoinHandle<()>>>,

    ready: AtomicBool,
    closed: AtomicBool,
    fatal_error: Arc<Mutex<Option<CodexError>>>,
}

impl CodexTransport {
    /// Create a new transport (does NOT spawn the process yet).
    pub fn new(codex_bin: &str, extra_args: &[&str]) -> Self {
        let (notification_tx, _) = broadcast::channel(4096);

        Self {
            codex_bin: codex_bin.to_owned(),
            extra_args: extra_args.iter().map(|s| (*s).to_owned()).collect(),
            env: HashMap::new(),
            startup_timeout: Duration::from_secs(300),
            request_timeout: Duration::from_secs(300),
            child: Mutex::new(None),
            stdin_writer: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            notification_tx,
            server_request_handler: Arc::new(Mutex::new(None)),
            reader_task: Mutex::new(None),
            stderr_task: Mutex::new(None),
            wait_task: Mutex::new(None),
            ready: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            fatal_error: Arc::new(Mutex::new(None)),
        }
    }

    /// Override the startup timeout (default 300s).
    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    /// Override the per-request timeout (default 300s).
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Override environment variables for the spawned process.
    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Whether the transport is initialized and healthy.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst) && !self.closed.load(Ordering::SeqCst)
    }

    /// Whether the transport has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Return the fatal error if one has been set.
    pub async fn fatal_error(&self) -> Option<CodexError> {
        self.fatal_error.lock().await.clone()
    }

    /// Subscribe to server notifications.
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.notification_tx.subscribe()
    }

    /// Set a custom handler for server-initiated requests.
    pub async fn set_server_request_handler(&self, handler: Option<ServerRequestHandler>) {
        *self.server_request_handler.lock().await = handler;
    }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Spawn the child process, complete the JSON-RPC `initialize` handshake,
    /// and send the `initialized` notification.
    ///
    /// `init_params` is the value sent as `params` in the `initialize` request.
    pub async fn start(&self, init_params: Value) -> Result<Value, CodexError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(CodexError::AlreadyClosed);
        }
        if self.ready.load(Ordering::SeqCst) {
            return Err(CodexError::Fatal("already initialized".to_owned()));
        }

        let mut cmd = Command::new(&self.codex_bin);
        cmd.args(["app-server", "--listen", "stdio://"]);
        cmd.args(&self.extra_args);
        if !self.env.is_empty() {
            cmd.envs(&self.env);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        tracing::info!(codex_bin = %self.codex_bin, "spawning codex app-server");

        let mut child = cmd
            .spawn()
            .map_err(|e| CodexError::ConnectionFailed(format!("failed to spawn codex: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CodexError::ConnectionFailed("failed to open stdin pipe".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CodexError::ConnectionFailed("failed to open stdout pipe".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CodexError::ConnectionFailed("failed to open stderr pipe".to_owned()))?;

        let pid = child.id().unwrap_or(0);
        tracing::info!(pid, "codex app-server spawned");

        *self.stdin_writer.lock().await = Some(stdin);
        *self.child.lock().await = Some(child);

        // Start background read loop
        {
            let pending = Arc::clone(&self.pending);
            let stdin_writer = Arc::clone(&self.stdin_writer);
            let notification_tx = self.notification_tx.clone();
            let handler = Arc::clone(&self.server_request_handler);
            let fatal_error = Arc::clone(&self.fatal_error);

            let reader = tokio::spawn(reader_loop(
                pid,
                stdout,
                pending,
                notification_tx,
                stdin_writer,
                handler,
                fatal_error,
            ));
            *self.reader_task.lock().await = Some(reader);
        }

        // Start stderr logging loop
        {
            let handle = tokio::spawn(stderr_loop(pid, stderr));
            *self.stderr_task.lock().await = Some(handle);
        }

        let result = match self
            .request_with_timeout("initialize", Some(init_params), self.startup_timeout)
            .await
        {
            Ok(result) => result,
            Err(err) => {
                self.cleanup_after_failed_start().await;
                return Err(err);
            }
        };

        if let Err(err) = self.send_notification("initialized", None).await {
            self.cleanup_after_failed_start().await;
            return Err(err);
        }

        self.ready.store(true, Ordering::SeqCst);
        tracing::info!("codex app-server transport initialized");

        Ok(result)
    }

    /// Gracefully shut down the transport and child process.
    pub async fn shutdown(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.ready.store(false, Ordering::SeqCst);

        // Fail all pending requests
        {
            let mut pending = self.pending.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(serde_json::json!({
                    "error": {
                        "code": -32000,
                        "message": "transport shutting down",
                    }
                }));
            }
        }

        // Cancel background tasks
        for task_slot in [&self.reader_task, &self.stderr_task, &self.wait_task] {
            if let Some(task) = task_slot.lock().await.take() {
                task.abort();
                let _ = task.await;
            }
        }

        // Close stdin and terminate
        {
            let _ = self.stdin_writer.lock().await.take();
        }

        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.start_kill();
            match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                Ok(_) => {}
                Err(_) => {
                    let _ = child.kill().await;
                }
            }
        }

        tracing::info!("codex transport shut down");
    }

    // -----------------------------------------------------------------------
    // JSON-RPC messaging
    // -----------------------------------------------------------------------

    /// Send a JSON-RPC request and await the response.
    pub async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, CodexError> {
        self.request_with_timeout(method, params, self.request_timeout)
            .await
    }

    /// Send a JSON-RPC request with overload retry.
    pub async fn send_request_with_retry(
        &self,
        method: &str,
        params: Option<Value>,
        max_retries: u32,
    ) -> Result<Value, CodexError> {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match self.send_request(method, params.clone()).await {
                Ok(val) => return Ok(val),
                Err(ref e) if e.is_overload() && attempt <= max_retries => {
                    let delay_ms = (200.0 * f64::from(1u32 << (attempt - 1))).min(2000.0);
                    let jitter = rand_jitter();
                    let delay =
                        Duration::from_millis(delay_ms as u64) + Duration::from_millis(jitter);
                    tracing::warn!(
                        method,
                        attempt,
                        max_retries,
                        delay_ms = delay.as_millis() as u64,
                        "codex overloaded, retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Send a JSON-RPC notification (no response expected).
    pub async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), CodexError> {
        if method != "initialized" {
            self.check_health()?;
            if !self.ready.load(Ordering::SeqCst) {
                return Err(CodexError::NotInitialized);
            }
        }

        let mut payload = serde_json::json!({ "method": method });
        if let Some(p) = params {
            payload["params"] = p;
        }
        self.write_message(&payload).await
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn check_health(&self) -> Result<(), CodexError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(CodexError::AlreadyClosed);
        }
        Ok(())
    }

    async fn cleanup_after_failed_start(&self) {
        self.ready.store(false, Ordering::SeqCst);

        for task_slot in [&self.reader_task, &self.stderr_task, &self.wait_task] {
            if let Some(task) = task_slot.lock().await.take() {
                task.abort();
                let _ = task.await;
            }
        }

        {
            let _ = self.stdin_writer.lock().await.take();
        }

        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.start_kill();
            match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                Ok(_) => {}
                Err(_) => {
                    let _ = child.kill().await;
                }
            }
        }

        self.pending.lock().await.clear();
        *self.fatal_error.lock().await = None;
    }

    async fn request_with_timeout(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, CodexError> {
        self.check_health()?;

        if method != "initialize" && !self.ready.load(Ordering::SeqCst) {
            return Err(CodexError::NotInitialized);
        }

        // Check for fatal error
        if let Some(ref err) = *self.fatal_error.lock().await {
            return Err(err.clone());
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        self.pending.lock().await.insert(id, tx);

        let mut payload = serde_json::json!({
            "id": id,
            "method": method,
        });
        if let Some(p) = params {
            payload["params"] = p;
        }

        tracing::info!(id, method, "codex-rpc request");

        if let Err(e) = self.write_message(&payload).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        let response = tokio::time::timeout(timeout, rx).await.map_err(|_| {
            let pending = self.pending.clone();
            tokio::spawn(async move {
                pending.lock().await.remove(&id);
            });
            CodexError::RequestTimeout(timeout.as_secs())
        })?;

        let response =
            response.map_err(|_| CodexError::Fatal("response channel dropped".to_owned()))?;

        // Check for error in response
        if let Some(error) = response.get("error") {
            let code = error.get("code").and_then(|v| v.as_i64()).unwrap_or(-32000);
            let message = error
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown RPC error")
                .to_owned();
            let data = error.get("data").cloned();
            return Err(CodexError::RpcError {
                code,
                message,
                data,
            });
        }

        Ok(response
            .get("result")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new())))
    }

    async fn write_message(&self, payload: &Value) -> Result<(), CodexError> {
        let mut guard = self.stdin_writer.lock().await;
        let stdin = guard.as_mut().ok_or(CodexError::AlreadyClosed)?;

        let mut encoded = serde_json::to_string(payload)
            .map_err(|e| CodexError::Fatal(format!("JSON encode error: {e}")))?;
        encoded.push('\n');

        stdin
            .write_all(encoded.as_bytes())
            .await
            .map_err(|e| CodexError::Fatal(format!("stdin write error: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| CodexError::Fatal(format!("stdin flush error: {e}")))?;

        Ok(())
    }
}

impl Drop for CodexTransport {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.child.try_lock()
            && let Some(ref mut child) = *guard
        {
            let _ = child.start_kill();
        }
    }
}

// ---------------------------------------------------------------------------
// Background tasks
// ---------------------------------------------------------------------------

/// Read lines from stdout and dispatch JSON-RPC messages.
async fn reader_loop(
    pid: u32,
    stdout: tokio::process::ChildStdout,
    pending: Arc<Mutex<PendingMap>>,
    notification_tx: broadcast::Sender<JsonRpcNotification>,
    stdin_writer: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    server_request_handler: Arc<Mutex<Option<ServerRequestHandler>>>,
    fatal_error: Arc<Mutex<Option<CodexError>>>,
) {
    let mut reader = BufReader::new(stdout).lines();

    loop {
        let line = match reader.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                tracing::warn!(pid, "codex stdout EOF");
                let err = CodexError::Fatal("codex stdout closed unexpectedly".to_owned());
                set_fatal_error(&fatal_error, &pending, &notification_tx, err).await;
                break;
            }
            Err(e) => {
                tracing::error!(pid, error = %e, "codex stdout read error");
                let err = CodexError::Fatal(format!("stdout read error: {e}"));
                set_fatal_error(&fatal_error, &pending, &notification_tx, err).await;
                break;
            }
        };

        let line = line.trim().to_owned();
        if line.is_empty() {
            continue;
        }

        let payload: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(pid, error = %e, "failed to parse codex stdout line");
                continue;
            }
        };

        dispatch_message(
            &payload,
            &pending,
            &notification_tx,
            &stdin_writer,
            &server_request_handler,
            &fatal_error,
        )
        .await;
    }
}

/// Classify and dispatch a parsed JSON-RPC message.
async fn dispatch_message(
    payload: &Value,
    pending: &Arc<Mutex<PendingMap>>,
    notification_tx: &broadcast::Sender<JsonRpcNotification>,
    stdin_writer: &Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    server_request_handler: &Arc<Mutex<Option<ServerRequestHandler>>>,
    fatal_error: &Arc<Mutex<Option<CodexError>>>,
) {
    let has_id = payload.get("id").is_some();
    let has_method = payload.get("method").is_some();
    let has_result = payload.get("result").is_some() || payload.get("error").is_some();

    // Case 1: Server-initiated request (has method AND id, but no result/error)
    if has_method && has_id && !has_result {
        handle_server_request(
            payload,
            stdin_writer,
            server_request_handler,
            fatal_error,
            pending,
            notification_tx,
        )
        .await;
        return;
    }

    // Case 2: Response to our request (has id AND result/error)
    if has_id && has_result {
        if let Some(id) = payload.get("id").and_then(|v| v.as_u64()) {
            let tx = pending.lock().await.remove(&id);
            if let Some(tx) = tx {
                let _ = tx.send(payload.clone());
            }
        }
        return;
    }

    // Case 3: Notification (has method, no id)
    if has_method
        && !has_id
        && let Some(method) = payload.get("method").and_then(|v| v.as_str())
    {
        let params = payload
            .get("params")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new()));
        let _ = notification_tx.send(JsonRpcNotification {
            method: method.to_owned(),
            params,
        });
    }
}

/// Handle a server-initiated request.
async fn handle_server_request(
    payload: &Value,
    stdin_writer: &Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    server_request_handler: &Arc<Mutex<Option<ServerRequestHandler>>>,
    fatal_error: &Arc<Mutex<Option<CodexError>>>,
    pending: &Arc<Mutex<PendingMap>>,
    notification_tx: &broadcast::Sender<JsonRpcNotification>,
) {
    let method = match payload.get("method").and_then(|v| v.as_str()) {
        Some(m) => m.to_owned(),
        None => return,
    };
    let request_id = match payload.get("id") {
        Some(id) => id.clone(),
        None => return,
    };
    let params = payload
        .get("params")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    tracing::info!(method = %method, "handling codex server request");

    // Try custom handler first
    let handler_guard = server_request_handler.lock().await;
    if let Some(ref handler) = *handler_guard {
        match handler(&method, &params).await {
            Ok(Some(result)) => {
                let resp = JsonRpcServerResponse {
                    id: request_id,
                    result: Some(result),
                    error: None,
                };
                let _ = write_response(stdin_writer, &resp).await;
                return;
            }
            Ok(None) => {
                // Fall through to default handling
            }
            Err((code, message, data)) => {
                let resp = JsonRpcServerResponse {
                    id: request_id,
                    result: None,
                    error: Some(JsonRpcError {
                        code,
                        message,
                        data,
                    }),
                };
                let _ = write_response(stdin_writer, &resp).await;
                return;
            }
        }
    }
    drop(handler_guard);

    // Default handling
    let resp = match method.as_str() {
        "item/commandExecution/requestApproval" => JsonRpcServerResponse {
            id: request_id,
            result: Some(serde_json::json!({
                "decision": "accept",
                "acceptSettings": { "forSession": true },
            })),
            error: None,
        },
        "item/fileChange/requestApproval" => JsonRpcServerResponse {
            id: request_id,
            result: Some(serde_json::json!({ "decision": "accept" })),
            error: None,
        },
        "item/tool/requestUserInput" => JsonRpcServerResponse {
            id: request_id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "item/tool/requestUserInput is not supported".to_owned(),
                data: None,
            }),
        },
        "item/tool/call" => JsonRpcServerResponse {
            id: request_id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "item/tool/call is not supported".to_owned(),
                data: None,
            }),
        },
        "account/chatgptAuthTokens/refresh" => {
            let err = CodexError::Fatal(
                "codex requested chatgptAuthTokens/refresh - run `codex login`".to_owned(),
            );
            set_fatal_error(fatal_error, pending, notification_tx, err).await;
            JsonRpcServerResponse {
                id: request_id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32000,
                    message: "token broker not available, run `codex login`".to_owned(),
                    data: None,
                }),
            }
        }
        _ => {
            tracing::debug!(method = %method, "unhandled codex server request, returning empty result");
            JsonRpcServerResponse {
                id: request_id,
                result: Some(serde_json::json!({})),
                error: None,
            }
        }
    };

    if let Err(e) = write_response(stdin_writer, &resp).await {
        tracing::error!(error = %e, "failed to write server-request response");
    }
}

/// Write a JSON-RPC server response to stdin.
async fn write_response(
    stdin_writer: &Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    resp: &JsonRpcServerResponse,
) -> Result<(), CodexError> {
    let mut guard = stdin_writer.lock().await;
    let stdin = guard.as_mut().ok_or(CodexError::AlreadyClosed)?;

    let payload = serde_json::to_value(resp)
        .map_err(|e| CodexError::Fatal(format!("JSON encode error: {e}")))?;

    let mut encoded =
        serde_json::to_string(&payload).map_err(|e| CodexError::Fatal(e.to_string()))?;
    encoded.push('\n');

    stdin
        .write_all(encoded.as_bytes())
        .await
        .map_err(|e| CodexError::Fatal(format!("stdin write: {e}")))?;
    stdin
        .flush()
        .await
        .map_err(|e| CodexError::Fatal(format!("stdin flush: {e}")))?;

    Ok(())
}

/// Set a fatal error, fail all pending requests, and broadcast a transport/fatal
/// notification.
async fn set_fatal_error(
    fatal_error: &Arc<Mutex<Option<CodexError>>>,
    pending: &Arc<Mutex<PendingMap>>,
    notification_tx: &broadcast::Sender<JsonRpcNotification>,
    error: CodexError,
) {
    let mut guard = fatal_error.lock().await;
    if guard.is_some() {
        return;
    }
    *guard = Some(error.clone());
    drop(guard);

    // Fail all pending requests
    let mut pending_guard = pending.lock().await;
    for (_, tx) in pending_guard.drain() {
        let _ = tx.send(serde_json::json!({
            "error": {
                "code": -32000,
                "message": error.to_string(),
            }
        }));
    }

    // Broadcast fatal notification
    let _ = notification_tx.send(JsonRpcNotification {
        method: "transport/fatal".to_owned(),
        params: serde_json::json!({ "error": error.to_string() }),
    });
}

/// Read and log stderr from the child process.
async fn stderr_loop(pid: u32, stderr: tokio::process::ChildStderr) {
    let mut reader = BufReader::new(stderr).lines();

    loop {
        match reader.next_line().await {
            Ok(Some(line)) => {
                if !line.is_empty() {
                    tracing::info!(pid, "[codex-app-server] {}", line);
                }
            }
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(pid, error = %e, "codex stderr read error");
                break;
            }
        }
    }
}

/// Simple pseudo-random jitter in [0, 100) milliseconds.
fn rand_jitter() -> u64 {
    // Use the current time nanoseconds for cheap jitter without pulling in `rand`.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 100) as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
