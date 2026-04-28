//! Error types for the Codex SDK.

use serde_json::Value;

/// Errors produced by the Codex SDK.
#[derive(thiserror::Error, Debug, Clone)]
pub enum CodexError {
    /// Failed to spawn or connect to the codex app-server process.
    #[error("connection failed: {0}")]
    ConnectionFailed(String),

    /// The codex app-server process exited unexpectedly.
    #[error("codex process died unexpectedly (exit code: {0})")]
    ProcessDied(i32),

    /// A JSON-RPC request timed out.
    #[error("request timed out after {0}s")]
    RequestTimeout(u64),

    /// The server returned a JSON-RPC error response.
    #[error("RPC error {code}: {message}")]
    RpcError {
        code: i64,
        message: String,
        data: Option<Value>,
    },

    /// A fatal error occurred that requires transport restart.
    #[error("fatal: {0}")]
    Fatal(String),

    /// The transport is already closed.
    #[error("transport already closed")]
    AlreadyClosed,

    /// The client is not initialized.
    #[error("client not initialized")]
    NotInitialized,
}

impl CodexError {
    /// Whether this error is an overload error (code -32001) that can be retried.
    pub fn is_overload(&self) -> bool {
        matches!(self, CodexError::RpcError { code: -32001, .. })
    }

    /// Whether this error is fatal and requires transport restart.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            CodexError::Fatal(_) | CodexError::ProcessDied(_) | CodexError::AlreadyClosed
        )
    }
}
