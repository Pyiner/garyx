//! Process and private-transcript protocol support for the Antigravity CLI.
//!
//! This crate deliberately contains no host-product semantics. Callers supply
//! resolved paths, environment, prompts, and approval policy, then map the
//! structured events into their own domain.

mod client;
mod discovery;
pub mod error;
mod transcript;
pub mod types;

pub use client::{AntigravityClient, AntigravityClientConfig};
pub use error::{AntigravityError, Result};
pub use types::{
    AntigravityEvent, AntigravityRunFailure, AntigravityRunFailureKind, AntigravityRunOutcome,
    AntigravityRunRequest, ApprovalCallback, ApprovalDecision, ApprovalFuture, ApprovalRequest,
};
