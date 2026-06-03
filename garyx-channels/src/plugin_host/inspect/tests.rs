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
            survives_respawn: false,
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
            "https://example.test/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz".into(),
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
    assert_eq!(
        update.checksum_url_template.as_deref(),
        Some("{url}.sha256")
    );
    assert_eq!(
        update.binary_in_archive.as_deref(),
        Some("{id}/garyx-plugin-{id}"),
    );
}

#[test]
fn synthesized_manifest_threads_survives_respawn_opt_in() {
    // Regression guard: a plugin that advertises survives_respawn
    // in describe MUST get the bit into the synthesized plugin.toml,
    // round-tripping through the loader so the host's swap-time
    // snapshot read sees the same value. Pre-fix this field was
    // dropped silently, stranding opted-in plugins at first-install.
    let mut report = sample_report();
    report.capabilities.survives_respawn = true;
    let toml_out = synthesize_manifest_toml(&report, "garyx-plugin-acmechat", None);
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("plugin.toml");
    std::fs::write(&manifest_path, &toml_out).unwrap();
    let reloaded = PluginManifest::load(&manifest_path).expect("manifest must reload");
    assert!(reloaded.capabilities.survives_respawn);
}

#[test]
fn backfill_writes_field_when_missing_from_capabilities() {
    // Stale plugin.toml shape produced by garyx < 0.1.25 — the
    // [capabilities] block has the original five fields and no
    // survives_respawn. Backfill must insert the field at the end
    // of the section so the host's swap-time snapshot sees it.
    let stale = "[plugin]\nid = \"acmechat\"\nversion = \"0.1.0\"\ndisplay_name = \"Acmechat\"\n\n\
                 [entry]\nbinary = \"./garyx-plugin-acmechat\"\n\n\
                 [capabilities]\ndelivery_model = \"pull_explicit_ack\"\n\
                 outbound = true\ninbound = true\nstreaming = false\n\
                 images = true\nfiles = true\n\n\
                 [runtime]\nstop_grace_ms = 5000\nshutdown_grace_ms = 3000\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plugin.toml");
    std::fs::write(&path, stale).unwrap();

    let outcome = backfill_survives_respawn_in_place(&path).expect("backfill should succeed");
    assert_eq!(outcome, BackfillOutcome::Wrote);

    // Reload via the canonical loader to confirm the on-disk
    // result parses and the bit is visible to PluginManifest
    // consumers (this is what the host snapshot reads).
    let reloaded = PluginManifest::load(&path).expect("patched manifest must reload");
    assert!(reloaded.capabilities.survives_respawn);

    // Other fields must survive untouched — backfill is narrow.
    assert!(reloaded.capabilities.outbound);
    assert!(reloaded.capabilities.files);
    assert_eq!(reloaded.plugin.id, "acmechat");
    assert_eq!(reloaded.plugin.version, "0.1.0");
}

#[test]
fn backfill_respects_explicit_opt_out() {
    // Operator wrote `survives_respawn = false` by hand to opt OUT
    // of silent self-update — that's a legitimate choice (e.g. a
    // plugin under active local development they don't want
    // hot-swapped). Backfill must NOT flip it back to true.
    let opt_out = "[plugin]\nid = \"acmechat\"\nversion = \"0.1.0\"\ndisplay_name = \"Acmechat\"\n\n\
                   [entry]\nbinary = \"./garyx-plugin-acmechat\"\n\n\
                   [capabilities]\ndelivery_model = \"pull_explicit_ack\"\n\
                   outbound = true\ninbound = true\nstreaming = false\n\
                   images = true\nfiles = true\nsurvives_respawn = false\n\n\
                   [runtime]\nstop_grace_ms = 5000\nshutdown_grace_ms = 3000\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plugin.toml");
    std::fs::write(&path, opt_out).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();

    let outcome = backfill_survives_respawn_in_place(&path).expect("backfill should succeed");
    assert_eq!(outcome, BackfillOutcome::AlreadyPresent);

    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        before, after,
        "file must be byte-identical when survives_respawn is already present"
    );
}

#[test]
fn backfill_errors_when_no_capabilities_section() {
    // A plugin.toml with no [capabilities] block at all is
    // malformed (loader would reject it too). Backfill returns
    // an error rather than guessing where to put the field.
    let no_caps = "[plugin]\nid = \"foo\"\nversion = \"0.1.0\"\n\n\
                   [entry]\nbinary = \"./foo\"\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plugin.toml");
    std::fs::write(&path, no_caps).unwrap();

    let err = backfill_survives_respawn_in_place(&path)
        .expect_err("should refuse to write without [capabilities]");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn backfill_ignores_field_name_mention_outside_capabilities_section() {
    // Codex review: a stray comment elsewhere in the file
    // (e.g. a release note, a doc comment in [runtime], or the
    // field name appearing inside a different section's key)
    // must NOT suppress the heal — backfill scopes the
    // "already present" check to the [capabilities] block.
    let stale = "[plugin]\nid = \"x\"\nversion = \"0.1.0\"\ndisplay_name = \"X\"\n\
                 # see survives_respawn semantics in §9.4\n\n\
                 [entry]\nbinary = \"./x\"\n\n\
                 [capabilities]\ndelivery_model = \"pull_explicit_ack\"\n\
                 outbound = true\ninbound = true\nstreaming = false\n\
                 images = true\nfiles = true\n\n\
                 [runtime]\nstop_grace_ms = 5000\nshutdown_grace_ms = 3000\n\
                 # operator note: survives_respawn migration tracker\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plugin.toml");
    std::fs::write(&path, stale).unwrap();

    let outcome = backfill_survives_respawn_in_place(&path).expect("backfill should succeed");
    assert_eq!(
        outcome,
        BackfillOutcome::Wrote,
        "comments mentioning the field name must not suppress healing"
    );
    let reloaded = PluginManifest::load(&path).expect("must reload");
    assert!(reloaded.capabilities.survives_respawn);
}

#[test]
fn backfill_explicit_opt_out_after_blank_line_is_detected() {
    // Codex round-3: blank lines do NOT end a TOML table. If an
    // operator chose to opt out by writing `survives_respawn =
    // false` AFTER a blank line inside [capabilities], the prior
    // boundary-via-blank-line logic would treat the blank as
    // end-of-section, insert `survives_respawn = true` there, and
    // miss the explicit `false` below — producing a malformed
    // plugin.toml with duplicate keys. New logic only ends the
    // section on the next `[...]` header (or EOF).
    let opt_out_after_blank = "[plugin]\nid = \"x\"\nversion = \"0.1.0\"\ndisplay_name = \"X\"\n\n\
         [entry]\nbinary = \"./x\"\n\n\
         [capabilities]\ndelivery_model = \"pull_explicit_ack\"\n\
         outbound = true\ninbound = true\nstreaming = false\n\
         images = true\nfiles = true\n\n\
         survives_respawn = false\n\n\
         [runtime]\nstop_grace_ms = 5000\nshutdown_grace_ms = 3000\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plugin.toml");
    std::fs::write(&path, opt_out_after_blank).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();

    let outcome = backfill_survives_respawn_in_place(&path).expect("backfill should succeed");
    assert_eq!(
        outcome,
        BackfillOutcome::AlreadyPresent,
        "explicit false after a blank line inside [capabilities] must still be detected"
    );

    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after, "opt-out file must remain byte-identical");

    // Sanity: the file still parses (no duplicate-key corruption).
    let reloaded = PluginManifest::load(&path).expect("must reload as valid TOML");
    assert!(
        !reloaded.capabilities.survives_respawn,
        "operator's explicit opt-out must be preserved"
    );
}

#[test]
fn backfill_same_file_concurrent_contention_lands_one_writer() {
    // Codex round-3: same-file contention is the production concern
    // — two garyx instances racing the SAME plugin.toml (e.g.
    // launchd-managed + manual `gateway run`). Each thread either
    // writes first (Wrote) or sees the field already present
    // (AlreadyPresent); both outcomes are acceptable and the file
    // must end up valid + carry the field exactly once.
    use std::sync::Barrier;
    use std::thread;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plugin.toml");
    let stale = "[plugin]\nid = \"x\"\nversion = \"0.1.0\"\ndisplay_name = \"X\"\n\n\
                 [entry]\nbinary = \"./x\"\n\n\
                 [capabilities]\ndelivery_model = \"pull_explicit_ack\"\n\
                 outbound = true\ninbound = true\nstreaming = false\n\
                 images = true\nfiles = true\n\n\
                 [runtime]\nstop_grace_ms = 5000\nshutdown_grace_ms = 3000\n";
    std::fs::write(&path, stale).unwrap();

    let barrier = std::sync::Arc::new(Barrier::new(4));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let path = path.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            backfill_survives_respawn_in_place(&path)
        }));
    }
    let mut outcomes = Vec::new();
    for h in handles {
        outcomes.push(h.join().unwrap().expect("no thread should error"));
    }
    assert!(
        outcomes.iter().any(|o| matches!(o, BackfillOutcome::Wrote)),
        "at least one writer should have landed the patch"
    );

    // Final file: must be valid TOML, carry survives_respawn = true,
    // and contain the key EXACTLY once (proving no two writers
    // raced into a duplicate-key state).
    let body = std::fs::read_to_string(&path).unwrap();
    let occurrences = body
        .lines()
        .filter(|l| {
            let t = l.trim();
            t.strip_prefix("survives_respawn")
                .map(|rest| rest.trim_start().starts_with('='))
                .unwrap_or(false)
        })
        .count();
    assert_eq!(
        occurrences, 1,
        "exactly one survives_respawn key should land:\n{body}"
    );
    let reloaded = PluginManifest::load(&path).expect("must reload");
    assert!(reloaded.capabilities.survives_respawn);
}

#[test]
fn backfill_handles_capabilities_as_trailing_section() {
    // Edge case: [capabilities] is the LAST section in the file
    // with no blank line / next-section header after it. The
    // boundary detector must fall through to EOF and still
    // emit the new field at the end.
    let trailing = "[plugin]\nid = \"x\"\nversion = \"0.1.0\"\n\n\
                    [entry]\nbinary = \"./x\"\n\n\
                    [capabilities]\ndelivery_model = \"pull_explicit_ack\"\n\
                    outbound = true\ninbound = true\nstreaming = false\n\
                    images = true\nfiles = true\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plugin.toml");
    std::fs::write(&path, trailing).unwrap();

    let outcome = backfill_survives_respawn_in_place(&path).expect("backfill should succeed");
    assert_eq!(outcome, BackfillOutcome::Wrote);
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(
        body.contains("survives_respawn = true"),
        "trailing-section file should still get the field:\n{body}"
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
