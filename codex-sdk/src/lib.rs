//! # codex-sdk
//!
//! Rust SDK for the Codex app-server JSON-RPC protocol.
//!
//! Provides a transport layer for spawning and communicating with
//! `codex app-server --listen stdio://` via line-delimited JSON-RPC,
//! and a high-level client for managing threads and turns.

pub mod client;
pub mod error;
pub mod transport;
pub mod types;

// Re-export primary types for convenience
pub use client::{CodexClient, CodexClientConfig};
pub use error::CodexError;
pub use transport::CodexTransport;
pub use types::{
    AgentMessageDelta, Capabilities, ClientInfo, CommandApprovalRequest, FileChangeApprovalRequest,
    InitializeParams, InputItem, ItemEventParams, JsonRpcNotification, ThreadResumeParams,
    ThreadStartParams, TurnCompletedParams, TurnInfo, TurnInterruptParams, TurnStartParams,
    TurnSteerParams, UsageInfo,
};
