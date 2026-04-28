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
