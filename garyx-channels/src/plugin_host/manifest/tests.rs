use super::*;
use tempfile::TempDir;

fn write_manifest(body: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("plugin.toml");
    std::fs::write(&path, body).unwrap();
    (dir, path)
}

#[test]
fn parses_minimal_manifest() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = "acmechat"
version = "0.2.0"
display_name = "AcmeChat"

[entry]
binary = "./garyx-acmechat-plugin"

[capabilities]
delivery_model = "pull_explicit_ack"
"#,
    );
    let m = PluginManifest::load(&path).unwrap();
    assert_eq!(m.plugin.id, "acmechat");
    assert_eq!(
        m.capabilities.delivery_model,
        DeliveryModel::PullExplicitAck
    );
    assert!(m.capabilities.outbound);
    assert!(m.capabilities.inbound);
    assert_eq!(m.runtime.stop_grace_ms, 5000);
    assert_eq!(m.runtime.max_inflight_inbound, 32);
}

#[test]
fn runtime_overrides_clamp_above_ceiling() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = "p"
version = "1"
display_name = "P"

[entry]
binary = "./x"

[capabilities]
delivery_model = "pull_explicit_ack"

[runtime]
stop_grace_ms = 120000
"#,
    );
    let err = PluginManifest::load(&path).unwrap_err();
    assert!(matches!(
        err,
        ManifestError::StopGraceTooLarge { got: 120000, .. }
    ));
}

#[test]
fn unknown_auth_flow_keeps_going() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = "feishu-plugin"
version = "0.1.0"
display_name = "Feishu"

[entry]
binary = "./feishu-plugin"

[capabilities]
delivery_model = "push_negative_ack"

[[auth_flows]]
id = "device_code"
label = "One-click via device code"
prompt = "opens browser"
"#,
    );
    let m = PluginManifest::load(&path).unwrap();
    assert_eq!(m.auth_flows.len(), 1);
    assert_eq!(m.auth_flows[0].id, "device_code");
    assert_eq!(
        m.capabilities.delivery_model,
        DeliveryModel::PushNegativeAck
    );
}

#[test]
fn missing_binary_surfaces_later() {
    let (dir, path) = write_manifest(
        r#"
[plugin]
id = "ghost"
version = "0.0.0"
display_name = "Ghost"

[entry]
binary = "./does-not-exist"

[capabilities]
delivery_model = "push_at_most_once"
"#,
    );
    let m = PluginManifest::load(&path).unwrap();
    assert!(m.verify_binary(&path).is_err());
    assert_eq!(m.binary_path(), dir.path().join("does-not-exist"));
}

#[test]
fn rejects_empty_id() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = ""
version = "1"
display_name = "x"

[entry]
binary = "./x"

[capabilities]
delivery_model = "pull_explicit_ack"
"#,
    );
    let err = PluginManifest::load(&path).unwrap_err();
    assert!(matches!(err, ManifestError::EmptyId { .. }));
}

#[test]
fn schema_roundtrips_untouched() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = "schema-test"
version = "1"
display_name = "s"

[entry]
binary = "./s"

[capabilities]
delivery_model = "pull_explicit_ack"

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
required = ["token"]

[schema.properties.token]
type = "string"
format = "password"
"#,
    );
    let m = PluginManifest::load(&path).unwrap();
    assert_eq!(m.schema["type"], "object");
    assert_eq!(m.schema["properties"]["token"]["format"], "password");
}
