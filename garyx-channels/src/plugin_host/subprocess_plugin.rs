//! `ChannelPlugin` impl that wraps a subprocess plugin so the
//! manager can treat built-in and subprocess plugins uniformly.
//!
//! This is the Scheme-A mirror of [`crate::plugin::ManagedChannelPlugin`]:
//! same trait, same semantics, different execution model. Everything
//! the trait exposes (metadata / capabilities / schema / auth_flow /
//! dispatch_outbound / account validation) is forwarded over JSON-RPC
//! to the child process through [`crate::plugin_host::PluginRpcClient`].
//!
//! Account state is deliberately NOT part of the `ChannelPlugin`
//! trait: the host owns `ChannelsConfig`, plugins merely reflect
//! whatever account list the host hands them. When that list
//! changes at runtime, the host pushes the new set via the
//! subprocess-specific [`SubprocessChannelPlugin::reload_accounts`]
//! method, which maps to the `accounts/reload` RPC (§6.5). Built-in
//! plugins have their own reload path and don't need this hook.
//!
//! What lives **here** vs in [`crate::plugin_host::SubprocessPlugin`]:
//! - `SubprocessPlugin` owns the child — spawn / stdio / exit /
//!   respawn mechanics. It does not know about the `ChannelPlugin`
//!   trait at all.
//! - This wrapper holds a cloned [`PluginRpcClient`] handle + the
//!   manifest-derived metadata / schema / capabilities. The manager
//!   keeps both behind an `Arc` so respawn can atomically swap the
//!   underlying client without the gateway noticing.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::auth_flow::AuthFlowExecutor;
use crate::channel_trait::ChannelError;
use crate::dispatcher::{OutboundMessage, SendMessageResult};
use crate::plugin::{
    AccountValidationResult, ChannelPlugin, LIFECYCLE_RPC_TIMEOUT, PluginAccountUi,
    PluginConversationEndpoint, PluginConversationNode, PluginLifecycle, PluginMetadata,
};
use crate::plugin_host::auth_flow_bridge::SubprocessAuthFlowExecutor;
use crate::plugin_host::manifest::{AccountRootBehavior, ManifestCapabilities};
use crate::plugin_host::protocol::{
    AccountDescriptor, CapabilitiesResponse, DispatchOutbound, DispatchOutboundResult,
    PluginErrorCode, ReloadAccountsParams, ResolveAccountUiParams, ResolveAccountUiResult,
    UiConversationNode, UiEndpointDescriptor, ValidateAccountParams, ValidateAccountResult,
};
use crate::plugin_host::{PluginRpcClient, PluginSenderHandle, RpcError};

/// Thin `ChannelPlugin` façade over a subprocess plugin. All trait
/// methods either return data captured at registration time (metadata
/// / schema / capabilities) or forward over the current live client
/// (auth_flow / dispatch_outbound).
///
/// `client` is behind a `Mutex` so respawn can swap it without
/// tearing an in-flight caller: each trait method grabs a cloned
/// `PluginRpcClient` under the lock and releases before `await`ing.
/// `PluginRpcClient` itself is `Arc`-backed, so the clone is cheap.
pub struct SubprocessChannelPlugin {
    metadata: PluginMetadata,
    schema: Value,
    capabilities: ManifestCapabilities,
    root_behavior: AccountRootBehavior,
    /// Exposed to the dispatcher via [`sender_handle`] for outbound
    /// routing. The handle carries the RPC client's capabilities
    /// gating (refuses dispatch if the plugin didn't advertise
    /// outbound) and maps RPC errors to `ChannelError` per §9.4.
    sender: PluginSenderHandle,
    /// Live RPC client. Atomically swappable on respawn.
    client: Arc<Mutex<PluginRpcClient>>,
}

impl SubprocessChannelPlugin {
    pub fn new(
        metadata: PluginMetadata,
        schema: Value,
        capabilities: ManifestCapabilities,
        root_behavior: AccountRootBehavior,
        sender: PluginSenderHandle,
        client: PluginRpcClient,
    ) -> Self {
        Self {
            metadata,
            schema,
            capabilities,
            root_behavior,
            sender,
            client: Arc::new(Mutex::new(client)),
        }
    }

    /// Hand the current sender handle back to the dispatcher. Used
    /// by `ChannelPluginManager::register_subprocess_plugin` to
    /// publish into the SwappableDispatcher's routing table.
    pub fn sender_handle(&self) -> PluginSenderHandle {
        self.sender.clone()
    }

    /// Atomically swap the RPC client (called by respawn). The old
    /// client's pending RPCs have already been aborted by the
    /// manager via `PluginRpcClient::abort_pending` before this is
    /// called, so the swap is safe and trait method calls that fire
    /// after it see the new child immediately.
    pub fn replace_client(&self, client: PluginRpcClient) {
        *self
            .client
            .lock()
            .expect("subprocess client mutex poisoned") = client;
    }

    fn rpc_client(&self) -> PluginRpcClient {
        self.client
            .lock()
            .expect("subprocess client mutex poisoned")
            .clone()
    }

    // `reload_accounts` lives on the `ChannelPlugin` trait so
    // callers with `Arc<dyn ChannelPlugin>` can invoke it
    // uniformly. See the trait impl below.
}

#[async_trait]
impl PluginLifecycle for SubprocessChannelPlugin {
    /// Subprocess lifecycle (spawn / initialize / start / stop /
    /// shutdown) is driven by `ChannelPluginManager` directly
    /// through the JSON-RPC client — not through these trait
    /// methods. Keeping these as no-ops means the manager can slot
    /// a `SubprocessChannelPlugin` into the same `PluginEntry` map
    /// used for built-ins without a second lifecycle pass running.
    async fn initialize(&self) -> Result<(), String> {
        Ok(())
    }
    async fn start(&self) -> Result<(), String> {
        Ok(())
    }
    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }
    async fn cleanup(&self) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
impl ChannelPlugin for SubprocessChannelPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn account_root_behavior(&self) -> AccountRootBehavior {
        self.root_behavior
    }

    fn capabilities(&self) -> ManifestCapabilities {
        self.capabilities.clone()
    }

    fn schema(&self) -> Value {
        self.schema.clone()
    }

    fn auth_flow(&self) -> Option<Arc<dyn AuthFlowExecutor>> {
        // The bridge is cheap — one PluginRpcClient clone + one Arc
        // allocation per lookup. No need to cache; the executor is
        // stateless (session state lives inside the plugin).
        Some(Arc::new(SubprocessAuthFlowExecutor::new(
            self.metadata.id.clone(),
            self.rpc_client(),
        )))
    }

    async fn validate_account_config(
        &self,
        account: AccountDescriptor,
    ) -> Result<AccountValidationResult, String> {
        let req = ValidateAccountParams { account };
        let result: ValidateAccountResult = match self
            .rpc_client()
            .call_with_timeout("accounts/validate", &req, Some(LIFECYCLE_RPC_TIMEOUT))
            .await
        {
            Ok(result) => result,
            Err(RpcError::Remote { code, .. })
                if code == PluginErrorCode::MethodNotFound as i32 =>
            {
                return Ok(AccountValidationResult {
                    validated: false,
                    message: format!(
                        "plugin '{}' does not expose account connectivity validation",
                        self.metadata.id
                    ),
                });
            }
            Err(error) => {
                return Err(format!(
                    "plugin '{}' accounts/validate: {error}",
                    self.metadata.id
                ));
            }
        };
        Ok(AccountValidationResult {
            validated: result.validated,
            message: if result.message.trim().is_empty() {
                "Plugin account validation completed.".to_owned()
            } else {
                result.message
            },
        })
    }

    async fn dispatch_outbound(
        &self,
        msg: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        // Reuse the existing sender handle — same capability gate,
        // same error mapping, same timeout contract. Converting
        // `OutboundMessage` to the plugin_host DTO shape is a
        // field-copy since both types were designed alongside.
        let req = DispatchOutbound {
            account_id: msg.account_id,
            chat_id: msg.chat_id,
            delivery_target_type: msg.delivery_target_type,
            delivery_target_id: msg.delivery_target_id,
            content: msg.content,
            reply_to: msg.reply_to,
            thread_id: msg.thread_id,
        };
        let result: DispatchOutboundResult = self.sender.dispatch(req).await?;
        Ok(SendMessageResult {
            message_ids: result.message_ids,
        })
    }

    /// Forward `accounts/reload` (§6.5) to the subprocess. Host
    /// holds the authoritative `ChannelsConfig`; this pushes a
    /// snapshot down so the plugin upserts / tears down its
    /// internal account store in place — cheaper than respawn for
    /// pure account-config edits.
    async fn reload_accounts(&self, accounts: Vec<AccountDescriptor>) -> Result<(), String> {
        let req = ReloadAccountsParams { accounts };
        let _: Value = self
            .rpc_client()
            .call_with_timeout("accounts/reload", &req, Some(LIFECYCLE_RPC_TIMEOUT))
            .await
            .map_err(|e| format!("plugin '{}' accounts/reload: {e}", self.metadata.id))?;
        Ok(())
    }

    async fn resolve_account_ui(
        &self,
        account_id: &str,
        endpoints: &[PluginConversationEndpoint],
    ) -> Option<PluginAccountUi> {
        let req = ResolveAccountUiParams {
            account_id: account_id.to_owned(),
            endpoints: endpoints
                .iter()
                .map(|endpoint| UiEndpointDescriptor {
                    endpoint_key: endpoint.endpoint_key.clone(),
                    channel: endpoint.channel.clone(),
                    account_id: endpoint.account_id.clone(),
                    binding_key: endpoint.binding_key.clone(),
                    chat_id: endpoint.chat_id.clone(),
                    delivery_target_type: endpoint.delivery_target_type.clone(),
                    delivery_target_id: endpoint.delivery_target_id.clone(),
                    delivery_thread_id: endpoint.delivery_thread_id.clone(),
                    display_label: endpoint.display_label.clone(),
                    thread_id: endpoint.thread_id.clone(),
                    thread_label: endpoint.thread_label.clone(),
                    workspace_dir: endpoint.workspace_dir.clone(),
                    thread_updated_at: endpoint.thread_updated_at.clone(),
                    last_inbound_at: endpoint.last_inbound_at.clone(),
                    last_delivery_at: endpoint.last_delivery_at.clone(),
                    conversation_kind: endpoint.conversation_kind.clone(),
                    conversation_label: endpoint.conversation_label.clone(),
                })
                .collect(),
        };
        let result: ResolveAccountUiResult = match self
            .rpc_client()
            .call_with_timeout("ui/resolve_account", &req, Some(LIFECYCLE_RPC_TIMEOUT))
            .await
        {
            Ok(result) => result,
            Err(_) => {
                return None;
            }
        };
        Some(PluginAccountUi {
            default_open_endpoint_key: result.default_open_endpoint_key,
            conversation_nodes: result
                .conversation_nodes
                .into_iter()
                .map(|node: UiConversationNode| PluginConversationNode {
                    id: node.id,
                    endpoint_key: node.endpoint_key,
                    kind: node.kind,
                    title: node.title,
                    badge: node.badge,
                    latest_activity: node.latest_activity,
                    openable: node.openable,
                })
                .collect(),
        })
    }
}

// Suppress the "unused" warning when no doc tests exercise the
// `CapabilitiesResponse` import — retained for future use when
// capabilities need to be re-fetched at runtime vs manifest-time.
#[allow(dead_code)]
fn _capabilities_response_marker() -> Option<CapabilitiesResponse> {
    None
}

#[cfg(test)]
mod tests;
