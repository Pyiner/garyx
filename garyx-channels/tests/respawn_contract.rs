//! End-to-end contract test for §6.4 / §9.4 respawn.
//!
//! Drives `ChannelPluginManager::register_subprocess_plugin` and
//! `respawn_plugin` against the Python `fake_lifecycle_plugin.py`
//! fixture so the test exercises:
//!
//! - initialize + start handshake (with `dry_run=false`),
//! - `dispatch_outbound` round-trip through the
//!   `SwappableDispatcher`,
//! - §9.4 quiesce + hot-swap (the OLD child is torn down, outbound
//!   traffic flips to the NEW child before the OLD is reaped),
//! - account-mutation propagation on respawn.
//!
//! Gated on `python3` availability so CI images without Python skip
//! rather than fail.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use garyx_channels::OutboundMessage;
use garyx_channels::channel_trait::ChannelError;
use garyx_channels::dispatcher::{ChannelDispatcher, ChannelDispatcherImpl, SwappableDispatcher};
use garyx_channels::plugin::{ChannelPluginManager, PluginState, SubprocessPluginError};
use garyx_channels::plugin_host::{
    AccountDescriptor, HostContext, InboundHandler, PluginErrorCode, PluginManifest, RpcError,
    SpawnOptions,
};
use garyx_models::ChannelOutboundContent;
use serde_json::{Value, json};
use tempfile::TempDir;

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn fixture_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/fake_lifecycle_plugin.py");
    p
}

/// A host-side handler that ignores every inbound — the fixture
/// doesn't send any during these tests.
struct NoopHandler;

#[async_trait]
impl InboundHandler for NoopHandler {
    async fn on_request(&self, _method: String, _params: Value) -> Result<Value, (i32, String)> {
        Err((-32601, "test host accepts no inbound requests".into()))
    }

    async fn on_notification(&self, _method: String, _params: Value) {}
}

fn install_manifest(dir: &TempDir, plugin_id: &str) -> PluginManifest {
    let source = fixture_path();
    let target = dir.path().join("fake_lifecycle_plugin.py");
    std::fs::copy(&source, &target).expect("copy fake lifecycle plugin");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms).unwrap();
    }

    let manifest_path = dir.path().join("plugin.toml");
    let body = format!(
        r#"
[plugin]
id = "{plugin_id}"
version = "0.1.0"
display_name = "Fake lifecycle"

[entry]
binary = "./fake_lifecycle_plugin.py"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = true
inbound = true
streaming = false
images = false
files = false

[runtime]
stop_grace_ms = 1000
shutdown_grace_ms = 1000

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
"#
    );
    std::fs::write(&manifest_path, body).unwrap();
    PluginManifest::load(&manifest_path).unwrap()
}

fn host_ctx() -> HostContext {
    HostContext {
        version: "0.2.0-test".into(),
        public_url: "https://example.invalid".into(),
        data_dir: "/tmp/garyx-test".into(),
        locale: Some("en".into()),
    }
}

fn dispatch_request(channel: &str, account: &str, chat: &str) -> OutboundMessage {
    OutboundMessage {
        channel: channel.to_owned(),
        account_id: account.to_owned(),
        chat_id: chat.to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: chat.to_owned(),
        content: ChannelOutboundContent::text("hi"),
        reply_to: None,
        thread_id: None,
    }
}

#[tokio::test]
async fn register_then_respawn_hot_swaps_the_subprocess() {
    if !python3_available() {
        eprintln!("skipping respawn_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let manifest = install_manifest(&dir, "fake-lifecycle-plugin");

    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));

    let mut manager = ChannelPluginManager::new();
    manager.attach_dispatcher(swap.clone());

    let accounts = vec![AccountDescriptor {
        id: "acct-1".into(),
        enabled: true,
        config: json!({"token": "x"}),
    }];

    manager
        .register_subprocess_plugin(
            manifest.clone(),
            SpawnOptions::default(),
            host_ctx(),
            accounts.clone(),
            Arc::new(NoopHandler),
        )
        .await
        .expect("register should succeed against live fixture");

    // Catalog shape check: the desktop UI uses this endpoint to
    // render schema-driven account forms, so we pin the contract
    // here rather than a separate HTTP-level test (no gateway needed).
    let catalog = manager.subprocess_plugin_catalog();
    assert_eq!(catalog.len(), 1, "one plugin registered");
    let entry = &catalog[0];
    assert_eq!(entry.id, "fake-lifecycle-plugin");
    assert_eq!(entry.state, PluginState::Running);
    assert_eq!(entry.accounts.len(), 1);
    assert_eq!(entry.accounts[0].id, "acct-1");
    // The synthesized manifest we installed only has a type:object
    // schema, but the catalog entry should carry it verbatim.
    assert_eq!(entry.schema["type"], "object");
    // Serialises cleanly — if this regresses the desktop fetch breaks.
    let as_json = serde_json::to_value(entry).expect("catalog entry serialisation");
    assert_eq!(as_json["id"], "fake-lifecycle-plugin");
    assert!(
        as_json["schema"].is_object(),
        "schema must serialise as an object"
    );

    // Round-trip through the dispatcher so we observe OLD's label.
    let old_reply = swap
        .send_message(dispatch_request(
            "fake-lifecycle-plugin",
            "acct-1",
            "chat-A",
        ))
        .await
        .expect("dispatch to OLD child");
    assert_eq!(old_reply.message_ids.len(), 1);
    let old_label = old_reply.message_ids[0]
        .split_once(':')
        .expect("label:account:chat shape")
        .0
        .to_owned();
    assert!(
        old_label.starts_with("pid-"),
        "fixture should emit a pid-scoped label, got {old_label}"
    );

    // Respawn with a different account set.
    let new_accounts = vec![AccountDescriptor {
        id: "acct-2".into(),
        enabled: true,
        config: json!({"token": "y"}),
    }];

    manager
        .respawn_plugin("fake-lifecycle-plugin", Some(new_accounts.clone()))
        .await
        .expect("respawn should succeed");

    // The dispatcher now routes through the NEW child. The label is
    // PID-scoped in the fixture so OLD and NEW differ — the test
    // doesn't need to assume a specific new label, just that it
    // changed.
    let new_reply = swap
        .send_message(dispatch_request(
            "fake-lifecycle-plugin",
            "acct-2",
            "chat-A",
        ))
        .await
        .expect("dispatch to NEW child");
    assert_eq!(new_reply.message_ids.len(), 1);
    let new_label = new_reply.message_ids[0]
        .split_once(':')
        .expect("label:account:chat shape")
        .0
        .to_owned();
    assert_ne!(
        old_label, new_label,
        "respawn must swap in a fresh subprocess; got same label {old_label}"
    );
    assert!(
        new_label.starts_with("pid-"),
        "NEW fixture should also emit a pid-scoped label, got {new_label}"
    );

    // Sanity: channel entry still present in available_channels.
    let channels = swap.available_channels();
    assert!(
        channels
            .iter()
            .any(|c| c.channel == "fake-lifecycle-plugin"),
        "plugin must still be listed after respawn: {channels:?}"
    );
}

#[tokio::test]
async fn respawn_unknown_plugin_errors() {
    if !python3_available() {
        eprintln!("skipping respawn_contract: python3 not available");
        return;
    }
    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
    let mut manager = ChannelPluginManager::new();
    manager.attach_dispatcher(swap);

    let err = manager
        .respawn_plugin("does-not-exist", None)
        .await
        .expect_err("unknown plugin should error");
    match err {
        SubprocessPluginError::UnknownPlugin(id) => assert_eq!(id, "does-not-exist"),
        other => panic!("expected UnknownPlugin, got {other:?}"),
    }
}

#[tokio::test]
async fn register_without_dispatcher_errors() {
    if !python3_available() {
        eprintln!("skipping respawn_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let manifest = install_manifest(&dir, "fake-lifecycle-plugin");

    let mut manager = ChannelPluginManager::new();
    // Intentionally skip attach_dispatcher.
    let err = manager
        .register_subprocess_plugin(
            manifest,
            SpawnOptions::default(),
            host_ctx(),
            vec![],
            Arc::new(NoopHandler),
        )
        .await
        .expect_err("register without dispatcher should error");
    assert!(matches!(err, SubprocessPluginError::DispatcherNotAttached));
}

/// Register twice against the same id: the second attempt must fail
/// cleanly and leave the registry in a state that accepts a later
/// retry (the first child is still live, second never existed).
#[tokio::test]
async fn duplicate_id_registration_errors() {
    if !python3_available() {
        eprintln!("skipping respawn_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let manifest = install_manifest(&dir, "fake-lifecycle-plugin");
    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
    let mut manager = ChannelPluginManager::new();
    manager.attach_dispatcher(swap);

    manager
        .register_subprocess_plugin(
            manifest.clone(),
            SpawnOptions::default(),
            host_ctx(),
            vec![],
            Arc::new(NoopHandler),
        )
        .await
        .expect("first register");

    let err = manager
        .register_subprocess_plugin(
            manifest,
            SpawnOptions::default(),
            host_ctx(),
            vec![],
            Arc::new(NoopHandler),
        )
        .await
        .expect_err("second register under same id must fail");
    assert!(
        matches!(err, SubprocessPluginError::Registry(_)),
        "duplicate id should surface a Registry error, got {err:?}"
    );
}

/// A plugin that replies to `initialize` with `ConfigRejected` maps to
/// [`SubprocessPluginError::InitializeRejected`] — *not* `LifecycleRpc`.
/// The registry claim must be released so a corrected config can
/// re-register under the same id.
#[tokio::test]
async fn initialize_config_rejected_is_dedicated_variant() {
    if !python3_available() {
        eprintln!("skipping respawn_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let mut manifest = install_manifest(&dir, "fake-lifecycle-plugin");
    manifest.entry.env.insert(
        "FAKE_FAIL_INIT_CODE".into(),
        PluginErrorCode::ConfigRejected.as_i32().to_string(),
    );

    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
    let mut manager = ChannelPluginManager::new();
    manager.attach_dispatcher(swap);

    let err = manager
        .register_subprocess_plugin(
            manifest.clone(),
            SpawnOptions::default(),
            host_ctx(),
            vec![],
            Arc::new(NoopHandler),
        )
        .await
        .expect_err("config-rejected initialize should surface");
    match err {
        SubprocessPluginError::InitializeRejected { plugin_id, message } => {
            assert_eq!(plugin_id, "fake-lifecycle-plugin");
            assert!(
                message.contains("forced by test"),
                "must preserve plugin message: {message}"
            );
        }
        other => panic!("expected InitializeRejected, got {other:?}"),
    }

    // Registry claim must be released: a retry with a corrected config
    // (no failure env var) must not trip DuplicateId.
    let retry_manifest = install_manifest(&dir, "fake-lifecycle-plugin");
    manager
        .register_subprocess_plugin(
            retry_manifest,
            SpawnOptions::default(),
            host_ctx(),
            vec![],
            Arc::new(NoopHandler),
        )
        .await
        .expect("retry should succeed after config-rejected cleanup");
}

/// A non-`ConfigRejected` initialize error (e.g. `MethodNotFound`,
/// which indicates a protocol-mismatched plugin) surfaces as
/// `LifecycleRpc` so the caller can see the raw code for debugging.
#[tokio::test]
async fn initialize_protocol_error_is_lifecycle_rpc() {
    if !python3_available() {
        eprintln!("skipping respawn_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let mut manifest = install_manifest(&dir, "fake-lifecycle-plugin");
    manifest.entry.env.insert(
        "FAKE_FAIL_INIT_CODE".into(),
        PluginErrorCode::MethodNotFound.as_i32().to_string(),
    );

    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
    let mut manager = ChannelPluginManager::new();
    manager.attach_dispatcher(swap);

    let err = manager
        .register_subprocess_plugin(
            manifest,
            SpawnOptions::default(),
            host_ctx(),
            vec![],
            Arc::new(NoopHandler),
        )
        .await
        .expect_err("method-not-found initialize should surface");
    match err {
        SubprocessPluginError::LifecycleRpc {
            plugin_id,
            method,
            source,
        } => {
            assert_eq!(plugin_id, "fake-lifecycle-plugin");
            assert_eq!(method, "initialize");
            match source {
                RpcError::Remote { code, .. } => {
                    assert_eq!(code, PluginErrorCode::MethodNotFound.as_i32());
                }
                other => panic!("expected Remote variant, got {other:?}"),
            }
        }
        other => panic!("expected LifecycleRpc, got {other:?}"),
    }
}

/// Respawn while OLD is mid-dispatch: `HANG_DISPATCH=1` makes the
/// fixture park forever on `dispatch_outbound`. The in-flight caller
/// must receive a `ChannelError::Connection("... outbound aborted")`
/// when `respawn_plugin` hits `stop_grace_ms` expiry, per §9.4.
#[tokio::test]
async fn respawn_aborts_straggler_dispatch() {
    if !python3_available() {
        eprintln!("skipping respawn_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    // Short stop_grace so the test doesn't take the default 5s.
    let manifest_path = dir.path().join("plugin.toml");
    let source = fixture_path();
    let target = dir.path().join("fake_lifecycle_plugin.py");
    std::fs::copy(&source, &target).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms).unwrap();
    }
    std::fs::write(
        &manifest_path,
        r#"
[plugin]
id = "straggler-plugin"
version = "0.1.0"
display_name = "straggler"

[entry]
binary = "./fake_lifecycle_plugin.py"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = true
inbound = true
streaming = false
images = false
files = false

[runtime]
stop_grace_ms = 250
shutdown_grace_ms = 500

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
"#,
    )
    .unwrap();
    let mut manifest = PluginManifest::load(&manifest_path).unwrap();
    manifest
        .entry
        .env
        .insert("FAKE_HANG_DISPATCH".into(), "1".into());

    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
    let mut manager = ChannelPluginManager::new();
    manager.attach_dispatcher(swap.clone());

    manager
        .register_subprocess_plugin(
            manifest.clone(),
            SpawnOptions::default(),
            host_ctx(),
            vec![],
            Arc::new(NoopHandler),
        )
        .await
        .expect("register");

    // Fire a dispatch that will hang on the OLD child.
    let swap_for_fut = swap.clone();
    let in_flight = tokio::spawn(async move {
        swap_for_fut
            .send_message(dispatch_request("straggler-plugin", "acct", "chat-X"))
            .await
    });

    // Give the dispatch time to enter the child so `pending_count()`
    // observes it.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Drop the HANG_DISPATCH env on the NEW incarnation by mutating
    // manifest before respawn. We reuse the same accounts via None.
    // NOTE: manager already stashed the HANG version in the entry; to
    // let the NEW child serve normally we need to re-register the
    // entry's env. Simplest: the NEW child also hangs but the
    // straggler abort still must fire. That's what we assert on.
    manager
        .respawn_plugin("straggler-plugin", None)
        .await
        .expect("respawn must succeed even with a hung OLD dispatch");

    let result = in_flight.await.expect("task join");
    match result {
        Err(ChannelError::Connection(msg)) => {
            // §9.4 normative wording. Exact match — any drift here
            // breaks callers that key on the message.
            assert_eq!(
                msg, "plugin straggler-plugin respawning; outbound aborted",
                "straggler must receive §9.4 exact abort wording"
            );
        }
        other => {
            panic!("straggler dispatch must fail with ChannelError::Connection; got {other:?}")
        }
    }
}

/// §9.4 respawn-failure invariant: when the NEW child fails to come
/// up, OLD must remain the live sender in the dispatcher and the
/// entry state must not flip to `Error`. Drives this by arming the
/// lifecycle fixture with `FAKE_FAIL_INIT_IF_FILE=<path>` and touching
/// the file between `register_subprocess_plugin` and `respawn_plugin`.
/// OLD came up before the file existed and stays healthy; NEW reads
/// the file on startup and refuses to initialize.
#[tokio::test]
async fn respawn_with_failing_new_preserves_old() {
    if !python3_available() {
        eprintln!("skipping respawn_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let trigger = dir.path().join("fail-after-respawn");
    let mut manifest = install_manifest(&dir, "resilient-plugin");
    manifest.entry.env.insert(
        "FAKE_FAIL_INIT_IF_FILE".into(),
        trigger.to_string_lossy().into_owned(),
    );

    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
    let mut manager = ChannelPluginManager::new();
    manager.attach_dispatcher(swap.clone());

    manager
        .register_subprocess_plugin(
            manifest,
            SpawnOptions::default(),
            host_ctx(),
            vec![],
            Arc::new(NoopHandler),
        )
        .await
        .expect("initial register — trigger file does not yet exist");

    // Baseline dispatch proves OLD is serving. The label is
    // pid-scoped so we can assert the SAME child is still serving
    // after the failed respawn.
    let pre_reply = swap
        .send_message(dispatch_request("resilient-plugin", "a", "c"))
        .await
        .expect("pre-respawn dispatch");
    let pre_label = pre_reply.message_ids[0].clone();

    // Arm the failure. Respawn will spawn a NEW child that reads the
    // file at startup and refuses to initialize.
    std::fs::write(&trigger, b"fail").unwrap();

    let err = manager
        .respawn_plugin("resilient-plugin", None)
        .await
        .expect_err("respawn must fail because NEW cannot initialize");
    assert!(
        matches!(err, SubprocessPluginError::InitializeRejected { .. }),
        "NEW must surface InitializeRejected, got {err:?}"
    );

    // OLD must still be the live sender. Same pid-scoped label means
    // the dispatcher was never swapped away from OLD.
    let post_reply = swap
        .send_message(dispatch_request("resilient-plugin", "a", "c"))
        .await
        .expect("OLD must still serve after respawn failure");
    assert_eq!(
        pre_label, post_reply.message_ids[0],
        "dispatcher must still point at OLD when respawn's NEW fails"
    );

    // Manager-observable state must also reflect that OLD is still
    // the live plugin: the entry state should stay `Running`, NOT
    // flip to `Error`. A misrepresented state would mislead
    // `garyx doctor` / UI callers that key on this to decide whether
    // to prompt for reconfiguration.
    let status = manager
        .statuses()
        .into_iter()
        .find(|s| s.metadata.id == "resilient-plugin")
        .expect("status must still be present");
    assert_eq!(
        status.state,
        PluginState::Running,
        "entry state must stay Running when respawn's NEW fails"
    );
    assert!(
        status.last_error.is_none(),
        "last_error must not be populated when OLD keeps serving; got {:?}",
        status.last_error
    );
}

/// Plugin-supplied brand icon: install drops `icon.svg` next to the
/// binary, the manifest references it, and `subprocess_plugin_catalog()`
/// bakes the file into a data URL so the desktop UI can bind it
/// directly to `<img src={...}>` without a second round-trip.
#[tokio::test]
async fn plugin_icon_flows_through_catalog_as_data_url() {
    if !python3_available() {
        eprintln!("skipping respawn_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    // Stage the fixture binary.
    let source = fixture_path();
    let target = dir.path().join("fake_lifecycle_plugin.py");
    std::fs::copy(&source, &target).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms).unwrap();
    }
    // Drop an icon file alongside it.
    let icon_bytes = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"/>";
    std::fs::write(dir.path().join("icon.svg"), icon_bytes).unwrap();

    // Write a manifest that references the icon.
    let manifest_path = dir.path().join("plugin.toml");
    std::fs::write(
        &manifest_path,
        r#"
[plugin]
id = "iconic"
version = "0.1.0"
display_name = "Iconic"
icon = "./icon.svg"

[entry]
binary = "./fake_lifecycle_plugin.py"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = true
inbound = true
streaming = false
images = false
files = false

[runtime]
stop_grace_ms = 500
shutdown_grace_ms = 500

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
"#,
    )
    .unwrap();
    let manifest = PluginManifest::load(&manifest_path).unwrap();

    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
    let mut manager = ChannelPluginManager::new();
    manager.attach_dispatcher(swap);
    manager
        .register_subprocess_plugin(
            manifest,
            SpawnOptions::default(),
            host_ctx(),
            vec![],
            Arc::new(NoopHandler),
        )
        .await
        .expect("register");

    let catalog = manager.subprocess_plugin_catalog();
    let entry = catalog.iter().find(|e| e.id == "iconic").expect("entry");
    let data_url = entry
        .icon_data_url
        .as_deref()
        .expect("icon_data_url must be populated when plugin ships an icon");
    assert!(
        data_url.starts_with("data:image/svg+xml;base64,"),
        "expected SVG data URL, got {data_url}"
    );
    // Round-trip the payload to make sure the bytes match what we
    // dropped on disk — otherwise a subtle encoding bug (double
    // base64, extra newlines) would silently corrupt icons.
    use base64::Engine as _;
    let payload = data_url.trim_start_matches("data:image/svg+xml;base64,");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .expect("decode");
    assert_eq!(decoded, icon_bytes);
}
