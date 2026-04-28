use super::*;
use crate::plugin_host::transport::{InboundHandler, Transport, TransportConfig};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::io::duplex;

fn caps(outbound: bool) -> CapabilitiesResponse {
    CapabilitiesResponse {
        outbound,
        inbound: false,
        streaming: false,
        images: false,
        files: false,
    }
}

fn sample_request() -> DispatchOutbound {
    DispatchOutbound {
        account_id: "acct".into(),
        chat_id: "chat-1".into(),
        delivery_target_type: "chat_id".into(),
        delivery_target_id: "chat-1".into(),
        text: "hi".into(),
        reply_to: None,
        thread_id: None,
    }
}

type DispatchResponder = dyn Fn(DispatchOutbound) -> Result<Value, (i32, String)> + Send + Sync;

/// A plugin-side handler driven by a closure. Every `dispatch_outbound`
/// call runs `responder`; everything else responds with
/// MethodNotFound. Sync closure is enough because none of the error-
/// mapping tests need asynchronous gating on the plugin side.
struct ClosurePlugin {
    dispatch_count: Arc<std::sync::atomic::AtomicUsize>,
    responder: Arc<DispatchResponder>,
}

#[async_trait]
impl InboundHandler for ClosurePlugin {
    async fn on_request(&self, method: String, params: Value) -> Result<Value, (i32, String)> {
        match method.as_str() {
            "dispatch_outbound" => {
                self.dispatch_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let req: DispatchOutbound = serde_json::from_value(params)
                    .map_err(|err| (-32602, format!("bad params: {err}")))?;
                (self.responder)(req)
            }
            other => Err((-32601, format!("no {other}"))),
        }
    }

    async fn on_notification(&self, _method: String, _params: Value) {}
}

/// Stand up a host ↔ fake-plugin pair and return the host-side
/// [`PluginRpcClient`], the shared dispatch counter (so tests can
/// assert whether a round-trip actually happened), and a
/// keep-alive handle that must stay in scope for the duration of
/// the test.
///
/// If the caller drops the keep-alive the plugin-side writer closes,
/// the host reader hits EOF, and pending RPCs resolve as
/// `Disconnected` — which is a useful mode for the disconnected-
/// peer test but fatal for every other test.
fn wire_fake_plugin(
    responder: impl Fn(DispatchOutbound) -> Result<Value, (i32, String)> + Send + Sync + 'static,
) -> (
    PluginRpcClient,
    Arc<std::sync::atomic::AtomicUsize>,
    PluginRpcClient,
) {
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
            default_rpc_timeout: Duration::from_secs(60),
            ..Default::default()
        },
        Arc::new(HostDrop),
    );

    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let plugin_handler = ClosurePlugin {
        dispatch_count: counter.clone(),
        responder: Arc::new(responder),
    };

    let (plugin_keep_alive, _plugin_handles) = Transport::spawn(
        plugin_r,
        plugin_w,
        TransportConfig {
            plugin_id: "test-peer".into(),
            ..Default::default()
        },
        Arc::new(plugin_handler),
    );

    (host_client, counter, plugin_keep_alive)
}

// -- Capability gate --------------------------------------------------

#[tokio::test]
async fn outbound_capability_gate_short_circuits_before_rpc() {
    // If the plugin advertises !outbound we must NOT pay for a
    // round-trip. Verify by using a wired plugin that would count
    // the call if it came through.
    let (rpc, counter, _keep) = wire_fake_plugin(|_req| Ok(json!({"message_ids": ["ok"]})));
    let handle = PluginSenderHandle::new("gated".into(), rpc, caps(false));
    let err = handle
        .dispatch(sample_request())
        .await
        .expect_err("should short-circuit");
    match err {
        ChannelError::Config(msg) => assert!(
            msg.contains("outbound"),
            "error should mention outbound capability: {msg}"
        ),
        other => panic!("expected Config, got {other:?}"),
    }
    assert_eq!(
        counter.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "capability gate must NOT issue an RPC"
    );
}

// -- Happy path -------------------------------------------------------

#[tokio::test]
async fn success_returns_message_ids_verbatim() {
    let (rpc, counter, _keep) = wire_fake_plugin(|req| {
        assert_eq!(req.chat_id, "chat-1");
        Ok(json!({ "message_ids": ["plugin-msg-1", "plugin-msg-2"] }))
    });
    let handle = PluginSenderHandle::new("ok".into(), rpc, caps(true));
    let result = handle.dispatch(sample_request()).await.expect("ok");
    assert_eq!(result.message_ids, vec!["plugin-msg-1", "plugin-msg-2"]);
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
}

// -- Error mapping: Config (caller-correctable) -----------------------

async fn assert_config_error_for_code(code: PluginErrorCode) {
    let raw = code.as_i32();
    let (rpc, _, _keep) = wire_fake_plugin(move |_req| Err((raw, "rejected by plugin".into())));
    let handle = PluginSenderHandle::new("rej".into(), rpc, caps(true));
    let err = handle.dispatch(sample_request()).await.expect_err("err");
    match err {
        ChannelError::Config(msg) => {
            assert!(
                msg.contains("rejected by plugin"),
                "message should preserve plugin-provided text: {msg}"
            );
        }
        other => panic!("code {raw}: expected Config, got {other:?}"),
    }
}

#[tokio::test]
async fn method_not_found_maps_to_config() {
    assert_config_error_for_code(PluginErrorCode::MethodNotFound).await;
}

#[tokio::test]
async fn invalid_params_maps_to_config() {
    assert_config_error_for_code(PluginErrorCode::InvalidParams).await;
}

#[tokio::test]
async fn account_not_found_maps_to_config() {
    assert_config_error_for_code(PluginErrorCode::AccountNotFound).await;
}

#[tokio::test]
async fn channel_config_rejected_maps_to_config() {
    assert_config_error_for_code(PluginErrorCode::ChannelConfigRejected).await;
}

#[tokio::test]
async fn config_rejected_from_dispatch_is_bug_but_still_config() {
    // §9.4 "not expected here" — ConfigRejected is lifecycle-only.
    // Plugin bug path: we still return Config so the caller does not
    // retry, but the message should call out the bug so it surfaces
    // in logs.
    let (rpc, _, _keep) = wire_fake_plugin(|_req| {
        Err((
            PluginErrorCode::ConfigRejected.as_i32(),
            "wrong place".into(),
        ))
    });
    let handle = PluginSenderHandle::new("buggy".into(), rpc, caps(true));
    let err = handle.dispatch(sample_request()).await.expect_err("err");
    match err {
        ChannelError::Config(msg) => {
            assert!(
                msg.contains("plugin bug"),
                "bug advisory should be in the surface error: {msg}"
            );
            assert!(msg.contains("wrong place"), "must preserve message: {msg}");
        }
        other => panic!("expected Config, got {other:?}"),
    }
}

// -- Error mapping: SendFailed (unknown remote) -----------------------

#[tokio::test]
async fn unknown_remote_code_maps_to_send_failed() {
    // Any code the plugin emits that is NOT in the §9.4 Config
    // bucket (or the caller-correctable InternalError range) must
    // surface as SendFailed so the caller's retry policy treats it
    // as transient.
    let (rpc, _, _keep) = wire_fake_plugin(|_req| Err((-40001, "weird custom error".into())));
    let handle = PluginSenderHandle::new("x".into(), rpc, caps(true));
    let err = handle.dispatch(sample_request()).await.expect_err("err");
    match err {
        ChannelError::SendFailed(msg) => {
            assert!(msg.contains("-40001"), "error should preserve code: {msg}");
            assert!(msg.contains("weird custom error"), "message text: {msg}");
        }
        other => panic!("expected SendFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn internal_error_code_maps_to_send_failed() {
    // -32603 InternalError is NOT in the Config bucket per §9.4.
    let (rpc, _, _keep) =
        wire_fake_plugin(|_req| Err((PluginErrorCode::InternalError.as_i32(), "oops".into())));
    let handle = PluginSenderHandle::new("x".into(), rpc, caps(true));
    let err = handle.dispatch(sample_request()).await.expect_err("err");
    assert!(matches!(err, ChannelError::SendFailed(_)));
}

// -- Error mapping: Connection (transport) ----------------------------

#[tokio::test]
async fn disconnected_peer_maps_to_connection() {
    // Build a transport with a peer that never reads — then drop the
    // plugin side after spawning so the host sees Disconnected on
    // its pending request.
    let (host_rw, plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (plugin_r, _plugin_w) = tokio::io::split(plugin_rw);
    // Drop plugin read side immediately so writes back to host pile
    // up; we'll then drop the whole plugin to close the pipe.
    drop(plugin_r);

    struct Dumb;
    #[async_trait]
    impl InboundHandler for Dumb {
        async fn on_request(
            &self,
            _method: String,
            _params: Value,
        ) -> Result<Value, (i32, String)> {
            Err((-32601, "none".into()))
        }
        async fn on_notification(&self, _: String, _: Value) {}
    }

    let (rpc, _handles) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: "eof".into(),
            default_rpc_timeout: Duration::from_secs(30),
            ..Default::default()
        },
        Arc::new(Dumb),
    );
    let handle = PluginSenderHandle::new("eof".into(), rpc, caps(true));

    // Kick off the dispatch, then drop the plugin side so the host
    // reader hits EOF and fail_pending resolves the oneshot.
    let fut = tokio::spawn(async move { handle.dispatch(sample_request()).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    drop(_plugin_w);

    let err = fut.await.unwrap().expect_err("should be Connection");
    match err {
        ChannelError::Connection(msg) => {
            assert!(
                msg.contains("unavailable"),
                "Disconnected should map to Connection(unavailable): {msg}"
            );
        }
        other => panic!("expected Connection, got {other:?}"),
    }
}

#[tokio::test]
async fn rpc_timeout_maps_to_connection_end_to_end() {
    // Exercise the FULL timeout path: real transport round-trip,
    // plugin that never answers, sender enforcing its own
    // short-circuit deadline via `with_timeout`. This proves the
    // sender's `Some(timeout)` override beats the transport
    // default and that the resulting `RpcError::Timeout` flows
    // through `dispatch()` into `ChannelError::Connection`.
    //
    // The closure helper can't park (it's sync), so the plugin is
    // built inline with an async `on_request` that awaits
    // `pending()` forever.
    use tokio::io::{duplex, split};

    struct SilentPlugin;
    #[async_trait]
    impl InboundHandler for SilentPlugin {
        async fn on_request(
            &self,
            _method: String,
            _params: Value,
        ) -> Result<Value, (i32, String)> {
            // Park forever. The host-side sender timeout is what
            // resolves the call.
            std::future::pending::<()>().await;
            unreachable!()
        }
        async fn on_notification(&self, _: String, _: Value) {}
    }

    struct HostDrop;
    #[async_trait]
    impl InboundHandler for HostDrop {
        async fn on_request(
            &self,
            _method: String,
            _params: Value,
        ) -> Result<Value, (i32, String)> {
            Err((-32601, "no".into()))
        }
        async fn on_notification(&self, _: String, _: Value) {}
    }

    let (host_rw, plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = split(host_rw);
    let (plugin_r, plugin_w) = split(plugin_rw);
    let (host_rpc, _h) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: "timeout-host".into(),
            // Long transport default so we prove the SENDER
            // timeout is what fires, not the transport one.
            default_rpc_timeout: Duration::from_secs(60),
            ..Default::default()
        },
        Arc::new(HostDrop),
    );
    let (plugin_keep, _p) = Transport::spawn(
        plugin_r,
        plugin_w,
        TransportConfig {
            plugin_id: "timeout-peer".into(),
            ..Default::default()
        },
        Arc::new(SilentPlugin),
    );

    let handle = PluginSenderHandle::with_timeout(
        "silent".into(),
        host_rpc,
        caps(true),
        Duration::from_millis(80),
    );

    let started = std::time::Instant::now();
    let err = handle
        .dispatch(sample_request())
        .await
        .expect_err("silent plugin must trip the sender timeout");
    let elapsed = started.elapsed();
    match err {
        ChannelError::Connection(msg) => {
            assert!(
                msg.contains("timed out"),
                "Timeout should map to Connection(timed out): {msg}"
            );
        }
        other => panic!("expected Connection, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(5),
        "sender timeout should fire well before transport default; took {elapsed:?}"
    );
    drop(plugin_keep);
}

// -- Static mapping checks on the pure helper -------------------------
//
// `map_rpc_error` is the only branch of `dispatch` that isn't
// exercised by a live round-trip. Pin its table directly so the
// §9.4 mapping doesn't drift unnoticed.

#[tokio::test]
async fn map_rpc_error_covers_every_branch() {
    let (rpc, _, _keep) = wire_fake_plugin(|_| Ok(json!({"message_ids": []})));
    let h = PluginSenderHandle::new("p".into(), rpc, caps(true));

    for (code, want_config) in [
        (PluginErrorCode::MethodNotFound.as_i32(), true),
        (PluginErrorCode::InvalidParams.as_i32(), true),
        (PluginErrorCode::AccountNotFound.as_i32(), true),
        (PluginErrorCode::ChannelConfigRejected.as_i32(), true),
        (PluginErrorCode::ConfigRejected.as_i32(), true),
        (PluginErrorCode::InternalError.as_i32(), false),
        (-99999, false),
    ] {
        let mapped = h.map_rpc_error(RpcError::Remote {
            code,
            message: "x".into(),
        });
        if want_config {
            assert!(
                matches!(mapped, ChannelError::Config(_)),
                "code {code} should map to Config, got {mapped:?}"
            );
        } else {
            assert!(
                matches!(mapped, ChannelError::SendFailed(_)),
                "code {code} should map to SendFailed, got {mapped:?}"
            );
        }
    }

    // Transport-side mapping.
    assert!(matches!(
        h.map_rpc_error(RpcError::Timeout(Duration::from_secs(1))),
        ChannelError::Connection(_)
    ));
    assert!(matches!(
        h.map_rpc_error(RpcError::Disconnected),
        ChannelError::Connection(_)
    ));
    assert!(matches!(
        h.map_rpc_error(RpcError::MalformedResponse("bad".into())),
        ChannelError::SendFailed(_)
    ));
}

// -- End-to-end smoke: snapshot of request params on the wire --------

#[tokio::test]
async fn dispatch_outbound_request_carries_every_field() {
    // Guard against quietly dropping a field while refactoring
    // `PluginSenderHandle::dispatch`: snapshot the full
    // request as the plugin sees it.
    let captured: Arc<StdMutex<Option<DispatchOutbound>>> = Arc::new(StdMutex::new(None));
    let captured_clone = captured.clone();
    let (rpc, _, _keep) = wire_fake_plugin(move |req| {
        *captured_clone.lock().unwrap() = Some(req);
        Ok(json!({"message_ids": ["m"]}))
    });
    let handle = PluginSenderHandle::new("snap".into(), rpc, caps(true));

    let req = DispatchOutbound {
        account_id: "a".into(),
        chat_id: "c".into(),
        delivery_target_type: "chat_id".into(),
        delivery_target_id: "c".into(),
        text: "body".into(),
        reply_to: Some("r".into()),
        thread_id: Some("t".into()),
    };
    let _ = handle.dispatch(req.clone()).await.expect("ok");
    let got = captured.lock().unwrap().clone().expect("received");
    assert_eq!(got.account_id, req.account_id);
    assert_eq!(got.chat_id, req.chat_id);
    assert_eq!(got.delivery_target_type, req.delivery_target_type);
    assert_eq!(got.delivery_target_id, req.delivery_target_id);
    assert_eq!(got.text, req.text);
    assert_eq!(got.reply_to, req.reply_to);
    assert_eq!(got.thread_id, req.thread_id);
}
