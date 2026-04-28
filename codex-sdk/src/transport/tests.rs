use super::*;
use serde_json::json;

async fn make_pending_with_id(id: u64) -> (Arc<Mutex<PendingMap>>, oneshot::Receiver<Value>) {
    let (tx, rx) = oneshot::channel();
    let pending = Arc::new(Mutex::new(HashMap::new()));
    pending.lock().await.insert(id, tx);
    (pending, rx)
}

fn make_handler() -> Arc<Mutex<Option<ServerRequestHandler>>> {
    Arc::new(Mutex::new(None))
}

fn make_fatal() -> Arc<Mutex<Option<CodexError>>> {
    Arc::new(Mutex::new(None))
}

#[tokio::test]
async fn test_dispatch_response_resolves_pending() {
    let (pending, rx) = make_pending_with_id(42).await;
    let (notification_tx, _) = broadcast::channel(16);
    let stdin_writer = Arc::new(Mutex::new(None));
    let handler = make_handler();
    let fatal = make_fatal();

    let payload = json!({
        "id": 42,
        "result": { "status": "ok" }
    });

    dispatch_message(
        &payload,
        &pending,
        &notification_tx,
        &stdin_writer,
        &handler,
        &fatal,
    )
    .await;

    let response = rx.await.unwrap();
    assert_eq!(response["result"]["status"], "ok");
    assert!(pending.lock().await.is_empty());
}

#[tokio::test]
async fn test_dispatch_error_response_resolves_pending() {
    let (pending, rx) = make_pending_with_id(7).await;
    let (notification_tx, _) = broadcast::channel(16);
    let stdin_writer = Arc::new(Mutex::new(None));
    let handler = make_handler();
    let fatal = make_fatal();

    let payload = json!({
        "id": 7,
        "error": { "code": -32001, "message": "overloaded" }
    });

    dispatch_message(
        &payload,
        &pending,
        &notification_tx,
        &stdin_writer,
        &handler,
        &fatal,
    )
    .await;

    let response = rx.await.unwrap();
    assert_eq!(response["error"]["code"], -32001);
}

#[tokio::test]
async fn test_dispatch_notification_broadcasts() {
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let (notification_tx, mut rx) = broadcast::channel(16);
    let stdin_writer = Arc::new(Mutex::new(None));
    let handler = make_handler();
    let fatal = make_fatal();

    let payload = json!({
        "method": "item/agentMessage/delta",
        "params": { "threadId": "t1", "turnId": "u1", "delta": "hello " }
    });

    dispatch_message(
        &payload,
        &pending,
        &notification_tx,
        &stdin_writer,
        &handler,
        &fatal,
    )
    .await;

    let notif = rx.recv().await.unwrap();
    assert_eq!(notif.method, "item/agentMessage/delta");
    assert_eq!(notif.params["delta"], "hello ");
}

#[tokio::test]
async fn test_dispatch_notification_no_params() {
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let (notification_tx, mut rx) = broadcast::channel(16);
    let stdin_writer = Arc::new(Mutex::new(None));
    let handler = make_handler();
    let fatal = make_fatal();

    let payload = json!({ "method": "heartbeat" });

    dispatch_message(
        &payload,
        &pending,
        &notification_tx,
        &stdin_writer,
        &handler,
        &fatal,
    )
    .await;

    let notif = rx.recv().await.unwrap();
    assert_eq!(notif.method, "heartbeat");
    assert!(notif.params.is_object());
}

#[tokio::test]
async fn test_dispatch_unknown_id_ignored() {
    let (pending, _rx) = make_pending_with_id(1).await;
    let (notification_tx, _) = broadcast::channel(16);
    let stdin_writer = Arc::new(Mutex::new(None));
    let handler = make_handler();
    let fatal = make_fatal();

    let payload = json!({ "id": 999, "result": {} });

    dispatch_message(
        &payload,
        &pending,
        &notification_tx,
        &stdin_writer,
        &handler,
        &fatal,
    )
    .await;

    assert!(pending.lock().await.contains_key(&1));
}

#[test]
fn test_server_request_classification() {
    let payload = json!({
        "id": 10,
        "method": "item/commandExecution/requestApproval",
        "params": { "command": "ls" }
    });

    let has_id = payload.get("id").is_some();
    let has_method = payload.get("method").is_some();
    let has_result = payload.get("result").is_some() || payload.get("error").is_some();

    assert!(
        has_id && has_method && !has_result,
        "should classify as server request"
    );
}

#[test]
fn test_message_classification() {
    let response = json!({"id": 1, "result": {}});
    assert!(response.get("id").is_some());
    assert!(response.get("result").is_some());
    assert!(response.get("method").is_none());

    let notification = json!({"method": "event", "params": {}});
    assert!(notification.get("method").is_some());
    assert!(notification.get("id").is_none());

    let server_req = json!({"id": 5, "method": "test/req", "params": {}});
    assert!(server_req.get("method").is_some());
    assert!(server_req.get("id").is_some());
    assert!(server_req.get("result").is_none() && server_req.get("error").is_none());
}

#[test]
fn test_transport_new_defaults() {
    let t = CodexTransport::new("/usr/bin/codex", &["--flag"]);
    assert!(!t.is_ready());
    assert!(!t.is_closed());
}

#[test]
fn test_transport_builder_methods() {
    let t = CodexTransport::new("codex", &[])
        .with_startup_timeout(Duration::from_secs(60))
        .with_request_timeout(Duration::from_secs(30));
    assert_eq!(t.startup_timeout, Duration::from_secs(60));
    assert_eq!(t.request_timeout, Duration::from_secs(30));
}

#[tokio::test]
async fn test_request_when_closed_returns_error() {
    let t = CodexTransport::new("codex", &[]);
    t.closed.store(true, Ordering::SeqCst);

    let err = t.send_request("test", None).await.unwrap_err();
    assert!(matches!(err, CodexError::AlreadyClosed));
}

#[tokio::test]
async fn test_notification_when_not_initialized_returns_error() {
    let t = CodexTransport::new("codex", &[]);
    let err = t.send_notification("test", None).await.unwrap_err();
    assert!(matches!(err, CodexError::NotInitialized));
}

#[tokio::test]
async fn test_start_when_closed() {
    let t = CodexTransport::new("codex", &[]);
    t.closed.store(true, Ordering::SeqCst);

    let err = t.start(json!({})).await.unwrap_err();
    assert!(matches!(err, CodexError::AlreadyClosed));
}

#[tokio::test]
async fn test_shutdown_idempotent() {
    let t = CodexTransport::new("codex", &[]);
    t.shutdown().await;
    assert!(t.is_closed());
    t.shutdown().await;
    assert!(t.is_closed());
}

#[tokio::test]
async fn test_subscribe_notifications() {
    let t = CodexTransport::new("codex", &[]);
    let mut rx = t.subscribe_notifications();

    let _ = t.notification_tx.send(JsonRpcNotification {
        method: "test/event".to_owned(),
        params: json!({"key": "value"}),
    });

    let notif = rx.recv().await.unwrap();
    assert_eq!(notif.method, "test/event");
    assert_eq!(notif.params["key"], "value");
}

#[tokio::test]
async fn test_set_fatal_error_broadcasts() {
    let fatal = make_fatal();
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let (notification_tx, mut rx) = broadcast::channel(16);

    let err = CodexError::Fatal("test error".to_owned());
    set_fatal_error(&fatal, &pending, &notification_tx, err).await;

    assert!(fatal.lock().await.is_some());

    let notif = rx.recv().await.unwrap();
    assert_eq!(notif.method, "transport/fatal");
    assert!(
        notif.params["error"]
            .as_str()
            .unwrap()
            .contains("test error")
    );
}

#[tokio::test]
async fn test_set_fatal_error_fails_pending() {
    let fatal = make_fatal();
    let (pending, rx) = make_pending_with_id(1).await;
    let (notification_tx, _) = broadcast::channel(16);

    let err = CodexError::Fatal("boom".to_owned());
    set_fatal_error(&fatal, &pending, &notification_tx, err).await;

    let response = rx.await.unwrap();
    assert!(response.get("error").is_some());
    assert!(pending.lock().await.is_empty());
}

#[tokio::test]
async fn test_set_fatal_error_idempotent() {
    let fatal = make_fatal();
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let (notification_tx, _) = broadcast::channel(16);

    let err1 = CodexError::Fatal("first".to_owned());
    let err2 = CodexError::Fatal("second".to_owned());

    set_fatal_error(&fatal, &pending, &notification_tx, err1).await;
    set_fatal_error(&fatal, &pending, &notification_tx, err2).await;

    let guard = fatal.lock().await;
    assert!(guard.as_ref().unwrap().to_string().contains("first"));
}

#[tokio::test]
async fn test_reader_loop_eof_sets_fatal_error_and_fails_pending() {
    let (pending, rx) = make_pending_with_id(11).await;
    let (notification_tx, mut notification_rx) = broadcast::channel(16);
    let handler = make_handler();
    let fatal = make_fatal();

    let mut child = Command::new("sh")
        .arg("-c")
        .arg("exit 0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn shell");

    let stdout = child.stdout.take().unwrap();
    let pid = child.id().unwrap_or(0);
    let stdin_writer: Arc<Mutex<Option<tokio::process::ChildStdin>>> = Arc::new(Mutex::new(None));

    let reader_handle = tokio::spawn(reader_loop(
        pid,
        stdout,
        pending.clone(),
        notification_tx.clone(),
        stdin_writer,
        handler,
        fatal.clone(),
    ));

    let response = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .expect("timed out waiting for pending response")
        .expect("pending channel dropped");
    assert_eq!(response["error"]["code"], -32000);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("codex stdout closed unexpectedly")
    );

    let notif = tokio::time::timeout(Duration::from_secs(2), notification_rx.recv())
        .await
        .expect("timed out waiting for fatal notification")
        .expect("notification channel dropped");
    assert_eq!(notif.method, "transport/fatal");

    let fatal_guard = fatal.lock().await;
    assert!(
        fatal_guard
            .as_ref()
            .unwrap()
            .to_string()
            .contains("codex stdout closed unexpectedly")
    );
    drop(fatal_guard);

    let _ = reader_handle.await;
    let _ = child.wait().await;
}

#[tokio::test]
async fn test_start_failure_cleans_up_process_state() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let script_path = std::env::temp_dir().join(format!(
        "codex-sdk-exit-immediately-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    fs::write(&script_path, "#!/bin/sh\nexit 0\n").expect("failed to write mock codex script");
    let mut perms = fs::metadata(&script_path)
        .expect("failed to stat mock codex script")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).expect("failed to chmod mock codex script");

    let transport = CodexTransport::new(script_path.to_str().unwrap(), &[])
        .with_startup_timeout(Duration::from_secs(1))
        .with_request_timeout(Duration::from_secs(1));

    let err = transport
        .start(json!({"clientInfo": {"name": "test"}}))
        .await;
    assert!(err.is_err());
    assert!(!transport.is_ready());
    assert!(!transport.is_closed());
    assert!(transport.child.lock().await.is_none());
    assert!(transport.stdin_writer.lock().await.is_none());
    assert!(transport.reader_task.lock().await.is_none());
    assert!(transport.stderr_task.lock().await.is_none());
    assert!(transport.pending.lock().await.is_empty());
    assert!(transport.fatal_error().await.is_none());

    let _ = fs::remove_file(&script_path);
}

#[test]
fn test_error_display() {
    let cases: Vec<(CodexError, &str)> = vec![
        (
            CodexError::ConnectionFailed("boom".into()),
            "connection failed: boom",
        ),
        (
            CodexError::ProcessDied(1),
            "codex process died unexpectedly (exit code: 1)",
        ),
        (
            CodexError::RequestTimeout(30),
            "request timed out after 30s",
        ),
        (
            CodexError::RpcError {
                code: -32001,
                message: "overloaded".into(),
                data: None,
            },
            "RPC error -32001: overloaded",
        ),
        (CodexError::AlreadyClosed, "transport already closed"),
        (CodexError::Fatal("oops".into()), "fatal: oops"),
        (CodexError::NotInitialized, "client not initialized"),
    ];
    for (err, expected) in cases {
        assert_eq!(err.to_string(), expected);
    }
}

#[test]
fn test_rand_jitter() {
    let j = rand_jitter();
    assert!(j < 100);
}

/// Integration test: spawn `cat` as a mock process and verify round-trip.
#[tokio::test]
async fn test_roundtrip_with_cat_process() {
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let (notification_tx, mut notification_rx) = broadcast::channel(16);
    let handler = make_handler();
    let fatal = make_fatal();

    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn cat");

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let pid = child.id().unwrap_or(0);

    let stdin_writer: Arc<Mutex<Option<tokio::process::ChildStdin>>> =
        Arc::new(Mutex::new(Some(stdin)));

    let reader_handle = tokio::spawn(reader_loop(
        pid,
        stdout,
        pending.clone(),
        notification_tx.clone(),
        stdin_writer.clone(),
        handler,
        fatal,
    ));

    // Write a notification; cat echoes it back and reader broadcasts it.
    {
        let mut guard = stdin_writer.lock().await;
        let s = guard.as_mut().unwrap();
        let msg = json!({"method": "test/ping", "params": {"ts": 123}});
        let mut encoded = serde_json::to_string(&msg).unwrap();
        encoded.push('\n');
        s.write_all(encoded.as_bytes()).await.unwrap();
        s.flush().await.unwrap();
    }

    let notif = tokio::time::timeout(Duration::from_secs(2), notification_rx.recv())
        .await
        .expect("timed out")
        .expect("recv error");
    assert_eq!(notif.method, "test/ping");
    assert_eq!(notif.params["ts"], 123);

    // Write a response
    let (tx, rx) = oneshot::channel();
    pending.lock().await.insert(99, tx);

    {
        let mut guard = stdin_writer.lock().await;
        let s = guard.as_mut().unwrap();
        let msg = json!({"id": 99, "result": {"ok": true}});
        let mut encoded = serde_json::to_string(&msg).unwrap();
        encoded.push('\n');
        s.write_all(encoded.as_bytes()).await.unwrap();
        s.flush().await.unwrap();
    }

    let result = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .expect("timed out")
        .expect("channel error");
    assert_eq!(result["result"]["ok"], true);

    // Cleanup
    drop(stdin_writer.lock().await.take());
    reader_handle.abort();
    let _ = child.kill().await;
}
