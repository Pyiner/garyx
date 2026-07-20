mod client;
mod control;
pub mod error;
mod parse;
mod run_streaming;
mod transport;
pub mod types;

// Re-exports for convenience
pub use client::STOP_HOOK_OBSERVATION_SUBTYPE;
pub use control::CanUseToolRequest;
pub use error::{ClaudeSDKError, Result};
pub use run_streaming::{
    ClaudeRun, ClaudeRunControl, OutboundUserMessage, UserInput, run_streaming,
};
pub use types::{
    AssistantMessage, AssistantMessageError, CanUseToolCallback, CanUseToolFuture,
    ClaudeAgentDefinition, ClaudeAgentOptions, ContentBlock, DocumentBlock, DocumentSource,
    McpServerConfig, Message, MessageOrigin, PermissionMode, ResultMessage, StreamEvent,
    SystemMessage, TextBlock, ThinkingBlock, ToolResultBlock, ToolUseBlock, UnknownContentBlock,
    UserContent, UserMessage,
};
