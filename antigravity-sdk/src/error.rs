use thiserror::Error;

/// Errors raised before an Antigravity run can return a structured outcome.
#[derive(Debug, Error)]
pub enum AntigravityError {
    #[error("failed to spawn antigravity CLI: {0}")]
    Spawn(String),

    #[error("antigravity CLI is not ready: {0}")]
    NotReady(String),

    #[error("antigravity approval was denied: {0}")]
    ApprovalDenied(String),

    #[error("antigravity conversation id discovery timed out{0}")]
    DiscoveryTimeout(String),

    #[error("antigravity process wait failed: {0}")]
    ProcessWait(String),

    #[error("{0}")]
    ProcessExited(String),

    #[error("invalid antigravity conversation: {0}")]
    InvalidConversation(String),

    #[error("antigravity run timed out")]
    Timeout,

    #[error("antigravity transport error: {0}")]
    Transport(String),
}

impl AntigravityError {
    /// Whether retrying without the requested conversation can recover.
    pub fn is_invalid_conversation(&self) -> bool {
        matches!(self, Self::InvalidConversation(_))
    }
}

pub type Result<T> = std::result::Result<T, AntigravityError>;
