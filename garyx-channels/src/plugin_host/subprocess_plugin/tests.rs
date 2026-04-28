//! Exercises the non-trait `reload_accounts` helper over an
//! in-memory duplex transport. Mirrors the `wire_fake` pattern
//! in `auth_flow_bridge::tests` — one host-side
//! `PluginRpcClient` plus a fake plugin handler that checks the
//! incoming frame and returns `{}`.

use super::*;
use crate::plugin::PluginMetadata;
use crate::plugin_host::manifest::DeliveryModel;
use crate::plugin_host::transport::{InboundHandler, Transport, TransportConfig};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::duplex;

struct ClosurePlugin<F>
where
    F: Fn(&str, Value) -> Result<Value, (i32, String)> + Send + Sync + 'static,
{
    responder: Arc<F>,
}

#[async_trait]
impl<F> InboundHandler for ClosurePlugin<F>
where
    F: Fn(&str, Value) -> Result<Value, (i32, String)> + Send + Sync + 'static,
{
    async fn on_request(&self, method: String, params: Value) -> Result<Value, (i32, String)> {
        (self.responder)(method.as_str(), params)
    }
    async fn on_notification(&self, _method: String, _params: Value) {}
}

fn wire_fake<F>(responder: F) -> (PluginRpcClient, PluginRpcClient)
where
    F: Fn(&str, Value) -> Result<Value, (i32, String)> + Send + Sync + 'static,
{
    let (host_rw, plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (plugin_r, plugin_w) = tokio::io::split(plugin_rw);

    struct HostDrop;
    #[async_trait]
    impl InboundHandler for HostDrop {
        async fn on_request(
            &self,
            _method: String,
            _params: Value,
        ) -> Result<Value, (i32, String)> {
            Err((-32601, "host does not accept inbound in this test".into()))
        }
        async fn on_notification(&self, _method: String, _params: Value) {}
    }

    let (host_client, _host_handles) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: "test".into(),
            default_rpc_timeout: Duration::from_secs(10),
            ..Default::default()
        },
        Arc::new(HostDrop),
    );

    let plugin_handler = ClosurePlugin {
        responder: Arc::new(responder),
    };
    let (plugin_keep_alive, _plugin_handles) = Transport::spawn(
        plugin_r,
        plugin_w,
        TransportConfig {
            plugin_id: "test-peer".into(),
            default_rpc_timeout: Duration::from_secs(10),
            ..Default::default()
        },
        Arc::new(plugin_handler),
    );

    (host_client, plugin_keep_alive)
}

/// Build a `SubprocessChannelPlugin` wired to an in-memory RPC
/// client. Metadata / schema / capabilities / sender are stubbed
/// — this harness exercises `reload_accounts` only.
fn make_plugin(client: PluginRpcClient) -> SubprocessChannelPlugin {
    let metadata = PluginMetadata {
        id: "fake".into(),
        aliases: vec![],
        display_name: "Fake".into(),
        version: "0.0.1".into(),
        description: String::new(),
        source: "test".into(),
        config_methods: vec![],
    };
    let capabilities = ManifestCapabilities {
        outbound: true,
        inbound: true,
        streaming: false,
        images: false,
        files: false,
        hot_reload_accounts: false,
        requires_public_url: false,
        needs_host_ingress: false,
        delivery_model: DeliveryModel::PullExplicitAck,
    };
    let sender = PluginSenderHandle::new(
        metadata.id.clone(),
        client.clone(),
        CapabilitiesResponse {
            outbound: true,
            inbound: true,
            streaming: false,
            images: false,
            files: false,
        },
    );
    SubprocessChannelPlugin::new(
        metadata,
        serde_json::json!({}),
        capabilities,
        AccountRootBehavior::OpenDefault,
        sender,
        client,
    )
}

#[tokio::test]
async fn reload_accounts_forwards_method_and_params() {
    // Host sends a 2-account snapshot; the plugin side asserts
    // the RPC lands on `accounts/reload` with the exact account
    // list (preserving order + the `enabled` toggles) and acks
    // `{}`. No trait dispatch involved — this is the pure
    // subprocess fast-path.
    let (client, _keep) = wire_fake(|method, params| {
        assert_eq!(method, "accounts/reload");
        assert_eq!(
            params,
            json!({
                "accounts": [
                    { "id": "a1", "enabled": true,  "config": { "token": "x" } },
                    { "id": "a2", "enabled": false, "config": { "token": "y" } }
                ]
            })
        );
        Ok(json!({}))
    });

    let plugin = make_plugin(client);
    let accounts = vec![
        AccountDescriptor {
            id: "a1".into(),
            enabled: true,
            config: json!({ "token": "x" }),
        },
        AccountDescriptor {
            id: "a2".into(),
            enabled: false,
            config: json!({ "token": "y" }),
        },
    ];
    plugin.reload_accounts(accounts).await.expect("reload ok");
}

#[tokio::test]
async fn reload_accounts_surfaces_plugin_error_with_context() {
    // A ConfigRejected (§5.3) from the plugin MUST bubble as a
    // `Err(String)` carrying both the plugin id and the method
    // name so the gateway log / HTTP layer can relay verbatim.
    let (client, _keep) = wire_fake(|_method, _params| Err((-32005, "refusing snapshot".into())));
    let plugin = make_plugin(client);
    let err = plugin.reload_accounts(vec![]).await.expect_err("must fail");
    assert!(err.contains("fake"), "error mentions plugin id: {err}");
    assert!(
        err.contains("accounts/reload"),
        "error mentions method: {err}"
    );
    assert!(
        err.contains("refusing snapshot"),
        "error carries plugin message: {err}"
    );
}

#[tokio::test]
async fn reload_accounts_handles_empty_list() {
    // "Drop everything" is represented as an empty `accounts`
    // vector — must still serialise as `{"accounts":[]}`, not
    // `null` or absent.
    let (client, _keep) = wire_fake(|method, params| {
        assert_eq!(method, "accounts/reload");
        assert_eq!(params, json!({ "accounts": [] }));
        Ok(json!({}))
    });
    let plugin = make_plugin(client);
    plugin.reload_accounts(vec![]).await.expect("reload ok");
}
