//! Outbound sender over the plugin-host JSON-RPC transport.
//!
//! Wraps a [`PluginRpcClient`] with the §9.4 contract:
//! - Capability short-circuit before we spend a round-trip on a plugin
//!   that has told us it does not support outbound.
//! - The 30-second per-RPC timeout from §11.1.
//! - The error-code → `ChannelError` map that preserves the caller's
//!   retry semantics (Config is non-retryable, Connection is).
//!
//! Deliberately thin: this module owns the wire contract but does NOT
//! own plugin lifecycle, respawn, or registration. The dispatcher
//! (`crate::dispatcher`) and the manager compose these handles.

use std::time::Duration;

use tracing::warn;

use super::protocol::{
    CapabilitiesResponse, DispatchOutbound, DispatchOutboundResult, PluginErrorCode,
};
use super::transport::{PluginRpcClient, RpcError};
use crate::channel_trait::ChannelError;

/// §11.1: host-enforced timeout for `dispatch_outbound`. Each plugin
/// call carries this regardless of the transport default so a caller
/// never waits longer than the spec permits, even if a future
/// `TransportConfig` default drifts.
pub const DISPATCH_TIMEOUT: Duration = Duration::from_secs(30);

/// A plugin-backed outbound sender. Clone-cheap: the inner
/// [`PluginRpcClient`] is already `Arc`-backed, so producing additional
/// handles for the dispatcher is free.
///
/// Dropping the handle does NOT stop the subprocess; the manager owns
/// lifecycle separately.
#[derive(Clone)]
pub struct PluginSenderHandle {
    plugin_id: String,
    rpc: PluginRpcClient,
    capabilities: CapabilitiesResponse,
    /// Per-RPC timeout the handle enforces on every `dispatch_outbound`.
    /// Production constructs via [`Self::new`] and gets [`DISPATCH_TIMEOUT`];
    /// tests that need to exercise the real timeout path use
    /// [`Self::with_timeout`].
    timeout: Duration,
}

impl PluginSenderHandle {
    pub fn new(
        plugin_id: String,
        rpc: PluginRpcClient,
        capabilities: CapabilitiesResponse,
    ) -> Self {
        Self::with_timeout(plugin_id, rpc, capabilities, DISPATCH_TIMEOUT)
    }

    /// Construct a handle with a caller-supplied `dispatch_outbound`
    /// timeout. The only production caller is [`Self::new`], which
    /// hard-codes [`DISPATCH_TIMEOUT`]; this entry point is
    /// `pub(crate)` so same-crate tests can prove the full timeout
    /// path (sender → transport → cleanup) without waiting 30 seconds
    /// or stubbing `map_rpc_error` in isolation. It is deliberately
    /// NOT exposed to downstream crates because a per-plugin override
    /// would break the §11.1 host-enforced deadline contract.
    pub(crate) fn with_timeout(
        plugin_id: String,
        rpc: PluginRpcClient,
        capabilities: CapabilitiesResponse,
        timeout: Duration,
    ) -> Self {
        Self {
            plugin_id,
            rpc,
            capabilities,
            timeout,
        }
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn capabilities(&self) -> &CapabilitiesResponse {
        &self.capabilities
    }

    /// Fire a host → plugin **notification** on the same transport
    /// `dispatch` uses. Used by the §7.1 streaming path: the host
    /// emits `inbound/stream_frame` / `inbound/stream_end` frames on
    /// every agent-produced delta so the plugin can stream progress
    /// back to its upstream. Fire-and-forget by protocol; no response
    /// is expected and none is waited for.
    ///
    /// Errors surface exactly as the underlying [`RpcError`] so the
    /// caller (typically the stream callback) can decide whether to
    /// log and continue or short-circuit the stream.
    pub async fn notify(&self, method: &str, params: &serde_json::Value) -> Result<(), RpcError> {
        self.rpc.notify(method, params).await
    }

    /// Send one `dispatch_outbound` request. Maps every RPC failure to
    /// a [`ChannelError`] per §9.4; callers never see a [`RpcError`].
    pub async fn dispatch(
        &self,
        req: DispatchOutbound,
    ) -> Result<DispatchOutboundResult, ChannelError> {
        if !self.capabilities.outbound {
            return Err(ChannelError::Config(format!(
                "plugin '{}' does not advertise outbound capability",
                self.plugin_id
            )));
        }

        let params = serde_json::to_value(&req).map_err(|err| {
            ChannelError::SendFailed(format!(
                "encoding dispatch_outbound for '{}' failed: {err}",
                self.plugin_id
            ))
        })?;

        match self
            .rpc
            .call_value_with_timeout("dispatch_outbound", params, Some(self.timeout))
            .await
        {
            Ok(value) => serde_json::from_value::<DispatchOutboundResult>(value).map_err(|err| {
                ChannelError::SendFailed(format!(
                    "decoding dispatch_outbound reply from '{}' failed: {err}",
                    self.plugin_id
                ))
            }),
            Err(err) => Err(self.map_rpc_error(err)),
        }
    }

    fn map_rpc_error(&self, err: RpcError) -> ChannelError {
        match err {
            RpcError::Timeout(_) => ChannelError::Connection(format!(
                "plugin '{}' dispatch_outbound timed out",
                self.plugin_id
            )),
            RpcError::Disconnected => {
                ChannelError::Connection(format!("plugin '{}' unavailable", self.plugin_id))
            }
            // §9.4: respawning plugin → outbound aborted. The host-
            // authored message already includes the plugin id and the
            // mandated "respawning; outbound aborted" wording.
            RpcError::HostAborted(msg) => ChannelError::Connection(msg),
            RpcError::Remote { code, message } => match PluginErrorCode::from_i32(code) {
                Some(
                    PluginErrorCode::MethodNotFound
                    | PluginErrorCode::InvalidParams
                    | PluginErrorCode::AccountNotFound
                    | PluginErrorCode::ChannelConfigRejected,
                ) => ChannelError::Config(format!(
                    "plugin '{}' rejected dispatch_outbound ({code}): {message}",
                    self.plugin_id
                )),
                Some(PluginErrorCode::ConfigRejected) => {
                    // §9.4: ConfigRejected is lifecycle-only. If a
                    // plugin emits it from dispatch_outbound it's a
                    // plugin bug; surface as Config so the caller does
                    // not retry, and log loudly for `garyx doctor`.
                    warn!(
                        plugin = %self.plugin_id,
                        message = %message,
                        "plugin emitted ConfigRejected from dispatch_outbound; this is a plugin bug (garyx doctor advisory)"
                    );
                    ChannelError::Config(format!(
                        "plugin '{}' reported ConfigRejected from dispatch_outbound (plugin bug): {message}",
                        self.plugin_id
                    ))
                }
                _ => ChannelError::SendFailed(format!(
                    "plugin '{}' dispatch_outbound error ({code}): {message}",
                    self.plugin_id
                )),
            },
            RpcError::MalformedResponse(msg) => ChannelError::SendFailed(format!(
                "plugin '{}' returned malformed dispatch_outbound response: {msg}",
                self.plugin_id
            )),
            // A codec error bubbling to the caller means the transport
            // itself is broken (framing / I/O). Map to Connection so
            // retry policies do the right thing.
            RpcError::Codec(err) => ChannelError::Connection(format!(
                "plugin '{}' codec error during dispatch_outbound: {err}",
                self.plugin_id
            )),
            RpcError::Serialization(err) => ChannelError::SendFailed(format!(
                "plugin '{}' serialization error in dispatch_outbound: {err}",
                self.plugin_id
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
