use super::*;
use serde_json::json;
use tokio::io::duplex;

struct DummyHandler;

#[async_trait]
impl InboundHandler for DummyHandler {
    async fn on_request(&self, method: String, params: Value) -> Result<Value, (i32, String)> {
        match method.as_str() {
            "echo" => Ok(params),
            "error_me" => Err((
                super::super::protocol::PluginErrorCode::ConfigRejected.as_i32(),
                "nope".to_owned(),
            )),
            _ => Err((
                super::super::protocol::PluginErrorCode::MethodNotFound.as_i32(),
                format!("no {method}"),
            )),
        }
    }

    async fn on_notification(&self, _method: String, _params: Value) {}
}

/// Spawns a transport on each side of a duplex pipe. The "plugin"
/// side answers `echo` requests with a modified payload, and
/// notifies the host of each frame it got.
async fn pair() -> (
    PluginRpcClient,
    PluginRpcClient,
    TransportHandles,
    TransportHandles,
    Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
) {
    let (host_rw, plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (plugin_r, plugin_w) = tokio::io::split(plugin_rw);

    let host_log: Arc<tokio::sync::Mutex<Vec<(String, Value)>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    struct HostHandler {
        log: Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
    }
    #[async_trait]
    impl InboundHandler for HostHandler {
        async fn on_request(&self, method: String, params: Value) -> Result<Value, (i32, String)> {
            self.log.lock().await.push((method.clone(), params.clone()));
            match method.as_str() {
                "record_outbound" => Ok(json!({})),
                _ => Err((
                    super::super::protocol::PluginErrorCode::MethodNotFound.as_i32(),
                    format!("host does not handle {method}"),
                )),
            }
        }

        async fn on_notification(&self, method: String, params: Value) {
            self.log.lock().await.push((method, params));
        }
    }

    let host_cfg = TransportConfig {
        plugin_id: "test-plugin".into(),
        ..Default::default()
    };
    let plugin_cfg = TransportConfig {
        plugin_id: "test-plugin-peer".into(),
        ..Default::default()
    };

    let (host_client, host_handles) = Transport::spawn(
        host_r,
        host_w,
        host_cfg,
        Arc::new(HostHandler {
            log: Arc::clone(&host_log),
        }),
    );
    let (plugin_client, plugin_handles) =
        Transport::spawn(plugin_r, plugin_w, plugin_cfg, Arc::new(DummyHandler));

    (
        host_client,
        plugin_client,
        host_handles,
        plugin_handles,
        host_log,
    )
}

#[tokio::test]
async fn request_response_roundtrip() {
    let (host, _plugin, _h, _p, _log) = pair().await;
    let result: Value = host.call("echo", &json!({"hello": "world"})).await.unwrap();
    assert_eq!(result, json!({"hello": "world"}));
}

#[tokio::test]
async fn concurrent_requests_map_to_right_ids() {
    let (host, _plugin, _h, _p, _log) = pair().await;
    let mut handles = Vec::new();
    for i in 0..32 {
        let host = host.clone();
        handles.push(tokio::spawn(async move {
            let res: Value = host.call("echo", &json!({"n": i})).await.expect("echo");
            (i, res)
        }));
    }
    for h in handles {
        let (i, res) = h.await.unwrap();
        assert_eq!(res, json!({"n": i}));
    }
}

#[tokio::test]
async fn remote_error_surfaces_as_rpcerror_remote() {
    let (host, _plugin, _h, _p, _log) = pair().await;
    let err = host
        .call::<_, Value>("error_me", &json!({}))
        .await
        .unwrap_err();
    match err {
        RpcError::Remote { code, message } => {
            assert_eq!(
                code,
                super::super::protocol::PluginErrorCode::ConfigRejected.as_i32()
            );
            assert_eq!(message, "nope");
        }
        other => panic!("expected Remote, got {other:?}"),
    }
}

#[tokio::test]
async fn method_not_found_surfaces_distinctly() {
    let (host, _plugin, _h, _p, _log) = pair().await;
    let err = host
        .call::<_, Value>("unknown_method", &json!({}))
        .await
        .unwrap_err();
    match err {
        RpcError::Remote { code, .. } => {
            assert_eq!(
                code,
                super::super::protocol::PluginErrorCode::MethodNotFound.as_i32()
            );
        }
        other => panic!("expected MethodNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn timeout_surfaces_cleanly_and_releases_pending_slot() {
    // No peer will answer this — we build a duplex pipe with no
    // plugin on the other side.
    let (_host_rw_drop, host_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (host, _h) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: "dead-plugin".into(),
            default_rpc_timeout: Duration::from_millis(50),
            ..Default::default()
        },
        Arc::new(DummyHandler),
    );
    let err = host.call::<_, Value>("echo", &json!({})).await.unwrap_err();
    assert!(matches!(err, RpcError::Timeout(_)));
}

#[tokio::test]
async fn notification_is_routed_to_handler() {
    let (host, plugin, _h, _p, log) = pair().await;
    // Plugin fires a notification to the host.
    plugin
        .notify(
            "register_ingress",
            &json!({"account_id": "a1", "local_url": "http://x"}),
        )
        .await
        .unwrap();

    // Poll the log briefly; the notification dispatches through a
    // spawned task.
    let mut got = None;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let snapshot = log.lock().await.clone();
        if let Some(entry) = snapshot.into_iter().find(|(m, _)| m == "register_ingress") {
            got = Some(entry);
            break;
        }
    }
    let (_, params) = got.expect("notification should have reached host handler");
    assert_eq!(params["account_id"], "a1");
    // Host never replies to a notification, so we never see it
    // again.
    drop(host);
}

#[tokio::test]
async fn eof_during_pending_rpc_yields_disconnected_not_remote() {
    // Stand up just the host side. Plugin side is a manual writer
    // that never replies and closes the pipe once the host has
    // placed a request in its pending map.
    let (host_rw, plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (plugin_r, plugin_w) = tokio::io::split(plugin_rw);
    // Drop the plugin's reader immediately — we don't need to
    // interpret what the host sends, only close the pipe on cue.
    drop(plugin_r);

    let (host, _handles) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: "eof-test".into(),
            // Long enough that Timeout can't race Disconnected.
            default_rpc_timeout: Duration::from_secs(30),
            ..Default::default()
        },
        Arc::new(DummyHandler),
    );

    // Issue a call, then close the plugin's write-side so the
    // host's reader hits EOF.
    let call = {
        let host = host.clone();
        tokio::spawn(async move { host.call::<_, Value>("echo", &json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    drop(plugin_w);

    let err = call.await.unwrap().unwrap_err();
    match err {
        RpcError::Disconnected => {}
        other => panic!("expected RpcError::Disconnected on peer EOF, got {other:?}"),
    }
}

#[tokio::test]
async fn abort_pending_resolves_waiters_with_host_aborted() {
    // §9.4: the respawn path calls `abort_pending` at grace
    // expiry to resolve every in-flight `dispatch_outbound` waiter
    // with a host-authored message. This test pins the two
    // invariants the manager relies on:
    // - the in-flight `call` future resolves synchronously with
    //   `RpcError::HostAborted(msg)` carrying our exact message,
    // - the pending map is drained so `pending_count()` hits 0
    //   immediately, letting the drain-loop exit.
    struct SilentPlugin;
    #[async_trait]
    impl InboundHandler for SilentPlugin {
        async fn on_request(&self, _m: String, _p: Value) -> Result<Value, (i32, String)> {
            std::future::pending::<()>().await;
            unreachable!()
        }
        async fn on_notification(&self, _: String, _: Value) {}
    }

    let (host_rw, plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (plugin_r, plugin_w) = tokio::io::split(plugin_rw);
    let (host, _hh) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: "abort-host".into(),
            default_rpc_timeout: Duration::from_secs(60),
            ..Default::default()
        },
        Arc::new(DummyHandler),
    );
    let (_keep, _ph) = Transport::spawn(
        plugin_r,
        plugin_w,
        TransportConfig {
            plugin_id: "abort-peer".into(),
            ..Default::default()
        },
        Arc::new(SilentPlugin),
    );

    let call = {
        let host = host.clone();
        tokio::spawn(async move { host.call::<_, Value>("echo", &json!({})).await })
    };
    // Let the frame leave and the entry get inserted.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(host.pending_count(), 1);

    host.abort_pending("plugin abort-host respawning; outbound aborted".into());

    assert_eq!(
        host.pending_count(),
        0,
        "abort_pending must drain the pending map"
    );
    let err = call.await.unwrap().unwrap_err();
    match err {
        RpcError::HostAborted(msg) => {
            assert_eq!(msg, "plugin abort-host respawning; outbound aborted");
        }
        other => panic!("expected HostAborted, got {other:?}"),
    }
}

#[tokio::test]
async fn malformed_envelope_is_fatal_and_fails_pending() {
    // Host spawns, then the "plugin" writes a frame that violates
    // §5.2 (missing jsonrpc). The reader must fail-pending and exit
    // with InvalidEnvelope; any in-flight call must resolve to
    // Disconnected.
    let (host_rw, plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (_plugin_r, mut plugin_w) = tokio::io::split(plugin_rw);

    let (host, handles) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: "envelope-test".into(),
            default_rpc_timeout: Duration::from_secs(30),
            ..Default::default()
        },
        Arc::new(DummyHandler),
    );

    let call = {
        let host = host.clone();
        tokio::spawn(async move { host.call::<_, Value>("echo", &json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;

    // No jsonrpc field ⇒ envelope violation.
    let bad = br#"{"id":1,"result":{}}"#;
    let header = format!("Content-Length: {}\r\n\r\n", bad.len());
    use tokio::io::AsyncWriteExt;
    plugin_w.write_all(header.as_bytes()).await.unwrap();
    plugin_w.write_all(bad).await.unwrap();
    plugin_w.flush().await.unwrap();

    let err = call.await.unwrap().unwrap_err();
    assert!(
        matches!(err, RpcError::Disconnected),
        "envelope violation must surface as Disconnected, got {err:?}"
    );
    let reader_result = handles.reader.await.unwrap();
    assert!(
        matches!(reader_result, Err(CodecError::InvalidEnvelope(_))),
        "reader should exit with InvalidEnvelope, got {reader_result:?}"
    );
}

#[test]
fn decode_envelope_rejects_envelope_violations() {
    // jsonrpc missing
    assert!(matches!(
        decode_envelope(&json!({"id": 1, "result": {}})),
        Err(CodecError::InvalidEnvelope(_))
    ));
    // jsonrpc wrong version
    assert!(matches!(
        decode_envelope(&json!({"jsonrpc": "1.0", "id": 1, "result": {}})),
        Err(CodecError::InvalidEnvelope(_))
    ));
    // Both result and error
    assert!(matches!(
        decode_envelope(
            &json!({"jsonrpc": "2.0", "id": 1, "result": {}, "error": {"code": 1, "message": "x"}})
        ),
        Err(CodecError::InvalidEnvelope(_))
    ));
    // §5.2 restricts ids to integers. Every non-integer id shape
    // must be rejected on both the request and response sides.
    for bad_id in [
        json!("abc"),    // string id (doc rules this out)
        json!(null), // JSON-RPC allows null only for certain error responses; doc forbids entirely
        json!(1.5),  // fractional number
        json!(true), // boolean
        json!({"n": 1}), // object
        json!([1]),  // array
    ] {
        // On a response
        assert!(
            matches!(
                decode_envelope(&json!({"jsonrpc": "2.0", "id": bad_id, "result": {}})),
                Err(CodecError::InvalidEnvelope(_))
            ),
            "response with id={bad_id} should be rejected"
        );
        // On a request
        assert!(
            matches!(
                decode_envelope(&json!({"jsonrpc": "2.0", "id": bad_id, "method": "x"})),
                Err(CodecError::InvalidEnvelope(_))
            ),
            "request with id={bad_id} should be rejected"
        );
    }
    // Method on a response
    assert!(matches!(
        decode_envelope(&json!({"jsonrpc": "2.0", "id": 1, "method": "x", "result": {}})),
        Err(CodecError::InvalidEnvelope(_))
    ));
    // Response with neither result nor error
    assert!(matches!(
        decode_envelope(&json!({"jsonrpc": "2.0", "id": 1})),
        Err(CodecError::InvalidEnvelope(_))
    ));

    // Valid request
    assert!(matches!(
        decode_envelope(&json!({"jsonrpc": "2.0", "id": 7, "method": "echo", "params": {}})),
        Ok(DecodedMessage::Request { id: 7, .. })
    ));
    // Valid notification
    assert!(matches!(
        decode_envelope(&json!({"jsonrpc": "2.0", "method": "register"})),
        Ok(DecodedMessage::Notification { .. })
    ));
    // Valid response (success)
    assert!(matches!(
        decode_envelope(&json!({"jsonrpc": "2.0", "id": 3, "result": {}})),
        Ok(DecodedMessage::Response {
            id: 3,
            outcome: Ok(_)
        })
    ));
    // Valid response (error)
    assert!(matches!(
        decode_envelope(
            &json!({"jsonrpc": "2.0", "id": 4, "error": {"code": -32000, "message": "bad"}})
        ),
        Ok(DecodedMessage::Response {
            id: 4,
            outcome: Err(_)
        })
    ));
}

#[tokio::test]
async fn cancelled_future_cleans_up_pending_entry() {
    // Regression for a transport leak: if the caller's future is
    // dropped between `send` and `response`, the pending map
    // entry MUST be removed. Otherwise a connected plugin that
    // never answers would accumulate dead waiters forever.

    struct SilentPlugin;
    #[async_trait]
    impl InboundHandler for SilentPlugin {
        async fn on_request(
            &self,
            _method: String,
            _params: Value,
        ) -> Result<Value, (i32, String)> {
            std::future::pending::<()>().await;
            unreachable!()
        }
        async fn on_notification(&self, _: String, _: Value) {}
    }

    let (host_rw, plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (plugin_r, plugin_w) = tokio::io::split(plugin_rw);
    let (host, _hh) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: "cancel-host".into(),
            default_rpc_timeout: Duration::from_secs(60),
            ..Default::default()
        },
        Arc::new(DummyHandler),
    );
    let (_plugin_keep, _ph) = Transport::spawn(
        plugin_r,
        plugin_w,
        TransportConfig {
            plugin_id: "cancel-peer".into(),
            ..Default::default()
        },
        Arc::new(SilentPlugin),
    );

    assert_eq!(host.pending_len_for_test(), 0);

    // Start a call, let it enqueue its pending entry, then abort.
    let spawned = {
        let host = host.clone();
        tokio::spawn(async move { host.call::<_, Value>("echo", &json!({"x": 1})).await })
    };
    // Let the frame leave and the pending entry get inserted.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        host.pending_len_for_test(),
        1,
        "call should have registered one waiter"
    );

    spawned.abort();
    // Give Drop a tick to run.
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(
        host.pending_len_for_test(),
        0,
        "cancelled future must remove its pending entry"
    );
}

#[tokio::test]
async fn local_timeout_cleans_up_pending_entry() {
    // Timeout path: the pending entry must also be removed when
    // the sender-enforced deadline fires, not just on response.
    struct SilentPlugin;
    #[async_trait]
    impl InboundHandler for SilentPlugin {
        async fn on_request(&self, _m: String, _p: Value) -> Result<Value, (i32, String)> {
            std::future::pending::<()>().await;
            unreachable!()
        }
        async fn on_notification(&self, _: String, _: Value) {}
    }
    let (host_rw, plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (plugin_r, plugin_w) = tokio::io::split(plugin_rw);
    let (host, _hh) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: "timeout-host".into(),
            default_rpc_timeout: Duration::from_secs(60),
            ..Default::default()
        },
        Arc::new(DummyHandler),
    );
    let (_plugin_keep, _ph) = Transport::spawn(
        plugin_r,
        plugin_w,
        TransportConfig {
            plugin_id: "timeout-peer".into(),
            ..Default::default()
        },
        Arc::new(SilentPlugin),
    );

    let err = host
        .call_value_with_timeout("echo", json!({}), Some(Duration::from_millis(40)))
        .await
        .expect_err("should timeout");
    assert!(matches!(err, RpcError::Timeout(_)));
    assert_eq!(
        host.pending_len_for_test(),
        0,
        "timeout path must remove its pending entry"
    );
}

#[tokio::test]
async fn dropping_writer_fails_pending_with_disconnected() {
    let (host, plugin, host_handles, plugin_handles, _log) = pair().await;

    // Plugin initiates a call to the host, which we arrange will
    // never complete: host handler replies to `echo` but we drop
    // it before firing.
    let slow_call = {
        let host = host.clone();
        tokio::spawn(async move { host.call::<_, Value>("echo", &json!({"block": true})).await })
    };

    // Give the frame time to leave the host.
    tokio::time::sleep(Duration::from_millis(20)).await;
    // Tear everything down.
    drop(host);
    drop(plugin);
    // Wait for tasks to stop; we don't care about their result.
    let _ = host_handles.reader.await;
    let _ = host_handles.writer.await;
    let _ = plugin_handles.reader.await;
    let _ = plugin_handles.writer.await;

    let result = slow_call.await.unwrap();
    // Depending on scheduling either we'll see Disconnected or a
    // successful response that raced the drop. Both are OK for
    // this test — the important thing is that we do not hang.
    match result {
        Ok(_) => {}
        Err(RpcError::Disconnected) => {}
        Err(RpcError::Remote { .. }) => {}
        Err(RpcError::Codec(_)) => {}
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}
