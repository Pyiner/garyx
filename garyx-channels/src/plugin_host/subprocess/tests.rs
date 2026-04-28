use super::*;
use async_trait::async_trait;
use serde_json::Value;
use tempfile::TempDir;

struct NoopHandler;

#[async_trait]
impl InboundHandler for NoopHandler {
    async fn on_request(&self, _method: String, _params: Value) -> Result<Value, (i32, String)> {
        Ok(Value::Null)
    }
    async fn on_notification(&self, _method: String, _params: Value) {}
}

/// Materialise a tiny shell "plugin" that echoes its stdin to
/// stdout verbatim and prints a banner to stderr. Lets us exercise
/// spawn + stdio wiring without needing a compiled Rust plugin.
fn write_echo_plugin(dir: &TempDir) -> PluginManifest {
    let bin_path = dir.path().join("echo-plugin.sh");
    // Using /bin/cat as the echo body lets the host's JSON-RPC
    // frames flow straight back out, which is enough to assert the
    // reader + writer connect correctly.
    let script = "#!/bin/sh\nexec /bin/cat\n";
    std::fs::write(&bin_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).unwrap();
    }

    let manifest_path = dir.path().join("plugin.toml");
    let body = r#"
[plugin]
id = "echo-plugin"
version = "0.0.1"
display_name = "Echo"

[entry]
binary = "./echo-plugin.sh"

[capabilities]
delivery_model = "pull_explicit_ack"

[runtime]
shutdown_grace_ms = 500
"#;
    std::fs::write(&manifest_path, body).unwrap();
    PluginManifest::load(&manifest_path).unwrap()
}

#[tokio::test]
async fn spawn_and_shutdown_escalates_cleanly() {
    let dir = TempDir::new().unwrap();
    let manifest = write_echo_plugin(&dir);
    let mut plugin =
        SubprocessPlugin::spawn(&manifest, SpawnOptions::default(), Arc::new(NoopHandler)).unwrap();
    assert_eq!(plugin.plugin_id(), "echo-plugin");

    // /bin/cat echoes every frame back. We don't speak the
    // protocol here; we just sanity-check that shutdown escalates.
    // `shutdown` RPC will time out (cat never answers), we'll fall
    // through to SIGTERM then SIGKILL.
    let exit_rx = plugin.take_exit_future().unwrap();
    let report = plugin.shutdown_gracefully().await;
    assert!(
        !report.success || report.code == Some(0),
        "unexpected exit report: {report:?}"
    );
    // exit_rx should also resolve.
    let _ = exit_rx.await;
}

#[tokio::test]
async fn exit_watch_reports_early_death() {
    // `/bin/false` exits immediately; we should observe the exit
    // report via the watch channel without blocking.
    let dir = TempDir::new().unwrap();
    let bin_path = dir.path().join("false-plugin.sh");
    std::fs::write(&bin_path, "#!/bin/sh\nexit 3\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).unwrap();
    }
    let manifest_path = dir.path().join("plugin.toml");
    std::fs::write(
        &manifest_path,
        r#"
[plugin]
id = "false-plugin"
version = "0.0.0"
display_name = "False"
[entry]
binary = "./false-plugin.sh"
[capabilities]
delivery_model = "pull_explicit_ack"
"#,
    )
    .unwrap();
    let manifest = PluginManifest::load(&manifest_path).unwrap();

    let plugin =
        SubprocessPlugin::spawn(&manifest, SpawnOptions::default(), Arc::new(NoopHandler)).unwrap();
    let mut watch_rx = plugin.exit_watch();
    // Wait for the child to exit. It should be prompt.
    let report = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(r) = watch_rx.borrow().clone() {
                return r;
            }
            watch_rx.changed().await.unwrap();
        }
    })
    .await
    .expect("child should have exited quickly");
    assert_eq!(report.code, Some(3));
    assert!(!report.success);
}

#[test]
fn stderr_redacts_configured_fields_and_preserves_others() {
    let mut redactions = BTreeSet::new();
    redactions.insert("token".to_owned());
    redactions.insert("password".to_owned());

    let line = r#"{"level":"error","message":"oops","fields":{"token":"secret-xyz","user":"alice","password":"hunter2"}}"#;
    let rec = parse_and_redact_stderr_line(line, &redactions).expect("valid JSON line");
    assert_eq!(rec.level, "error");
    assert_eq!(rec.message, "oops");
    // Field order in serde_json::Map follows insertion order, so we
    // check semantically by re-parsing.
    let fields: Value = serde_json::from_str(&rec.fields_json).unwrap();
    assert_eq!(fields["token"], "<redacted>");
    assert_eq!(fields["password"], "<redacted>");
    assert_eq!(fields["user"], "alice");
}

#[test]
fn stderr_redacts_nested_fields_at_any_depth() {
    // §8.2 permits nested config objects; §8.4 requires every
    // matching secret key to be scrubbed before the host writes the
    // log line. The earlier implementation only walked top-level
    // `fields` keys, which silently leaked nested secrets.
    let mut redactions = BTreeSet::new();
    redactions.insert("token".to_owned());
    redactions.insert("api_key".to_owned());

    let line = r#"{
            "level":"info",
            "message":"connecting",
            "fields":{
                "credentials":{"token":"secret-a","user":"alice"},
                "nested":{"inner":{"api_key":"secret-b"}},
                "list":[{"token":"secret-c"}, {"other":"ok"}],
                "public":"ok"
            }
        }"#;
    let rec = parse_and_redact_stderr_line(line, &redactions).unwrap();
    let fields: Value = serde_json::from_str(&rec.fields_json).unwrap();
    assert_eq!(fields["credentials"]["token"], "<redacted>");
    assert_eq!(fields["credentials"]["user"], "alice");
    assert_eq!(fields["nested"]["inner"]["api_key"], "<redacted>");
    assert_eq!(fields["list"][0]["token"], "<redacted>");
    assert_eq!(fields["list"][1]["other"], "ok");
    assert_eq!(fields["public"], "ok");
}

#[test]
fn stderr_redaction_rewrites_non_string_values_too() {
    // Secrets can have any JSON shape (int tokens, boolean flags,
    // structured objects). The redactor must replace whatever is
    // there with "<redacted>", not leak the raw value.
    let mut redactions = BTreeSet::new();
    redactions.insert("token".to_owned());

    let line =
        r#"{"level":"info","message":"","fields":{"token":12345,"also":{"token":{"nested":"x"}}}}"#;
    let rec = parse_and_redact_stderr_line(line, &redactions).unwrap();
    let fields: Value = serde_json::from_str(&rec.fields_json).unwrap();
    assert_eq!(fields["token"], "<redacted>");
    assert_eq!(fields["also"]["token"], "<redacted>");
}

#[test]
fn noncompliant_structured_json_drops_body_and_never_leaks_secret() {
    // The exact pass-4 payload. `{"fields":"token=leak-xyz"}` parses
    // as JSON, but `fields` is a string — key-scoped recursive
    // redaction can't touch its leaf. The only safe policy is to
    // drop the body entirely. Verify that the classifier does NOT
    // return the body anywhere in its output.
    let leak = r#"{"fields":"token=leak-xyz"}"#;
    match classify_noncompliant_line(leak) {
        NoncompliantLine::ParseableJsonWrongShape { shape } => {
            assert!(
                !shape.contains("leak-xyz"),
                "shape summary must not echo the secret-carrying leaf: {shape}"
            );
            assert!(
                !shape.contains("token=leak"),
                "shape summary must not echo the raw body: {shape}"
            );
            assert!(
                shape.contains("fields"),
                "shape summary should report the keys for debugging: {shape}"
            );
        }
        other => panic!("expected ParseableJsonWrongShape, got {other:?}"),
    }

    // Extra: run the same leak through emit_stderr_line and confirm
    // it still flips the advisory flag (so operators see the
    // schema complaint once per plugin lifetime).
    let mut redactions = BTreeSet::new();
    redactions.insert("token".to_owned());
    let mut warned = false;
    emit_stderr_line("p", leak, &redactions, &mut warned);
    assert!(
        warned,
        "non-compliant JSON should flip the advisory flag on first hit"
    );
}

#[test]
fn describe_shape_never_reflects_values() {
    // A second, broader invariant on the shape summary: across all
    // interesting JSON top-level shapes, the string never echoes
    // any of the input values. We use high-entropy sentinels so we
    // aren't confused by substrings that happen to appear inside a
    // type label like `array` or `object`.
    let cases: &[(&str, &[&str])] = &[
        // Object values (numeric + string) must not leak; only keys
        // are surfaced.
        (
            r#"{"alpha":"val-ABCDEF","beta":987654321}"#,
            &["val-ABCDEF", "987654321"],
        ),
        // Nested array with unique tokens inside — shape only
        // reports the outer length.
        (
            r#"[{"inner-uniq-a":"leaf-ZZZZ"}, 1, 2]"#,
            &["inner-uniq-a", "leaf-ZZZZ"],
        ),
        // Top-level scalars: the summary should say the type name,
        // not echo the value.
        (r#""free-TEXT-SENTINEL""#, &["free-TEXT-SENTINEL"]),
        (r#"13579246801"#, &["13579246801"]),
    ];
    for (payload, forbidden_substrings) in cases {
        let val: Value = serde_json::from_str(payload).unwrap();
        let shape = describe_shape(&val);
        for forbidden in *forbidden_substrings {
            assert!(
                !shape.contains(forbidden),
                "describe_shape({payload:?}) = {shape:?} must not contain {forbidden:?}"
            );
        }
    }
}

#[test]
fn classify_noncompliant_line_distinguishes_json_from_raw() {
    assert!(matches!(
        classify_noncompliant_line("this is not json"),
        NoncompliantLine::NonJson
    ));
    assert!(matches!(
        classify_noncompliant_line(r#"{"arbitrary":"object"}"#),
        NoncompliantLine::ParseableJsonWrongShape { .. }
    ));
    assert!(matches!(
        classify_noncompliant_line(r#"[1, 2, 3]"#),
        NoncompliantLine::ParseableJsonWrongShape { .. }
    ));
}

#[tokio::test]
async fn reader_fatal_error_forces_child_exit() {
    // Codex pass-4 blocker: a plugin that emits an invalid JSON-RPC
    // envelope trips a fatal reader error (§11.1), but without the
    // reader supervisor the child stays alive because supervision
    // only watches `child.wait()`. Verify that the supervisor now
    // escalates to SIGKILL so the exit_watch actually fires.
    let dir = TempDir::new().unwrap();
    let bin_path = dir.path().join("evil-plugin.sh");
    // Emit one frame that violates §5.2 (no `jsonrpc` field), then
    // sleep forever. Without the supervisor, the host's reader dies
    // but the child keeps looping.
    std::fs::write(
        &bin_path,
        r#"#!/bin/sh
printf 'Content-Length: 19\r\n\r\n{"not":"compliant"}'
while true; do sleep 1; done
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).unwrap();
    }
    let manifest_path = dir.path().join("plugin.toml");
    std::fs::write(
        &manifest_path,
        r#"
[plugin]
id = "evil-envelope"
version = "0.0.0"
display_name = "Evil"
[entry]
binary = "./evil-plugin.sh"
[capabilities]
delivery_model = "pull_explicit_ack"
[runtime]
shutdown_grace_ms = 200
"#,
    )
    .unwrap();
    let manifest = PluginManifest::load(&manifest_path).unwrap();
    let plugin =
        SubprocessPlugin::spawn(&manifest, SpawnOptions::default(), Arc::new(NoopHandler)).unwrap();

    let mut watch_rx = plugin.exit_watch();
    let report = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(r) = watch_rx.borrow().clone() {
                return r;
            }
            watch_rx.changed().await.unwrap();
        }
    })
    .await
    .expect("reader supervisor must kill the child on fatal envelope error");

    assert!(
        !report.success,
        "force-killed child must not report success: {report:?}"
    );
    // On Unix the child should have been terminated by SIGKILL.
    #[cfg(unix)]
    assert_eq!(
        report.signal,
        Some(libc::SIGKILL),
        "expected SIGKILL signal on supervisor-forced exit: {report:?}"
    );
}

#[tokio::test]
async fn drop_does_not_signal_after_reap_observed() {
    // Codex pass-3 flagged: Drop unconditionally signalled the
    // cached PID, which after reap could hit a recycled PID. The
    // exit task now disarms `ChildKill`, and Drop also bails early
    // when `exit_watch` already shows a report. Verify both gates.
    let dir = TempDir::new().unwrap();
    let bin_path = dir.path().join("quick-exit.sh");
    std::fs::write(&bin_path, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).unwrap();
    }
    let manifest_path = dir.path().join("plugin.toml");
    std::fs::write(
        &manifest_path,
        r#"
[plugin]
id = "quick-drop"
version = "0.0.0"
display_name = "Quick"
[entry]
binary = "./quick-exit.sh"
[capabilities]
delivery_model = "pull_explicit_ack"
"#,
    )
    .unwrap();
    let manifest = PluginManifest::load(&manifest_path).unwrap();
    let plugin =
        SubprocessPlugin::spawn(&manifest, SpawnOptions::default(), Arc::new(NoopHandler)).unwrap();

    // Wait until the exit_task has observed reap and disarmed.
    let mut watch_rx = plugin.exit_watch();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if watch_rx.borrow().is_some() {
                return;
            }
            watch_rx.changed().await.unwrap();
        }
    })
    .await
    .expect("child should reap quickly");

    // After reap, the kill handle must be disarmed.
    assert_eq!(
        plugin.child_kill.pid.load(Ordering::Acquire),
        -1,
        "exit task must disarm ChildKill after wait() returns"
    );

    // And Drop must be a no-op — dropping does not panic, does not
    // crash the runtime even though signal() would now find -1.
    drop(plugin);
}

#[test]
fn stderr_non_object_fields_is_treated_as_malformed() {
    // §8.4 says `fields` must be an object so the host can redact.
    // A string/array there would bypass redaction entirely — route
    // to the malformed-line path so the operator sees it.
    let mut redactions = BTreeSet::new();
    redactions.insert("token".to_owned());
    let line = r#"{"level":"info","message":"hi","fields":"oops-token=abc"}"#;
    assert!(parse_and_redact_stderr_line(line, &redactions).is_none());

    let line2 = r#"{"level":"info","message":"hi","fields":[{"token":"x"}]}"#;
    assert!(parse_and_redact_stderr_line(line2, &redactions).is_none());
}

#[test]
fn stderr_malformed_line_returns_none() {
    let redactions = BTreeSet::new();
    assert!(parse_and_redact_stderr_line("this is not json", &redactions).is_none());
    // Non-object JSON is still malformed per §8.4 contract.
    assert!(parse_and_redact_stderr_line("[1,2,3]", &redactions).is_none());
    assert!(parse_and_redact_stderr_line("\"just a string\"", &redactions).is_none());
    // Empty / whitespace-only lines: also None (emit_stderr_line
    // short-circuits these before calling, but the helper is
    // defensive too).
    assert!(parse_and_redact_stderr_line("", &redactions).is_none());
    assert!(parse_and_redact_stderr_line("   ", &redactions).is_none());
}

#[test]
fn stderr_malformed_line_sets_one_shot_warn_flag() {
    let redactions = BTreeSet::new();
    let mut warned = false;
    emit_stderr_line("p", "not json", &redactions, &mut warned);
    assert!(warned, "first malformed line must flip the warn flag");
    // Second malformed line must not clear it — idempotent set.
    emit_stderr_line("p", "also not json", &redactions, &mut warned);
    assert!(warned);
    // A well-formed line doesn't reset the flag either.
    emit_stderr_line(
        "p",
        r#"{"level":"info","message":"ok","fields":{}}"#,
        &redactions,
        &mut warned,
    );
    assert!(warned);
}

#[test]
fn synthetic_post_sigkill_report_is_always_failure() {
    // Codex pass-2 flagged: the earlier fast-exit test didn't
    // actually exercise the synthetic branch (the kernel always
    // reported an exit before the budget expired). Pin the
    // invariant directly on the pure helper so the "never claim
    // success" guarantee doesn't depend on racing a real child.
    let report = synthetic_post_sigkill_report();
    assert!(
        !report.success,
        "synthetic post-SIGKILL report must report failure: {report:?}"
    );
    assert_eq!(report.code, None, "synthetic report has no exit code");
    #[cfg(unix)]
    assert_eq!(report.signal, Some(libc::SIGKILL));
}

#[tokio::test]
async fn shutdown_returns_real_report_for_quick_exit_child() {
    // Complementary smoke test: happy path returns the real report
    // and does not loiter in the escalation loop.
    let dir = TempDir::new().unwrap();
    let bin_path = dir.path().join("quick-exit.sh");
    std::fs::write(&bin_path, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).unwrap();
    }
    let manifest_path = dir.path().join("plugin.toml");
    std::fs::write(
        &manifest_path,
        r#"
[plugin]
id = "quick"
version = "0.0.0"
display_name = "Quick"
[entry]
binary = "./quick-exit.sh"
[capabilities]
delivery_model = "pull_explicit_ack"
[runtime]
shutdown_grace_ms = 200
"#,
    )
    .unwrap();
    let manifest = PluginManifest::load(&manifest_path).unwrap();
    let plugin =
        SubprocessPlugin::spawn(&manifest, SpawnOptions::default(), Arc::new(NoopHandler)).unwrap();

    let start = std::time::Instant::now();
    let report = plugin.shutdown_gracefully().await;
    let elapsed = start.elapsed();
    // Upper bound: we must never block beyond the sum of the
    // protocol-mandated escalation windows. With the split §11.1
    // timeout model that is: up to `SHUTDOWN_RPC_TIMEOUT` (10s) for
    // the `shutdown` RPC + `shutdown_grace_ms` (200ms here) for the
    // exit drain + `SIGTERM_GRACE` (2s) + `SIGKILL_REAP_BUDGET` (5s).
    // For a non-responsive child that never opens a reader the RPC
    // will resolve quickly — the writer drops on close, so the 10s
    // bound is not binding — but we still cap the assertion
    // conservatively below to guard against the "shutdown hangs"
    // regression.
    assert!(
        elapsed < Duration::from_secs(10),
        "shutdown took {elapsed:?}; escalation path is stuck"
    );
    // And critically: if the code was None (synthesised after
    // SIGKILL reap timeout), we must have reported failure, not
    // success. Codex's review called this out specifically.
    if report.code.is_none() {
        assert!(
            !report.success,
            "synthetic ExitReport after SIGKILL must not claim success: {report:?}"
        );
    } else {
        // Real report from /bin/sh exit 0.
        assert_eq!(report.code, Some(0));
        assert!(report.success);
    }
}

#[test]
fn binary_missing_surfaces_clearly() {
    let dir = TempDir::new().unwrap();
    let manifest_path = dir.path().join("plugin.toml");
    std::fs::write(
        &manifest_path,
        r#"
[plugin]
id = "ghost"
version = "0.0.0"
display_name = "Ghost"
[entry]
binary = "./not-there"
[capabilities]
delivery_model = "pull_explicit_ack"
"#,
    )
    .unwrap();
    let manifest = PluginManifest::load(&manifest_path).unwrap();
    let result = SubprocessPlugin::spawn(&manifest, SpawnOptions::default(), Arc::new(NoopHandler));
    match result {
        Err(SubprocessError::BinaryMissing(_)) => {}
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("spawn should have failed"),
    }
}
