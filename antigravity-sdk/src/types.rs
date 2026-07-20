use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::error::Result;

/// Context supplied to the caller-owned launch approval policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub model: String,
    pub conversation_id: Option<String>,
    pub workspace_dir: PathBuf,
}

/// Launch-level permission modes exposed by the Antigravity CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Do not add a permission flag; let the CLI use its own default.
    UseCliDefault,
    /// Pass `--mode accept-edits`.
    AcceptEdits,
    /// Pass `--mode plan`.
    Plan,
    /// Pass `--dangerously-skip-permissions`.
    BypassPermissions,
    /// Refuse to spawn the run.
    Deny { reason: String },
}

pub type ApprovalFuture = Pin<Box<dyn Future<Output = Result<ApprovalDecision>> + Send + 'static>>;
pub type ApprovalCallback = Arc<dyn Fn(ApprovalRequest) -> ApprovalFuture + Send + Sync + 'static>;

/// Fully resolved input for one Antigravity CLI invocation.
pub struct AntigravityRunRequest {
    pub run_id: String,
    pub prompt: String,
    /// Shorter caller-owned text used only to disambiguate fresh `.db`
    /// candidates when the run log has not exposed a conversation UUID.
    pub discovery_text: String,
    pub model: String,
    pub conversation_id: Option<String>,
    pub workspace_dir: PathBuf,
    pub log_path: PathBuf,
    /// Complete per-run environment overlay. Keys are opaque to the SDK.
    pub env: HashMap<String, String>,
    pub print_timeout: Duration,
    /// Required: the SDK has no default approval decision.
    pub approval_callback: ApprovalCallback,
}

impl std::fmt::Debug for AntigravityRunRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AntigravityRunRequest")
            .field("run_id", &self.run_id)
            .field("prompt", &"<redacted>")
            .field("discovery_text", &"<redacted>")
            .field("model", &self.model)
            .field("conversation_id", &self.conversation_id)
            .field("workspace_dir", &self.workspace_dir)
            .field("log_path", &self.log_path)
            .field("env", &format_args!("<{} entries>", self.env.len()))
            .field("print_timeout", &self.print_timeout)
            .field("approval_callback", &"<callback>")
            .finish()
    }
}

/// Stable events reconstructed from Antigravity's private transcript rows.
#[derive(Debug, Clone, PartialEq)]
pub enum AntigravityEvent {
    SessionBound {
        conversation_id: String,
    },
    AssistantDelta {
        step_index: i64,
        text: String,
        reasoning: Option<String>,
        created_at: Option<String>,
    },
    ToolUse {
        step_index: i64,
        tool_use_id: String,
        name: String,
        input: Value,
        created_at: Option<String>,
    },
    ToolResult {
        step_index: i64,
        tool_use_id: Option<String>,
        name: String,
        content: Value,
        is_error: bool,
        created_at: Option<String>,
    },
    Error {
        step_index: i64,
        message: String,
        created_at: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntigravityRunFailureKind {
    InvalidConversation,
    Transcript,
    ProcessExit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AntigravityRunFailure {
    pub kind: AntigravityRunFailureKind,
    pub message: String,
}

impl AntigravityRunFailure {
    pub fn is_invalid_conversation(&self) -> bool {
        self.kind == AntigravityRunFailureKind::InvalidConversation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AntigravityRunOutcome {
    pub conversation_id: String,
    pub success: bool,
    pub failure: Option<AntigravityRunFailure>,
    /// Matches the legacy adapter: measured after session binding and through
    /// the final transcript drain, excluding discovery and I/O task cleanup.
    pub duration: Duration,
}
