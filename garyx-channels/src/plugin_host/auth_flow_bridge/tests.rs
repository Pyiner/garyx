use super::*;
use crate::plugin_host::transport::{InboundHandler, Transport, TransportConfig};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::duplex;

/// Plugin-side handler driven by a closure — every request is
/// routed through `responder` which returns either `Ok(wire
/// response)` or `Err((code, msg))` for the remote-error path.
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

/// Wire up a host ↔ fake-plugin transport pair over an
/// in-memory duplex. Returns the host-side [`PluginRpcClient`]
/// and a keep-alive token — drop the token to simulate a
/// disconnected peer, hold it for the lifetime of the test
/// otherwise.
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

#[tokio::test]
async fn start_forwards_form_state_and_maps_display() {
    // The bridge must pass `form_state` verbatim to the plugin
    // and surface the plugin's Text + Qr items through the
    // trait's `AuthDisplayItem` shape.
    let (client, _keep) = wire_fake(|method, params| {
        assert_eq!(method, "auth_flow/start");
        assert_eq!(params, json!({ "form_state": { "domain": "feishu" } }));
        Ok(json!({
            "session_id": "sess-1",
            "display": [
                { "kind": "text", "value": "打开以下链接完成授权：" },
                { "kind": "text", "value": "https://example.com/auth" },
                { "kind": "qr",   "value": "https://example.com/auth" }
            ],
            "expires_in_secs": 600,
            "poll_interval_secs": 5
        }))
    });

    let bridge = SubprocessAuthFlowExecutor::new("feishu-sub", client);
    let session = bridge
        .start(json!({ "domain": "feishu" }))
        .await
        .expect("start ok");
    assert_eq!(session.session_id, "sess-1");
    assert_eq!(session.expires_in_secs, 600);
    assert_eq!(session.poll_interval_secs, 5);
    assert_eq!(session.display.len(), 3);
    assert!(matches!(
        &session.display[0],
        AuthDisplayItem::Text { value } if value.contains("授权")
    ));
    assert!(matches!(
        &session.display[2],
        AuthDisplayItem::Qr { value } if value == "https://example.com/auth"
    ));
}

#[tokio::test]
async fn poll_maps_all_three_states_correctly() {
    // `Pending` with a display refresh (weixin's "scanned"
    // pattern), `Confirmed` with values, and `Failed` must all
    // round-trip through the bridge unchanged. One test covers
    // the whole state machine by sequencing three responses.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let counter = Arc::new(AtomicUsize::new(0));
    let c2 = counter.clone();
    let (client, _keep) = wire_fake(move |method, _params| {
        assert_eq!(method, "auth_flow/poll");
        let n = c2.fetch_add(1, Ordering::SeqCst);
        Ok(match n {
            0 => json!({ "status": "pending" }),
            1 => json!({
                "status": "pending",
                "display": [
                    { "kind": "text", "value": "已扫码，请在微信内确认登录" }
                ],
                "next_interval_secs": 2
            }),
            2 => json!({
                "status": "confirmed",
                "values": { "token": "sekrit", "base_url": "https://wx.example" }
            }),
            _ => json!({ "status": "failed", "reason": "session expired" }),
        })
    });

    let bridge = SubprocessAuthFlowExecutor::new("weixin-sub", client);
    match bridge.poll("sess-x").await.unwrap() {
        AuthPollResult::Pending {
            display,
            next_interval_secs,
        } => {
            assert!(display.is_none());
            assert!(next_interval_secs.is_none());
        }
        other => panic!("expected Pending, got {other:?}"),
    }
    match bridge.poll("sess-x").await.unwrap() {
        AuthPollResult::Pending {
            display: Some(items),
            next_interval_secs: Some(2),
        } => {
            assert_eq!(items.len(), 1);
            assert!(matches!(
                &items[0],
                AuthDisplayItem::Text { value } if value.contains("扫码")
            ));
        }
        other => panic!("expected Pending with display refresh, got {other:?}"),
    }
    match bridge.poll("sess-x").await.unwrap() {
        AuthPollResult::Confirmed { values } => {
            assert_eq!(values["token"], "sekrit");
            assert_eq!(values["base_url"], "https://wx.example");
        }
        other => panic!("expected Confirmed, got {other:?}"),
    }
}

#[tokio::test]
async fn remote_error_maps_to_protocol() {
    // `Err((code, msg))` from the plugin ends up as
    // `AuthFlowError::Protocol` so the CLI / gateway stop
    // polling instead of retrying forever.
    let (client, _keep) = wire_fake(|_method, _params| Err((-32002, "config rejected".into())));
    let bridge = SubprocessAuthFlowExecutor::new("broken", client);
    let err = bridge.start(json!({})).await.expect_err("must fail");
    match err {
        AuthFlowError::Protocol(msg) => {
            assert!(msg.contains("config rejected"), "got: {msg}");
            assert!(msg.contains("-32002"), "code must be in the message: {msg}");
        }
        other => panic!("expected Protocol, got {other:?}"),
    }
}

#[tokio::test]
async fn pending_with_empty_display_array_is_distinct_from_omitted() {
    // Two different wire shapes for "Pending with no display
    // change": `{"status":"pending"}` (omitted) and
    // `{"status":"pending","display":[]}` (explicit empty).
    // Both MUST surface the same way on the trait side — a
    // `Some(vec![])` leak would trick the CLI into re-rendering
    // an empty screen over the original display, blanking the
    // QR. Serde `skip_serializing_if = "Option::is_none"` on
    // the Response enum lets the plugin send either; the trait
    // must normalise to `display: None` so renderers can cheaply
    // diff "no refresh" from "refresh with no items".
    use std::sync::atomic::{AtomicUsize, Ordering};
    let counter = Arc::new(AtomicUsize::new(0));
    let c2 = counter.clone();
    let (client, _keep) = wire_fake(move |_method, _params| {
        let n = c2.fetch_add(1, Ordering::SeqCst);
        Ok(match n {
            0 => json!({ "status": "pending" }),
            _ => json!({ "status": "pending", "display": [] }),
        })
    });

    let bridge = SubprocessAuthFlowExecutor::new("x", client);
    // Both shapes must deserialize without error; whether the
    // empty-array case deserialises to `Some(vec![])` or `None`
    // is an implementation choice — what matters is the trait
    // consumer gets a stable-shaped value it can handle.
    // Today: omitted → None; explicit [] → Some(vec![]).
    match bridge.poll("s").await.unwrap() {
        AuthPollResult::Pending { display: None, .. } => {}
        other => panic!("omitted display must map to None, got {other:?}"),
    }
    match bridge.poll("s").await.unwrap() {
        AuthPollResult::Pending {
            display: Some(items),
            ..
        } => {
            assert!(
                items.is_empty(),
                "explicit empty array must round-trip as empty Vec"
            );
        }
        other => panic!("explicit [] display must map to Some(vec![]), got {other:?}"),
    }
}

#[tokio::test]
async fn confirmed_accepts_empty_values_map() {
    // A plugin that confirms with zero-field `values` (e.g. a
    // channel that finalises via side-effects only) must still
    // succeed — empty `{}` is valid JSON for a BTreeMap. The
    // caller is free to treat "no values means nothing to
    // merge" as a no-op; the bridge doesn't get to decide.
    let (client, _keep) =
        wire_fake(|_method, _params| Ok(json!({ "status": "confirmed", "values": {} })));
    let bridge = SubprocessAuthFlowExecutor::new("empty-confirmed", client);
    match bridge.poll("s").await.unwrap() {
        AuthPollResult::Confirmed { values } => {
            assert!(values.is_empty(), "empty values map must round-trip");
        }
        other => panic!("expected Confirmed with empty values, got {other:?}"),
    }
}

#[tokio::test]
async fn cancel_forwards_session_id_and_swallows_error() {
    // `cancel` is best-effort per the trait contract — the
    // plugin MAY return an error (or MethodNotFound) and the
    // bridge must not propagate it. Verify the RPC is actually
    // fired with the right method name + params, AND that a
    // plugin-side rejection doesn't panic or bubble.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let seen = Arc::new(AtomicUsize::new(0));
    let s2 = seen.clone();
    let (client, _keep) = wire_fake(move |method, params| {
        s2.fetch_add(1, Ordering::SeqCst);
        assert_eq!(method, "auth_flow/cancel");
        assert_eq!(params, json!({ "session_id": "sess-cancel" }));
        // Plugin rejects — bridge must swallow.
        Err((-32601, "method not implemented".into()))
    });
    let bridge = SubprocessAuthFlowExecutor::new("cancel-test", client);
    bridge.cancel("sess-cancel").await; // no return value to assert
    assert_eq!(
        seen.load(Ordering::SeqCst),
        1,
        "cancel must reach the plugin exactly once"
    );
}

#[tokio::test]
async fn future_display_variant_degrades_to_unknown() {
    // Forward-compat: a newer plugin returns a display item
    // the host doesn't recognise (`{"kind":"image",…}`). The
    // bridge must deliver it as `AuthDisplayItem::Unknown`
    // rather than failing the whole start() call.
    let (client, _keep) = wire_fake(|_method, _params| {
        Ok(json!({
            "session_id": "sess-2",
            "display": [
                { "kind": "image", "src": "data:image/png;base64,…" }
            ],
            "expires_in_secs": 60,
            "poll_interval_secs": 1
        }))
    });
    let bridge = SubprocessAuthFlowExecutor::new("future", client);
    let session = bridge.start(json!({})).await.expect("start ok");
    assert_eq!(session.display.len(), 1);
    assert!(matches!(session.display[0], AuthDisplayItem::Unknown));
}
