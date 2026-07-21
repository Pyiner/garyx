//! Default approval policy for server-initiated approval requests.
//!
//! The SDK's built-in policy is **auto-approve**: when no custom
//! [`ServerRequestHandler`](crate::transport::ServerRequestHandler) is
//! installed — or the installed handler falls through with `Ok(None)` —
//! `item/commandExecution/requestApproval` and
//! `item/fileChange/requestApproval` are accepted without asking anyone.
//!
//! This is a deliberate product decision, not an oversight. Garyx runs
//! codex as an autonomous headless agent, and the same intent is expressed
//! at the source: `CodexClientConfig::approval_policy` defaults to
//! `"never"`, asking codex not to raise approval requests at all. This
//! module answers the ones it raises anyway. Callers that need
//! interactive or restrictive approval install a custom handler via
//! [`CodexTransport::set_server_request_handler`](crate::transport::CodexTransport::set_server_request_handler),
//! which always takes precedence over these defaults.

use serde_json::{Value, json};

/// Default result for `item/commandExecution/requestApproval`:
/// accept, and remember the acceptance for the rest of the session.
pub(crate) fn auto_approve_command_execution() -> Value {
    json!({
        "decision": "accept",
        "acceptSettings": { "forSession": true },
    })
}

/// Default result for `item/fileChange/requestApproval`: accept.
pub(crate) fn auto_approve_file_change() -> Value {
    json!({ "decision": "accept" })
}

#[cfg(test)]
mod tests;
