//! End-to-end contract test for `preflight` against a real child
//! process. Uses a Python fake plugin at `tests/fixtures/fake_plugin.py`
//! so the test exercises:
//!
//! - stdio wiring through `SubprocessPlugin`,
//! - LSP-style `Content-Length:` framing,
//! - the initialize → describe → shutdown dance in preflight.rs.
//!
//! Gated on `python3` availability: CI images without Python skip the
//! test rather than fail.

use std::path::PathBuf;
use std::process::Command;

use garyx_channels::plugin_host::{PluginManifest, preflight};
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
    p.push("tests/fixtures/fake_plugin.py");
    p
}

fn install_manifest(dir: &TempDir, plugin_id: &str) -> PluginManifest {
    // Copy the fake plugin into the manifest directory so `entry.binary`
    // is a relative path the way a real installed plugin would have it.
    let source = fixture_path();
    let target = dir.path().join("fake_plugin.py");
    std::fs::copy(&source, &target).expect("copy fake plugin");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms).unwrap();
    }

    // Must match the Python fake plugin's describe response: schema
    // declares one required `token` field; a single `device_code`
    // auth flow; capability bits outbound+inbound only. Preflight now
    // enforces manifest/runtime agreement (§6.3a) so skew here is a
    // fixture bug, not a plugin bug.
    let manifest_path = dir.path().join("plugin.toml");
    let body = format!(
        r#"
[plugin]
id = "{plugin_id}"
version = "0.1.0"
display_name = "Fake"

[entry]
binary = "./fake_plugin.py"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = true
inbound = true
streaming = false
images = false
files = false

[runtime]
shutdown_grace_ms = 1000

[[auth_flows]]
id = "device_code"
label = "Device code"
prompt = "opens browser"

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
required = ["token"]

[schema.properties.token]
type = "string"
"#
    );
    std::fs::write(&manifest_path, body).unwrap();
    PluginManifest::load(&manifest_path).unwrap()
}

#[tokio::test]
async fn preflight_round_trips_against_real_subprocess() {
    if !python3_available() {
        eprintln!("skipping preflight_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let manifest = install_manifest(&dir, "fake-plugin");

    let summary = preflight(
        &manifest,
        "0.2.0-test",
        "/tmp/garyx-test",
        "https://example.invalid",
    )
    .await
    .expect("preflight should succeed against fake plugin");

    assert_eq!(summary.id, "fake-plugin");
    assert_eq!(summary.version, "0.1.0");
    assert_eq!(summary.protocol_versions, vec![1]);
    assert_eq!(summary.schema["type"], "object");
    assert_eq!(summary.auth_flows.len(), 1);
    assert_eq!(summary.auth_flows[0].id, "device_code");
    assert!(summary.capabilities.outbound);
    assert!(summary.capabilities.inbound);
}

/// Drop in a manifest that passes id/protocol checks but lies about
/// the account schema. Preflight must refuse to hand back a summary.
fn install_manifest_with_body(dir: &TempDir, body: &str) -> PluginManifest {
    let source = fixture_path();
    let target = dir.path().join("fake_plugin.py");
    std::fs::copy(&source, &target).expect("copy fake plugin");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms).unwrap();
    }
    let manifest_path = dir.path().join("plugin.toml");
    std::fs::write(&manifest_path, body).unwrap();
    PluginManifest::load(&manifest_path).unwrap()
}

#[tokio::test]
async fn preflight_flags_schema_mismatch() {
    if !python3_available() {
        eprintln!("skipping preflight_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    // Manifest schema differs from the runtime schema: here we drop the
    // `required` array. Preflight must catch this as SchemaMismatch.
    let body = r#"
[plugin]
id = "fake-plugin"
version = "0.1.0"
display_name = "Fake"

[entry]
binary = "./fake_plugin.py"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = true
inbound = true
streaming = false
images = false
files = false

[runtime]
shutdown_grace_ms = 1000

[[auth_flows]]
id = "device_code"
label = "Device code"
prompt = "opens browser"

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"

[schema.properties.token]
type = "string"
"#;
    let manifest = install_manifest_with_body(&dir, body);
    let err = preflight(
        &manifest,
        "0.2.0-test",
        "/tmp/garyx-test",
        "https://example.invalid",
    )
    .await
    .expect_err("schema drift should fail preflight");
    assert!(
        matches!(
            err,
            garyx_channels::plugin_host::PreflightFailure::SchemaMismatch { .. }
        ),
        "unexpected error shape: {err:?}"
    );
}

#[tokio::test]
async fn preflight_flags_auth_flow_mismatch() {
    if !python3_available() {
        eprintln!("skipping preflight_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    // Manifest advertises a second auth flow the plugin doesn't know
    // about. Runtime reports only device_code. Expect AuthFlowMismatch.
    let body = r#"
[plugin]
id = "fake-plugin"
version = "0.1.0"
display_name = "Fake"

[entry]
binary = "./fake_plugin.py"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = true
inbound = true
streaming = false
images = false
files = false

[runtime]
shutdown_grace_ms = 1000

[[auth_flows]]
id = "device_code"
label = "Device code"
prompt = "opens browser"

[[auth_flows]]
id = "password"
label = "Password"
prompt = "legacy"

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
required = ["token"]

[schema.properties.token]
type = "string"
"#;
    let manifest = install_manifest_with_body(&dir, body);
    let err = preflight(
        &manifest,
        "0.2.0-test",
        "/tmp/garyx-test",
        "https://example.invalid",
    )
    .await
    .expect_err("auth_flow drift should fail preflight");
    assert!(
        matches!(
            err,
            garyx_channels::plugin_host::PreflightFailure::AuthFlowMismatch { .. }
        ),
        "unexpected error shape: {err:?}"
    );
}

#[tokio::test]
async fn preflight_flags_capability_mismatch() {
    if !python3_available() {
        eprintln!("skipping preflight_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    // Manifest claims streaming=true but runtime reports streaming=false.
    // The manifest MUST NOT over-promise; preflight enforces that.
    let body = r#"
[plugin]
id = "fake-plugin"
version = "0.1.0"
display_name = "Fake"

[entry]
binary = "./fake_plugin.py"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = true
inbound = true
streaming = true
images = false
files = false

[runtime]
shutdown_grace_ms = 1000

[[auth_flows]]
id = "device_code"
label = "Device code"
prompt = "opens browser"

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
required = ["token"]

[schema.properties.token]
type = "string"
"#;
    let manifest = install_manifest_with_body(&dir, body);
    let err = preflight(
        &manifest,
        "0.2.0-test",
        "/tmp/garyx-test",
        "https://example.invalid",
    )
    .await
    .expect_err("capability drift should fail preflight");
    match err {
        garyx_channels::plugin_host::PreflightFailure::CapabilityMismatch {
            manifest_capability,
            manifest_value,
            runtime_value,
        } => {
            assert_eq!(manifest_capability, "streaming");
            assert!(manifest_value);
            assert!(!runtime_value);
        }
        other => panic!("unexpected error shape: {other:?}"),
    }
}

#[tokio::test]
async fn preflight_flags_plugin_id_mismatch() {
    if !python3_available() {
        eprintln!("skipping preflight_contract: python3 not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    // Manifest declares `other-id` but the Python plugin reports
    // `FAKE_PLUGIN_ID` from the environment. We deliberately point the
    // env at a different id to catch the mismatch path in preflight.rs.
    let mut manifest = install_manifest(&dir, "other-id");
    manifest
        .entry
        .env
        .insert("FAKE_PLUGIN_ID".into(), "real-id".into());
    // Rewrite plugin.toml so the manifest reload picks up the env var
    // (the struct mutation above only affects the in-memory copy; but
    // that's what `preflight` reads, so no reload needed).

    let err = preflight(
        &manifest,
        "0.2.0-test",
        "/tmp/garyx-test",
        "https://example.invalid",
    )
    .await
    .expect_err("id mismatch should fail preflight");
    assert!(
        matches!(
            err,
            garyx_channels::plugin_host::PreflightFailure::PluginIdMismatch { .. }
        ),
        "unexpected error shape: {err:?}"
    );
}
