use thiserror::Error;

/// All errors that can occur in the Claude Agent SDK.
#[derive(Error, Debug)]
pub enum ClaudeSDKError {
    #[error("CLI connection error: {0}")]
    Connection(String),

    #[error("CLI not found: {0}")]
    NotFound(String),

    #[error("Process error (exit code {exit_code:?}): {message}")]
    Process {
        message: String,
        exit_code: Option<i32>,
        stderr: Option<String>,
    },

    #[error("JSON decode error on line: {line}")]
    JsonDecode {
        line: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("Message parse error: {message}")]
    MessageParse {
        message: String,
        data: Option<serde_json::Value>,
    },

    #[error("Control protocol error: {0}")]
    Control(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Transport not ready")]
    NotReady,
}

pub type Result<T> = std::result::Result<T, ClaudeSDKError>;
