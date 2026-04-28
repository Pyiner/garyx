mod client;
mod control;
pub mod error;
mod parse;
mod run_streaming;
mod transport;
pub mod types;

// Re-exports for convenience
pub use error::{ClaudeSDKError, Result};
pub use run_streaming::{
    ClaudeRun, ClaudeRunControl, OutboundUserMessage, UserInput, run_streaming,
};
pub use types::{
    AssistantMessage, AssistantMessageError, ClaudeAgentDefinition, ClaudeAgentOptions,
    ContentBlock, McpServerConfig, Message, PermissionMode, ResultMessage, StreamEvent,
    SystemMessage, TextBlock, ThinkingBlock, ToolResultBlock, ToolUseBlock, UserContent,
    UserMessage,
};
