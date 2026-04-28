use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::{Map, Value};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, error};

use crate::control::{
    ControlRequestKind, ControlResponseMessage, ControlResponsePayload, IncomingControlRequest,
    IncomingRequestPayload, SDKControlRequest, SDKControlResponse,
};
use crate::error::{ClaudeSDKError, Result};
use crate::parse::parse_message;
use crate::transport::SubprocessTransport;
use crate::types::{ClaudeAgentOptions, McpServerConfig, Message};

/// Prompt types accepted by the client.
pub enum Prompt {
    /// A simple string prompt for one-shot mode.
    Text(String),
    /// A channel receiver for streaming messages in bidirectional mode.
    Stream(mpsc::Receiver<Value>),
}

impl From<String> for Prompt {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for Prompt {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

impl From<mpsc::Receiver<Value>> for Prompt {
    fn from(rx: mpsc::Receiver<Value>) -> Self {
        Self::Stream(rx)
    }
}

type PendingMap = HashMap<String, oneshot::Sender<std::result::Result<Value, String>>>;

/// Internal client for bidirectional conversations with Claude Code.
///
/// Public consumers should use [`run_streaming`](crate::run_streaming::run_streaming).
pub struct ClaudeSDKClient {
    options: ClaudeAgentOptions,
    transport: Option<Arc<SubprocessTransport>>,
    /// Channel for parsed messages flowing to the consumer.
    /// Wrapped in Option so we can drop the original sender after handing a
    /// clone to the reader task, ensuring the channel closes when the reader
    /// finishes.
    msg_tx: Option<mpsc::Sender<Result<Message>>>,
    msg_rx: Option<mpsc::Receiver<Result<Message>>>,
    /// Pending control-request responses, keyed by request_id.
    pending: Arc<Mutex<PendingMap>>,
    /// Counter for generating unique request IDs.
    request_counter: AtomicU64,
    /// Background reader task handle.
    reader_handle: Option<JoinHandle<()>>,
    /// Background stdin-stream task handle.
    stream_handle: Option<JoinHandle<()>>,
    /// Signal to stop background tasks.
    closed: Arc<AtomicBool>,
}

impl ClaudeSDKClient {
    /// Create a new client with the given options.
    pub fn new(options: ClaudeAgentOptions) -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self {
            options,
            transport: None,
            msg_tx: Some(tx),
            msg_rx: Some(rx),
            pending: Arc::new(Mutex::new(HashMap::new())),
            request_counter: AtomicU64::new(0),
            reader_handle: None,
            stream_handle: None,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Connect to Claude, optionally with an initial prompt.
    ///
    /// For bidirectional streaming, pass `Prompt::Stream(rx)`. For one-shot
    /// queries, pass `Prompt::Text("your question")` or `None`.
    pub async fn connect(&mut self, prompt: Option<Prompt>) -> Result<()> {
        self.closed.store(false, Ordering::SeqCst);
        let is_streaming = matches!(&prompt, Some(Prompt::Stream(_)) | None);

        // Build and spawn transport
        let transport = SubprocessTransport::new(self.options.clone(), is_streaming);
        match &prompt {
            Some(Prompt::Text(text)) => transport.spawn(Some(text)).await?,
            _ => transport.spawn(None).await?,
        }

        let transport = Arc::new(transport);
        self.transport = Some(transport.clone());

        // Start background reader
        self.start_reader(transport.clone());

        // If streaming, send initialize
        if is_streaming {
            if let Err(err) = self
                .send_control_request(
                    ControlRequestKind::Initialize { hooks: None },
                    std::time::Duration::from_secs(60),
                )
                .await
            {
                self.cleanup_after_failed_connect().await?;
                return Err(err);
            }
        }

        // If we have a stream prompt, start streaming it
        if let Some(Prompt::Stream(rx)) = prompt {
            self.start_stream_input(rx, transport);
        }

        Ok(())
    }

    /// Send a user content payload in an already-connected session.
    ///
    /// `content` may be either a plain text string or a block array.
    pub async fn send_user_content(
        &self,
        content: Value,
        session_id: Option<&str>,
        parent_tool_use_id: Option<&str>,
    ) -> Result<()> {
        let transport = self
            .transport
            .as_ref()
            .ok_or_else(|| ClaudeSDKError::Connection("Not connected".into()))?;

        let msg = build_user_message_payload(content, session_id, parent_tool_use_id);
        let line = serde_json::to_string(&msg)? + "\n";
        transport.write(&line).await
    }

    /// Receive all messages from the current conversation.
    ///
    /// Returns the receiver half of the message channel. Each call replaces
    /// the previous receiver (there can only be one consumer).
    pub fn take_message_receiver(&mut self) -> Option<mpsc::Receiver<Result<Message>>> {
        self.msg_rx.take()
    }

    /// Send an interrupt control request.
    pub async fn interrupt(&self) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::Interrupt,
            std::time::Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Change the permission mode during a conversation.
    pub async fn set_permission_mode(&self, mode: &str) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::SetPermissionMode {
                mode: mode.to_string(),
            },
            std::time::Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Change the AI model during a conversation.
    pub async fn set_model(&self, model: Option<&str>) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::SetModel {
                model: model.map(String::from),
            },
            std::time::Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Set the maximum number of thinking tokens for subsequent turns.
    pub async fn set_max_thinking_tokens(&self, max_thinking_tokens: Option<i64>) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::SetMaxThinkingTokens {
                max_thinking_tokens,
            },
            std::time::Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Rewind tracked files to their state at a specific user message.
    pub async fn rewind_files(&self, user_message_id: &str) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::RewindFiles {
                user_message_id: user_message_id.to_string(),
                dry_run: None,
            },
            std::time::Duration::from_secs(30),
        )
        .await?;
        Ok(())
    }

    /// Rewind tracked files in dry-run mode without modifying files.
    pub async fn rewind_files_dry_run(&self, user_message_id: &str) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::RewindFiles {
                user_message_id: user_message_id.to_string(),
                dry_run: Some(true),
            },
            std::time::Duration::from_secs(30),
        )
        .await?;
        Ok(())
    }

    /// Get current MCP server connection status.
    pub async fn get_mcp_status(&self) -> Result<Value> {
        self.send_control_request(
            ControlRequestKind::McpStatus,
            std::time::Duration::from_secs(10),
        )
        .await
    }

    /// Replace dynamically managed MCP servers.
    pub async fn set_mcp_servers(
        &self,
        servers: HashMap<String, McpServerConfig>,
    ) -> Result<Value> {
        self.send_control_request(
            ControlRequestKind::McpSetServers { servers },
            std::time::Duration::from_secs(30),
        )
        .await
    }

    /// Reconnect an MCP server by name.
    pub async fn reconnect_mcp_server(&self, server_name: &str) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::McpReconnect {
                server_name: server_name.to_owned(),
            },
            std::time::Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Enable or disable an MCP server by name.
    pub async fn toggle_mcp_server(&self, server_name: &str, enabled: bool) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::McpToggle {
                server_name: server_name.to_owned(),
                enabled,
            },
            std::time::Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Stop a running task by task ID.
    pub async fn stop_task(&self, task_id: &str) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::StopTask {
                task_id: task_id.to_owned(),
            },
            std::time::Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Apply settings into the CLI flag-settings layer.
    pub async fn apply_flag_settings(&self, settings: Value) -> Result<()> {
        self.send_control_request(
            ControlRequestKind::ApplyFlagSettings { settings },
            std::time::Duration::from_secs(10),
        )
        .await?;
        Ok(())
    }

    /// Disconnect from Claude and clean up.
    pub async fn disconnect(&mut self) -> Result<()> {
        self.closed.store(true, Ordering::SeqCst);

        self.abort_background_tasks().await;

        if let Some(transport) = self.transport.take() {
            transport.close().await?;
        }

        self.pending.lock().await.clear();
        self.reset_message_channel();
        self.closed.store(false, Ordering::SeqCst);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn next_request_id(&self) -> String {
        let n = self.request_counter.fetch_add(1, Ordering::SeqCst);
        format!("req_{n}_{:08x}", rand_u32())
    }

    async fn send_control_request(
        &self,
        request: ControlRequestKind,
        timeout: std::time::Duration,
    ) -> Result<Value> {
        let transport = self
            .transport
            .as_ref()
            .ok_or_else(|| ClaudeSDKError::Connection("Not connected".into()))?;

        let request_id = self.next_request_id();

        // Register pending response
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(request_id.clone(), tx);
        }

        // Send the request
        let req = SDKControlRequest::new(&request_id, request);
        let line = serde_json::to_string(&req)? + "\n";
        transport.write(&line).await?;

        // Wait for response with timeout
        let result = tokio::time::timeout(timeout, rx).await;

        // Clean up pending entry
        {
            let mut pending = self.pending.lock().await;
            pending.remove(&request_id);
        }

        match result {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error_msg))) => Err(ClaudeSDKError::Control(error_msg)),
            Ok(Err(_)) => Err(ClaudeSDKError::Control(
                "Control response channel closed".into(),
            )),
            Err(_) => Err(ClaudeSDKError::Timeout("Control request timed out".into())),
        }
    }

    fn reset_message_channel(&mut self) {
        let (tx, rx) = mpsc::channel(256);
        self.msg_tx = Some(tx);
        self.msg_rx = Some(rx);
    }

    async fn cleanup_after_failed_connect(&mut self) -> Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        self.abort_background_tasks().await;

        if let Some(transport) = self.transport.take() {
            transport.close().await?;
        }

        self.pending.lock().await.clear();
        self.reset_message_channel();
        self.closed.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn start_reader(&mut self, transport: Arc<SubprocessTransport>) {
        // Take the sender out of self so the channel closes when the reader
        // task finishes (dropping this sender). If we only cloned it, the
        // original would keep the channel alive and rx.recv() would block
        // forever even after the reader exits.
        let Some(msg_tx) = self.msg_tx.take() else {
            error!("start_reader called after reader already started");
            return;
        };
        let pending = self.pending.clone();
        let closed = self.closed.clone();

        let handle = tokio::spawn(async move {
            loop {
                if closed.load(Ordering::SeqCst) {
                    break;
                }

                let msg_result = transport.read_message().await;

                match msg_result {
                    Ok(Some(value)) => {
                        let msg_type = value.get("type").and_then(|v| v.as_str());

                        // Route control responses
                        if msg_type == Some("control_response") {
                            if let Ok(resp) =
                                serde_json::from_value::<SDKControlResponse>(value.clone())
                            {
                                let req_id = resp.response.request_id().to_string();
                                let mut pending_guard = pending.lock().await;
                                if let Some(sender) = pending_guard.remove(&req_id) {
                                    let result = match resp.response {
                                        ControlResponsePayload::Success { response, .. } => {
                                            Ok(response.unwrap_or(Value::Null))
                                        }
                                        ControlResponsePayload::Error { error, .. } => Err(error),
                                    };
                                    let _ = sender.send(result);
                                }
                            }
                            continue;
                        }

                        // Route incoming control requests from CLI
                        if msg_type == Some("control_request") {
                            if let Ok(req) =
                                serde_json::from_value::<IncomingControlRequest>(value.clone())
                            {
                                let resp = match req.request {
                                    IncomingRequestPayload::CanUseTool(_request) => {
                                        ControlResponseMessage::error(
                                            &req.request_id,
                                            "Unsupported control request: can_use_tool",
                                        )
                                    }
                                    IncomingRequestPayload::HookCallback(_request) => {
                                        ControlResponseMessage::error(
                                            &req.request_id,
                                            "Unsupported control request: hook_callback",
                                        )
                                    }
                                    IncomingRequestPayload::McpMessage(_request) => {
                                        ControlResponseMessage::error(
                                            &req.request_id,
                                            "Unsupported control request: mcp_message",
                                        )
                                    }
                                    IncomingRequestPayload::Elicitation(_request) => {
                                        ControlResponseMessage::error(
                                            &req.request_id,
                                            "Unsupported control request: elicitation",
                                        )
                                    }
                                };
                                if let Ok(line) = serde_json::to_string(&resp) {
                                    let _ = transport.write(&(line + "\n")).await;
                                }
                            }
                            continue;
                        }

                        // Skip control_cancel_request
                        if msg_type == Some("control_cancel_request") {
                            continue;
                        }

                        // Parse and forward to consumer
                        let parsed = parse_message(&value);
                        if msg_tx.send(parsed).await.is_err() {
                            debug!("Message receiver dropped, stopping reader");
                            break;
                        }
                    }
                    Ok(None) => {
                        debug!("Transport stream ended");
                        break;
                    }
                    Err(e) => {
                        error!("Transport read error: {e}");
                        let _ = msg_tx.send(Err(e)).await;
                        break;
                    }
                }
            }

            // Reader exiting — wake any pending control requests so they don't
            // hang forever (e.g. Initialize waiting when CLI exited early due to
            // an invalid --resume session ID).
            let mut pending_guard = pending.lock().await;
            for (_req_id, sender) in pending_guard.drain() {
                let _ = sender.send(Err("CLI process exited before responding".to_owned()));
            }
        });

        self.reader_handle = Some(handle);
    }

    fn start_stream_input(
        &mut self,
        mut rx: mpsc::Receiver<Value>,
        transport: Arc<SubprocessTransport>,
    ) {
        let closed = self.closed.clone();

        let handle = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if closed.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(line) = serde_json::to_string(&msg) {
                    if transport.write(&(line + "\n")).await.is_err() {
                        break;
                    }
                }
            }

            let _ = transport.end_input().await;
        });

        self.stream_handle = Some(handle);
    }

    async fn abort_background_tasks(&mut self) {
        if let Some(handle) = self.stream_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for ClaudeSDKClient {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);

        if let Some(handle) = self.stream_handle.take() {
            handle.abort();
        }

        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }

        let _ = self.transport.take();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rand_u32() -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    std::time::Instant::now().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    hasher.finish() as u32
}

fn build_user_message_payload(
    content: Value,
    session_id: Option<&str>,
    parent_tool_use_id: Option<&str>,
) -> Value {
    let mut root = Map::new();
    root.insert("type".to_owned(), Value::String("user".to_owned()));
    root.insert(
        "message".to_owned(),
        serde_json::json!({
            "role": "user",
            "content": content,
        }),
    );

    if let Some(tool_use_id) = parent_tool_use_id.filter(|id| !id.is_empty()) {
        root.insert(
            "parent_tool_use_id".to_owned(),
            Value::String(tool_use_id.to_owned()),
        );
    }
    if let Some(sid) = session_id.filter(|sid| !sid.is_empty()) {
        root.insert("session_id".to_owned(), Value::String(sid.to_owned()));
    }

    Value::Object(root)
}

#[cfg(test)]
mod tests;
