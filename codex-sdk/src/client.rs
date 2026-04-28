//! High-level Codex client that wraps `CodexTransport` with a convenient API.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use crate::error::CodexError;
use crate::transport::CodexTransport;
use crate::types::{
    Capabilities, ClientInfo, InitializeParams, InputItem, JsonRpcNotification, ThreadResumeParams,
    ThreadStartParams, TurnInterruptParams, TurnStartParams, TurnSteerParams, extract_thread_id,
    extract_turn_id,
};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Configuration for `CodexClient`.
#[derive(Debug, Clone)]
pub struct CodexClientConfig {
    /// Path to the `codex` binary (default: "codex").
    pub codex_bin: String,
    /// Working directory passed to `thread/start`.
    pub workspace_dir: Option<String>,
    /// Model name to use.
    pub model: Option<String>,
    /// Approval policy (e.g., "never", "unless-allow-listed").
    pub approval_policy: String,
    /// Sandbox mode (e.g., "off", "danger-full-access").
    pub sandbox_mode: String,
    /// Whether to enable experimental API features.
    pub experimental_api: bool,
    /// Per-request timeout.
    pub request_timeout: Duration,
    /// Timeout for the initial startup handshake.
    pub startup_timeout: Duration,
    /// Maximum retries on overload (-32001) errors.
    pub max_overload_retries: u32,
    /// Extra environment variables for the app-server subprocess.
    pub env: HashMap<String, String>,
}

impl Default for CodexClientConfig {
    fn default() -> Self {
        Self {
            codex_bin: "codex".to_owned(),
            workspace_dir: None,
            model: None,
            approval_policy: "never".to_owned(),
            sandbox_mode: "off".to_owned(),
            experimental_api: false,
            request_timeout: Duration::from_secs(300),
            startup_timeout: Duration::from_secs(300),
            max_overload_retries: 4,
            env: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// CodexClient
// ---------------------------------------------------------------------------

/// High-level client for the Codex app-server JSON-RPC protocol.
///
/// Wraps `CodexTransport` and provides typed methods for thread/turn lifecycle.
pub struct CodexClient {
    transport: CodexTransport,
    config: CodexClientConfig,
    initialized: bool,
}

impl CodexClient {
    /// Create a new client with the given config. Does NOT start the process.
    pub fn new(config: CodexClientConfig) -> Self {
        let transport = CodexTransport::new(&config.codex_bin, &[])
            .with_env(config.env.clone())
            .with_startup_timeout(config.startup_timeout)
            .with_request_timeout(config.request_timeout);

        Self {
            transport,
            config,
            initialized: false,
        }
    }

    /// Spawn the codex app-server process, perform the initialize handshake,
    /// and send the `initialized` notification.
    pub async fn initialize(&mut self) -> Result<Value, CodexError> {
        if self.initialized {
            return Err(CodexError::Fatal("already initialized".to_owned()));
        }

        let params = InitializeParams {
            client_info: ClientInfo {
                name: "codex-sdk-rs".to_owned(),
                title: "Codex SDK (Rust)".to_owned(),
                version: "0.1.0".to_owned(),
            },
            capabilities: Capabilities {
                experimental_api: self.config.experimental_api,
            },
        };

        let init_value = serde_json::to_value(&params)
            .map_err(|e| CodexError::Fatal(format!("failed to serialize init params: {e}")))?;

        let result = self.transport.start(init_value).await?;
        self.initialized = true;
        Ok(result)
    }

    /// Start a new thread and return the thread ID.
    pub async fn start_thread(&self, params: ThreadStartParams) -> Result<String, CodexError> {
        self.require_initialized()?;

        let value = serde_json::to_value(&params)
            .map_err(|e| CodexError::Fatal(format!("serialize error: {e}")))?;

        let result = self
            .transport
            .send_request_with_retry(
                "thread/start",
                Some(value),
                self.config.max_overload_retries,
            )
            .await?;

        extract_thread_id(&result)
            .ok_or_else(|| CodexError::Fatal("thread/start did not return thread id".to_owned()))
    }

    /// Resume an existing thread and return the thread ID.
    pub async fn resume_thread(&self, params: ThreadResumeParams) -> Result<String, CodexError> {
        self.require_initialized()?;

        let value = serde_json::to_value(&params)
            .map_err(|e| CodexError::Fatal(format!("serialize error: {e}")))?;

        let result = self
            .transport
            .send_request_with_retry(
                "thread/resume",
                Some(value),
                self.config.max_overload_retries,
            )
            .await?;

        Ok(extract_thread_id(&result).unwrap_or(params.thread_id.clone()))
    }

    /// Start a turn on a thread with the given input items. Returns the turn ID.
    pub async fn start_turn(
        &self,
        thread_id: &str,
        input: Vec<InputItem>,
    ) -> Result<String, CodexError> {
        self.require_initialized()?;

        let params = TurnStartParams {
            thread_id: thread_id.to_owned(),
            input,
        };
        let value = serde_json::to_value(&params)
            .map_err(|e| CodexError::Fatal(format!("serialize error: {e}")))?;

        let result = self
            .transport
            .send_request_with_retry("turn/start", Some(value), self.config.max_overload_retries)
            .await?;

        extract_turn_id(&result)
            .ok_or_else(|| CodexError::Fatal("turn/start did not return turn id".to_owned()))
    }

    /// Steer an active turn with additional input.
    pub async fn steer_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
        input: Vec<InputItem>,
    ) -> Result<(), CodexError> {
        self.require_initialized()?;

        let params = TurnSteerParams {
            thread_id: thread_id.to_owned(),
            turn_id: Some(turn_id.to_owned()),
            expected_turn_id: turn_id.to_owned(),
            input,
        };
        let value = serde_json::to_value(&params)
            .map_err(|e| CodexError::Fatal(format!("serialize error: {e}")))?;

        self.transport
            .send_request("turn/steer", Some(value))
            .await?;
        Ok(())
    }

    /// Interrupt (cancel) an active turn.
    pub async fn interrupt_turn(&self, thread_id: &str, turn_id: &str) -> Result<(), CodexError> {
        self.require_initialized()?;

        let params = TurnInterruptParams {
            thread_id: thread_id.to_owned(),
            turn_id: turn_id.to_owned(),
        };
        let value = serde_json::to_value(&params)
            .map_err(|e| CodexError::Fatal(format!("serialize error: {e}")))?;

        self.transport
            .send_request("turn/interrupt", Some(value))
            .await?;
        Ok(())
    }

    /// Subscribe to server notifications (streaming deltas, item events, etc.).
    pub fn subscribe_events(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.transport.subscribe_notifications()
    }

    /// Whether the client is initialized and the transport is healthy.
    pub fn is_ready(&self) -> bool {
        self.initialized && self.transport.is_ready()
    }

    /// Return the fatal error if one has occurred.
    pub async fn fatal_error(&self) -> Option<CodexError> {
        self.transport.fatal_error().await
    }

    /// Gracefully shut down the client and the underlying transport.
    pub async fn shutdown(&mut self) {
        self.initialized = false;
        self.transport.shutdown().await;
    }

    /// Get a reference to the underlying transport.
    pub fn transport(&self) -> &CodexTransport {
        &self.transport
    }

    fn require_initialized(&self) -> Result<(), CodexError> {
        if !self.initialized {
            return Err(CodexError::NotInitialized);
        }
        Ok(())
    }
}

// Need to import broadcast for subscribe_events return type
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
