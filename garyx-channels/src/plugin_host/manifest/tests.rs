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
    // Default is opt-out: plugins must explicitly assert respawn
    // safety before the host's auto-updater will hot-replace them.
    assert!(!m.capabilities.survives_respawn);
    assert_eq!(m.runtime.stop_grace_ms, 5000);
    assert_eq!(m.runtime.max_inflight_inbound, 32);
}

#[test]
fn parses_survives_respawn_opt_in() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = "stateful"
version = "0.1.0"
display_name = "Stateful"

[entry]
binary = "./stateful"

[capabilities]
delivery_model = "pull_explicit_ack"
survives_respawn = true
"#,
    );
    let m = PluginManifest::load(&path).unwrap();
    assert!(m.capabilities.survives_respawn);
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

#[test]
fn parses_update_block() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = "demo"
version = "0.1.0"
display_name = "Demo"

[entry]
binary = "./garyx-plugin-demo"

[capabilities]
delivery_model = "pull_explicit_ack"

[update]
manifest_url = "https://example.test/plugins/{id}/latest.json"
url_template = "https://example.test/plugins/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz"
checksum_url_template = "{url}.sha256"
binary_in_archive = "{id}/garyx-plugin-{id}"
"#,
    );
    let manifest = PluginManifest::load(&path).expect("should parse update block");
    let update = manifest.update.expect("update present");
    assert_eq!(
        update.manifest_url.as_deref(),
        Some("https://example.test/plugins/{id}/latest.json"),
    );
    assert_eq!(
        update.url_template,
        "https://example.test/plugins/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz",
    );
    assert_eq!(update.checksum_url_template.as_deref(), Some("{url}.sha256"));
    assert_eq!(
        update.binary_in_archive.as_deref(),
        Some("{id}/garyx-plugin-{id}"),
    );
}

#[test]
fn update_block_absent_yields_none() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = "demo"
version = "0.1.0"
display_name = "Demo"

[entry]
binary = "./garyx-plugin-demo"

[capabilities]
delivery_model = "pull_explicit_ack"
"#,
    );
    let manifest = PluginManifest::load(&path).unwrap();
    assert!(manifest.update.is_none());
}

#[test]
fn rejects_unknown_placeholder_in_url_template() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = "demo"
version = "0.1.0"
display_name = "Demo"

[entry]
binary = "./garyx-plugin-demo"

[capabilities]
delivery_model = "pull_explicit_ack"

[update]
url_template = "https://example.test/{id}/{nope}/file.tar.gz"
"#,
    );
    let err =
        PluginManifest::load(&path).expect_err("unknown placeholder should reject");
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown placeholder"),
        "error message should mention unknown placeholder; got: {msg}",
    );
    assert!(
        msg.contains("nope"),
        "error should name the bad placeholder; got: {msg}",
    );
}

#[test]
fn update_template_accepts_known_placeholders() {
    let result = super::validate_update_template(
        "https://example.test/{id}/{version}/{target}.tar.gz",
        false,
        Path::new("test"),
    );
    assert!(result.is_ok(), "expected Ok(()), got {result:?}");
}

#[test]
fn update_template_accepts_url_only_when_allowed() {
    let ok = super::validate_update_template("{url}.sha256", true, Path::new("test"));
    assert!(ok.is_ok(), "url placeholder should be allowed; got {ok:?}");

    let err = super::validate_update_template("{url}.sha256", false, Path::new("test"))
        .expect_err("url placeholder must be rejected when not allowed");
    match err {
        ManifestError::UnknownUpdatePlaceholder { placeholder, .. } => {
            assert_eq!(placeholder, "url");
        }
        other => panic!("expected UnknownUpdatePlaceholder, got {other:?}"),
    }
}

#[test]
fn update_template_skips_escaped_double_brace() {
    let result = super::validate_update_template(
        "https://example.test/{{not-a-placeholder}}/{id}-{version}-{target}.tar.gz",
        false,
        Path::new("test"),
    );
    assert!(result.is_ok(), "expected Ok(()), got {result:?}");
}

#[test]
fn update_template_rejects_unterminated_brace() {
    let err = super::validate_update_template(
        "https://example.test/{id",
        false,
        Path::new("test"),
    )
    .expect_err("unterminated brace must be rejected");
    match err {
        ManifestError::UnknownUpdatePlaceholder { placeholder, .. } => {
            assert_eq!(placeholder, "<unterminated>");
        }
        other => panic!("expected UnknownUpdatePlaceholder, got {other:?}"),
    }
}

#[test]
fn update_template_rejects_empty_placeholder() {
    let err = super::validate_update_template(
        "https://example.test/{}/x",
        false,
        Path::new("test"),
    )
    .expect_err("empty placeholder must be rejected");
    match err {
        ManifestError::UnknownUpdatePlaceholder { placeholder, .. } => {
            assert_eq!(placeholder, "");
        }
        other => panic!("expected UnknownUpdatePlaceholder, got {other:?}"),
    }
}

#[test]
fn update_block_with_empty_checksum_template_loads() {
    let (_dir, path) = write_manifest(
        r#"
[plugin]
id = "demo"
version = "0.1.0"
display_name = "Demo"

[entry]
binary = "./garyx-plugin-demo"

[capabilities]
delivery_model = "pull_explicit_ack"

[update]
url_template = "https://example.test/{id}/{version}/{target}.tar.gz"
checksum_url_template = ""
"#,
    );
    let manifest =
        PluginManifest::load(&path).expect("empty checksum_url_template should disable, not error");
    let update = manifest.update.expect("update present");
    assert_eq!(update.checksum_url_template.as_deref(), Some(""));
}
