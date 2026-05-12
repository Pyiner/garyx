use super::*;
use crate::plugin_host::AccountRootBehavior;

fn sample_report() -> InspectReport {
    InspectReport {
        id: "acmechat".into(),
        version: "0.1.0".into(),
        protocol_versions: vec![1],
        capabilities: CapabilitiesResponse {
            outbound: true,
            inbound: true,
            streaming: false,
            images: false,
            files: false,
        },
        auth_flows: vec![AuthFlowDescriptor {
            id: "device_code".into(),
            label: "Device code".into(),
            prompt: "opens browser".into(),
        }],
        schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["token"],
            "properties": {
                "token": {"type": "string"},
                "base_url": {"type": "string"},
            }
        }),
        ui: PluginUiResponse {
            account_root_behavior: AccountRootBehavior::ExpandOnly,
        },
        update: None,
    }
}

#[test]
fn synthesized_manifest_parses_as_pluginmanifest() {
    let report = sample_report();
    let toml = synthesize_manifest_toml(&report, "garyx-plugin-acmechat", None);
    // Drop through toml → PluginManifest to prove the emitted
    // string conforms to the schema the loader expects. Paths to
    // disk aren't hit — we deserialise the string directly.
    let manifest: PluginManifest =
        toml::from_str(&toml).expect("auto-generated manifest must parse");
    assert_eq!(manifest.plugin.id, "acmechat");
    assert_eq!(manifest.plugin.version, "0.1.0");
    assert!(manifest.capabilities.outbound);
    assert!(manifest.capabilities.inbound);
    assert!(!manifest.capabilities.streaming);
    assert_eq!(manifest.entry.binary, "./garyx-plugin-acmechat");
    assert_eq!(manifest.auth_flows.len(), 1);
    assert_eq!(manifest.auth_flows[0].id, "device_code");
    assert_eq!(
        manifest.ui.account_root_behavior,
        AccountRootBehavior::ExpandOnly
    );
    assert_eq!(manifest.schema["type"], "object");
    assert_eq!(manifest.schema["required"][0], "token");
    assert_eq!(manifest.schema["properties"]["token"]["type"], "string");
}

#[test]
fn synthesized_manifest_round_trips_update_block() {
    use crate::plugin_host::manifest::PluginUpdate;

    let mut report = sample_report();
    report.update = Some(PluginUpdate {
        manifest_url: Some("https://example.test/{id}/latest.json".into()),
        url_template:
            "https://example.test/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz"
                .into(),
        checksum_url_template: Some("{url}.sha256".into()),
        binary_in_archive: Some("{id}/garyx-plugin-{id}".into()),
    });

    let toml_out = synthesize_manifest_toml(&report, "garyx-plugin-acmechat", None);
    assert!(
        toml_out.contains("[update]"),
        "expected [update] section:\n{toml_out}",
    );
    assert!(
        toml_out.contains("url_template ="),
        "expected url_template key:\n{toml_out}",
    );
    assert!(
        toml_out.contains(
            "https://example.test/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz",
        ),
        "expected verbatim url_template value:\n{toml_out}",
    );
    assert!(
        toml_out.contains("manifest_url ="),
        "expected manifest_url key:\n{toml_out}",
    );

    // Round-trip: write the synthesized output and reload it via the
    // real manifest loader to prove the [update] block survives a
    // disk round-trip with byte-for-byte field values intact.
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("plugin.toml");
    std::fs::write(&manifest_path, &toml_out).unwrap();
    let reloaded = PluginManifest::load(&manifest_path).expect("manifest must reload");
    let update = reloaded.update.expect("update block should round-trip");
    assert_eq!(
        update.url_template,
        "https://example.test/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz",
    );
    assert_eq!(
        update.manifest_url.as_deref(),
        Some("https://example.test/{id}/latest.json"),
    );
    assert_eq!(update.checksum_url_template.as_deref(), Some("{url}.sha256"));
    assert_eq!(
        update.binary_in_archive.as_deref(),
        Some("{id}/garyx-plugin-{id}"),
    );
}

#[test]
fn synthesized_manifest_round_trip_preserves_schema() {
    // Round-trip a schema with nested objects, arrays, and a
    // $schema key (needs quoting) to catch regressions in the
    // emitter's key-quoting logic.
    let mut report = sample_report();
    report.schema = json!({
        "$schema": "https://example.invalid",
        "type": "object",
        "required": ["a", "b"],
        "properties": {
            "a": {"type": "string", "enum": ["x", "y"]},
            "b": {
                "type": "object",
                "properties": {
                    "c": {"type": "integer", "minimum": 1}
                }
            }
        }
    });
    let toml = synthesize_manifest_toml(&report, "bin", None);
    let manifest: PluginManifest = toml::from_str(&toml).expect("round-trip");
    assert_eq!(manifest.schema["properties"]["a"]["enum"][1], "y");
    assert_eq!(
        manifest.schema["properties"]["b"]["properties"]["c"]["minimum"],
        1
    );
    assert_eq!(manifest.schema["$schema"], "https://example.invalid");
}
