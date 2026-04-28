//! Subprocess-plugin ↔ in-process [`AuthFlowExecutor`] bridge.
//!
//! The Mac App / CLI talk to channels through the channel-blind
//! [`crate::auth_flow::AuthFlowExecutor`] trait. Built-in channels
//! such as feishu and weixin ship concrete impls; subprocess
//! plugins expose the same capability over JSON-RPC. This module's
//! [`SubprocessAuthFlowExecutor`] is the adapter that lets the
//! subprocess case fit through the trait — it forwards `start` /
//! `poll` calls as `auth_flow/start` / `auth_flow/poll` RPCs and
//! converts the responses back into the trait's `AuthSession` /
//! `AuthPollResult` types.
//!
//! The wire DTOs in [`crate::plugin_host::protocol`] are
//! intentionally structural clones of the trait types — identical
//! JSON shape, different Rust names (`AuthFlowStartRequest` vs
//! `AuthSession`, etc.) — so the conversion is a mechanical field
//! copy. The "two types with the same shape" duplication is
//! deliberate: one names the **in-process contract**, the other
//! names the **wire contract**, and keeping them distinct means a
//! future wire-compat change (e.g. an extra optional field the
//! trait doesn't surface) doesn't force a trait revision.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::auth_flow::{
    AuthDisplayItem, AuthFlowError, AuthFlowExecutor, AuthPollResult, AuthSession,
};
use crate::plugin_host::protocol::{
    AuthFlowCancelRequest, AuthFlowDisplayItem, AuthFlowPollRequest, AuthFlowPollResponse,
    AuthFlowStartRequest, AuthFlowStartResponse,
};
use crate::plugin_host::transport::{PluginRpcClient, RpcError};

/// Per-RPC ceiling. Auth-flow round trips don't carry payloads or
/// media — 30s is generous enough for a busy plugin but short enough
/// that a wedged plugin surfaces as a `Transport` error quickly so
/// the UI can cancel and let the user retry.
const AUTH_FLOW_RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Bridges a subprocess plugin's `auth_flow/*` JSON-RPC methods to
/// the in-process [`AuthFlowExecutor`] trait.
///
/// Clone-cheap — everything is behind an `Arc` inside
/// [`PluginRpcClient`]. Callers typically construct one per
/// subprocess plugin when the manager registers it and hand out
/// clones through [`crate::ChannelPluginManager::auth_flow_executor`].
#[derive(Clone)]
pub struct SubprocessAuthFlowExecutor {
    plugin_id: String,
    client: PluginRpcClient,
}

impl SubprocessAuthFlowExecutor {
    pub fn new(plugin_id: impl Into<String>, client: PluginRpcClient) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            client,
        }
    }

    /// Best-effort cancel — fires `auth_flow/cancel` and swallows
    /// any RPC failure. The trait does NOT expose this; callers
    /// that want to cancel (e.g. the gateway on timeout) can reach
    /// for this method directly. Cancellation is strictly optional
    /// on the plugin side: a plugin may ignore it and let the
    /// session expire naturally.
    pub async fn cancel(&self, session_id: &str) {
        let req = AuthFlowCancelRequest {
            session_id: session_id.to_owned(),
        };
        let _ = self
            .client
            .call_with_timeout::<_, Value>("auth_flow/cancel", &req, Some(AUTH_FLOW_RPC_TIMEOUT))
            .await;
    }

    /// Plugin id this bridge talks to. Surfaced mainly for error
    /// messages / logging; the trait itself is plugin-blind.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
}

#[async_trait]
impl AuthFlowExecutor for SubprocessAuthFlowExecutor {
    async fn start(&self, form_state: Value) -> Result<AuthSession, AuthFlowError> {
        let req = AuthFlowStartRequest { form_state };
        let resp: AuthFlowStartResponse = self
            .client
            .call_with_timeout("auth_flow/start", &req, Some(AUTH_FLOW_RPC_TIMEOUT))
            .await
            .map_err(|e| map_rpc_err(&self.plugin_id, "auth_flow/start", e))?;
        Ok(AuthSession {
            session_id: resp.session_id,
            display: resp
                .display
                .into_iter()
                .map(display_wire_to_trait)
                .collect(),
            expires_in_secs: resp.expires_in_secs,
            poll_interval_secs: resp.poll_interval_secs,
        })
    }

    async fn poll(&self, session_id: &str) -> Result<AuthPollResult, AuthFlowError> {
        let req = AuthFlowPollRequest {
            session_id: session_id.to_owned(),
        };
        let resp: AuthFlowPollResponse = self
            .client
            .call_with_timeout("auth_flow/poll", &req, Some(AUTH_FLOW_RPC_TIMEOUT))
            .await
            .map_err(|e| map_rpc_err(&self.plugin_id, "auth_flow/poll", e))?;
        Ok(match resp {
            AuthFlowPollResponse::Pending {
                display,
                next_interval_secs,
            } => AuthPollResult::Pending {
                display: display
                    .map(|items| items.into_iter().map(display_wire_to_trait).collect()),
                next_interval_secs,
            },
            AuthFlowPollResponse::Confirmed { values } => AuthPollResult::Confirmed { values },
            AuthFlowPollResponse::Failed { reason } => AuthPollResult::Failed { reason },
        })
    }
}

/// Wire-DTO → trait-type conversion. Separate function (not
/// `impl From`) so the module is the sole owner of the mapping and
/// the compiler catches if the wire format gains a variant that
/// the trait type doesn't represent.
fn display_wire_to_trait(item: AuthFlowDisplayItem) -> AuthDisplayItem {
    match item {
        AuthFlowDisplayItem::Text { value } => AuthDisplayItem::Text { value },
        AuthFlowDisplayItem::Qr { value } => AuthDisplayItem::Qr { value },
        AuthFlowDisplayItem::Unknown => AuthDisplayItem::Unknown,
    }
}

/// Collapse a [`RpcError`] into the trait's narrower error enum.
/// The trait distinguishes `Transport` (retryable / plugin-side
/// fault) from `Protocol` (response shape was wrong) — mirror
/// [`crate::plugin_host::PluginSenderHandle`]'s mapping so the
/// gateway can treat bridge errors uniformly with dispatch errors.
fn map_rpc_err(plugin_id: &str, method: &str, err: RpcError) -> AuthFlowError {
    match err {
        RpcError::Timeout(_) => {
            AuthFlowError::Transport(format!("plugin '{plugin_id}' {method} timed out"))
        }
        RpcError::Disconnected => {
            AuthFlowError::Transport(format!("plugin '{plugin_id}' unavailable"))
        }
        RpcError::HostAborted(msg) => AuthFlowError::Transport(msg),
        RpcError::Remote { code, message } => AuthFlowError::Protocol(format!(
            "plugin '{plugin_id}' {method} rpc error {code}: {message}"
        )),
        RpcError::MalformedResponse(msg) => AuthFlowError::Protocol(format!(
            "plugin '{plugin_id}' {method} malformed response: {msg}"
        )),
        RpcError::Codec(err) => AuthFlowError::Transport(format!(
            "plugin '{plugin_id}' {method} transport codec error: {err}"
        )),
        RpcError::Serialization(err) => AuthFlowError::Protocol(format!(
            "plugin '{plugin_id}' {method} serialization error: {err}"
        )),
    }
}

#[cfg(test)]
mod tests;
