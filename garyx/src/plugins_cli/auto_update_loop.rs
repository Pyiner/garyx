//! Silent plugin auto-updater.
//!
//! Runs as a long-lived tokio task spawned at gateway boot. Each
//! tick walks every installed subprocess plugin under
//! [`default_plugin_install_root`], checks the plugin's declared
//! `[update].manifest_url` (`garyx-channels::plugin_host::PluginUpdate`)
//! for a newer version, and — when one is found AND the plugin opted
//! in via `[capabilities].survives_respawn = true` — downloads,
//! verifies, atomically promotes the new bundle, and hot-swaps the
//! running subprocess via
//! [`ChannelPluginManager::respawn_plugin_with_fresh_manifest`].
//!
//! Design parallels `desktop/garyx-desktop/src/main/updater.ts`: a
//! short initial delay so the loop doesn't compete with boot, then
//! a recurring tick. All failures are warn-logged and silently
//! retried on the next tick — auto-update is best-effort and must
//! never break a running gateway.
//!
//! Plugins that do not opt in to `survives_respawn` are skipped at
//! the hot-swap step (and the operator sees a single warn-log noting
//! that a new version is available but a manual restart is needed).
//! This matches the conservative default: the host has no way to
//! prove a subprocess will resume cleanly across a respawn, so the
//! plugin author must vouch for it.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use garyx_channels::plugin::ChannelPluginManager;
use garyx_channels::plugin_host::PluginManifest;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::update::{
    EffectiveSource, current_target_alias, discover_latest_version, download_archive,
    extract_and_locate_binary, http_client, render_template,
};
use crate::channel_plugin_host::default_plugin_install_root;

/// Delay before the first auto-update tick. Lets boot-time work
/// (config load, runtime assembly, initial plugin registration)
/// settle before the loop starts touching the install root. Mirrors
/// `INITIAL_CHECK_DELAY_MS = 8_000` in the desktop updater.
const INITIAL_CHECK_DELAY: Duration = Duration::from_secs(8);

/// Lower bound on the recurring tick. Operators can shorten the
/// interval in `garyx.json` but only down to here — anything tighter
/// risks rate-limiting from upstream manifest hosts.
const MIN_INTERVAL_SECS: u64 = 60;

/// Runtime knobs forwarded from `GaryxConfig.plugins`. Kept as a
/// dedicated struct so the spawn path doesn't have to pull in the
/// full config crate.
#[derive(Debug, Clone)]
pub struct AutoUpdateConfig {
    pub enabled: bool,
    pub interval_secs: u64,
}

/// Spawn the auto-update loop. Returns `None` when the loop is
/// disabled by config (or by the static dev-build guard), so callers
/// don't have to keep a join handle around in that case.
pub fn spawn(
    plugin_manager: Arc<Mutex<ChannelPluginManager>>,
    config: AutoUpdateConfig,
) -> Option<tokio::task::JoinHandle<()>> {
    if !config.enabled {
        info!("plugin auto-update disabled by config");
        return None;
    }
    Some(tokio::spawn(async move {
        run(plugin_manager, config).await;
    }))
}

async fn run(plugin_manager: Arc<Mutex<ChannelPluginManager>>, config: AutoUpdateConfig) {
    let interval = Duration::from_secs(config.interval_secs.max(MIN_INTERVAL_SECS));
    info!(
        initial_delay_secs = INITIAL_CHECK_DELAY.as_secs(),
        interval_secs = interval.as_secs(),
        "plugin auto-update loop scheduled"
    );
    tokio::time::sleep(INITIAL_CHECK_DELAY).await;
    loop {
        tick(&plugin_manager).await;
        tokio::time::sleep(interval).await;
    }
}

async fn tick(plugin_manager: &Arc<Mutex<ChannelPluginManager>>) {
    let target_root = default_plugin_install_root();
    let ids = match list_installed_plugin_ids(&target_root) {
        Ok(ids) => ids,
        Err(err) => {
            warn!(
                root = %target_root.display(),
                error = %err,
                "plugin auto-update: failed to enumerate install root; will retry"
            );
            return;
        }
    };
    for id in ids {
        if let Err(err) = process_plugin(plugin_manager, &target_root, &id).await {
            warn!(
                plugin_id = %id,
                error = %err,
                "plugin auto-update: failed; will retry on next tick"
            );
        }
    }
}

/// Decision a tick makes about a single plugin. Materialized so the
/// matrix of (running, disk, latest) version combinations is unit-
/// testable without exercising the full async path (which depends on
/// the filesystem and an upstream manifest_url).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoUpdateAction {
    /// Running subprocess is already at the advertised latest. Tick
    /// stays quiet — no download, no respawn, no log.
    NoOp,
    /// On-disk bundle already matches latest (e.g. a prior tick
    /// successfully promoted but the respawn step failed). Skip the
    /// download and go straight to hot-swap.
    RespawnOnly,
    /// Disk + running both behind latest. Full path: download +
    /// verify + atomic promote + respawn.
    DownloadAndRespawn,
}

/// Pure decision function. Compares the running subprocess version
/// (authoritative for "what's actually serving") against the on-disk
/// bundle version (which an earlier successful promote may have
/// advanced past the running image) and the upstream-advertised
/// latest. Three observable cases — see `AutoUpdateAction`.
fn decide_action(running: &str, disk: &str, latest: &str) -> AutoUpdateAction {
    if running == latest {
        AutoUpdateAction::NoOp
    } else if disk == latest {
        // The disk advanced past the running process — a prior
        // tick's promote landed but respawn didn't. Don't redownload
        // the same bundle; just retry the respawn this tick.
        AutoUpdateAction::RespawnOnly
    } else {
        AutoUpdateAction::DownloadAndRespawn
    }
}

async fn process_plugin(
    plugin_manager: &Arc<Mutex<ChannelPluginManager>>,
    target_root: &Path,
    id: &str,
) -> Result<(), String> {
    // Step 1 — load the on-disk manifest ONCE. Source of truth for
    // the [update] block and the [capabilities].survives_respawn
    // opt-in. The `disk_version` we extract is the version of the
    // bundle currently in the install root, which may be ahead of
    // the running subprocess if a prior tick promoted but failed to
    // respawn.
    let installed_manifest_path = target_root.join(id).join("plugin.toml");
    let installed_manifest = PluginManifest::load(&installed_manifest_path).map_err(|err| {
        format!(
            "loading installed manifest at {}: {err}",
            installed_manifest_path.display()
        )
    })?;
    let disk_version = installed_manifest.plugin.version.clone();

    // Step 2 — `[update]` + `manifest_url` are required for
    // unattended discovery. Plugins that opt out by declining either
    // are silently skipped — the operator can still drive
    // `garyx plugins update --version <x>` / `--from <bundle>` by
    // hand. We intentionally don't consult the builtin fallback
    // table (`BUILTIN_PLUGIN_UPDATE_SOURCES`): every entry there has
    // `manifest_url: None` anyway, so it could never advance past
    // this gate.
    let Some(update) = installed_manifest.update.as_ref() else {
        return Ok(());
    };
    let Some(manifest_url) = update.manifest_url.clone() else {
        return Ok(());
    };
    let source = EffectiveSource::from(update.clone());

    // Step 3 — capture the version of the running subprocess. This
    // is the authoritative "current" version: even after a prior
    // tick promoted a new bundle, the live entry's manifest will
    // still report the OLD version until a respawn succeeds (the
    // §9.4 respawn path rolls back the in-memory manifest on spawn
    // failure to keep observers consistent). Skip the plugin
    // silently if it's not registered — that means the boot path
    // hit an error registering it; nothing the auto-updater can do.
    let running_version = {
        let mgr = plugin_manager.lock().await;
        match mgr.subprocess_plugin_version(id) {
            Some(v) => v,
            None => return Ok(()),
        }
    };

    // Step 4 — discover latest version. Network failures are silent
    // retries; nothing to log every 6h about a flaky upstream.
    let target = current_target_alias().map_err(|e| e.to_string())?;
    let client = http_client().map_err(|e| e.to_string())?;
    let rendered_manifest_url =
        render_template(&manifest_url, id, "", target, None).map_err(|e| e.to_string())?;
    let latest = match discover_latest_version(&client, &rendered_manifest_url, id).await {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    // Step 5 — decide what this tick should do given the three
    // version anchors. See `decide_action` for the matrix.
    let action = decide_action(&running_version, &disk_version, &latest);
    match action {
        AutoUpdateAction::NoOp => return Ok(()),
        AutoUpdateAction::RespawnOnly => {
            info!(
                plugin_id = %id,
                running = %running_version,
                disk = %disk_version,
                to = %latest,
                "plugin auto-update: bundle already on disk from a prior tick; retrying respawn"
            );
        }
        AutoUpdateAction::DownloadAndRespawn => {
            info!(
                plugin_id = %id,
                running = %running_version,
                disk = %disk_version,
                to = %latest,
                "plugin auto-update: new version available"
            );
        }
    }

    // Step 6 — survives_respawn opt-in check. Plugins that haven't
    // opted in get skipped before any download or on-disk mutation
    // — both the bundle and the running subprocess stay on the old
    // version. (For the `RespawnOnly` branch we know the disk is
    // already ahead from a prior promote; we still respect the
    // current manifest's `survives_respawn` value — which is the
    // *new* bundle's contract since we read it from the disk
    // manifest above.)
    if !installed_manifest.capabilities.survives_respawn {
        warn!(
            plugin_id = %id,
            running = %running_version,
            disk = %disk_version,
            to = %latest,
            "plugin auto-update: new version available but plugin did not opt in to \
             [capabilities].survives_respawn; skipping silent hot-replace. Run \
             `garyx plugins update {id} && garyx gateway restart` to apply manually."
        );
        return Ok(());
    }

    // Step 7 — download + verify + extract + atomic promote (only
    // when the disk doesn't already have the target bundle).
    if action == AutoUpdateAction::DownloadAndRespawn {
        // Step 7a — render archive + checksum URLs. Same three-state
        // checksum semantics the CLI honors: None → "{url}.sha256",
        // Some("") → operator disabled, Some(t) → custom template.
        let archive_url = render_template(&source.url_template, id, &latest, target, None)
            .map_err(|e| e.to_string())?;
        let checksum_url = match source.checksum_url_template.as_deref() {
            Some("") => None,
            Some(template) => Some(
                render_template(template, id, &latest, target, Some(&archive_url))
                    .map_err(|e| e.to_string())?,
            ),
            None => Some(
                render_template("{url}.sha256", id, &latest, target, Some(&archive_url))
                    .map_err(|e| e.to_string())?,
            ),
        };

        // Step 7b — download + sha256-verify.
        let archive_bytes = if let Some(ck) = checksum_url.as_deref() {
            download_archive(&client, &archive_url, ck)
                .await
                .map_err(|e| e.to_string())?
        } else {
            client
                .get(&archive_url)
                .timeout(Duration::from_secs(300))
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| format!("download {archive_url}: {e}"))?
                .bytes()
                .await
                .map_err(|e| format!("download {archive_url} body: {e}"))?
                .to_vec()
        };

        // Step 7c — stage into a sibling tempdir under the install
        // root so the final rename is same-filesystem and atomic.
        std::fs::create_dir_all(target_root).map_err(|e| format!("creating install root: {e}"))?;
        let staging: TempDir =
            tempfile::tempdir_in(target_root).map_err(|e| format!("creating staging dir: {e}"))?;
        // `extract_and_locate_binary` unpacks the whole tarball into
        // `staging` AND validates that the binary lives at the path
        // the manifest claims. The bundle root is always
        // `<staging>/<id>/` per the canonical tarball layout
        // documented in `docs/configuration.md`. Trusting
        // `binary.parent()` to discover the bundle dir would break
        // plugins that use a nested `binary_in_archive` such as
        // `{id}/bin/garyx-plugin-{id}` — the rename would lift only
        // the `bin/` directory, dropping `plugin.toml` and `icon.*`.
        extract_and_locate_binary(
            &archive_bytes,
            staging.path(),
            id,
            &source.binary_in_archive,
        )
        .map_err(|e| e.to_string())?;
        let bundle_dir = staging.path().join(id);
        if !bundle_dir.is_dir() {
            return Err(format!(
                "expected bundle layout `<staging>/{id}/...` but {} is not a directory",
                bundle_dir.display()
            ));
        }

        // Step 7d — sanity-check the bundled plugin.toml before
        // promotion. The bundle ships with the author's own manifest
        // (preserving their `[update]` block + capability bits); we
        // trust it but still verify `plugin.id` matches the directory
        // — a mismatch would silently install plugin X under
        // /plugins/Y, breaking discovery.
        let bundle_manifest_path = bundle_dir.join("plugin.toml");
        let bundle_manifest = PluginManifest::load(&bundle_manifest_path).map_err(|err| {
            format!(
                "bundled plugin.toml at {} failed to parse: {err}",
                bundle_manifest_path.display()
            )
        })?;
        if bundle_manifest.plugin.id != id {
            return Err(format!(
                "bundled plugin.id `{}` does not match install directory `{}`; refusing promotion",
                bundle_manifest.plugin.id, id
            ));
        }
        if bundle_manifest.plugin.version != latest {
            // Not a hard error — the manifest_url may have
            // advertised a version that doesn't perfectly match
            // what the archive shipped. Log so operators can spot
            // publisher mismatches.
            warn!(
                plugin_id = %id,
                advertised = %latest,
                bundled = %bundle_manifest.plugin.version,
                "plugin auto-update: bundle version differs from manifest_url advertised version"
            );
        }

        // Step 7e — atomic promotion. Mirror `super::install`'s
        // pattern: remove existing install, then rename staging
        // bundle into place. Both live under the same filesystem
        // (install root) so the rename is a single inode swap.
        let dest = target_root.join(id);
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| {
                format!(
                    "removing old install at {} before atomic promotion: {e}",
                    dest.display()
                )
            })?;
        }
        std::fs::rename(&bundle_dir, &dest)
            .map_err(|e| format!("renaming bundle into {}: {e}", dest.display()))?;
    }

    // Step 8 — hot-swap the running subprocess. If this fails after
    // a successful promote, the next tick's `decide_action` will
    // take the `RespawnOnly` branch (disk == latest, running < latest)
    // and retry without re-downloading. The §9.4 respawn path itself
    // rolls back the in-memory manifest on spawn failure so
    // observers don't see a phantom version.
    let mut mgr = plugin_manager.lock().await;
    mgr.respawn_plugin_with_fresh_manifest(id)
        .await
        .map_err(|e| format!("hot-replace failed: {e}"))?;

    info!(
        plugin_id = %id,
        from = %running_version,
        to = %latest,
        "plugin auto-update: silently hot-replaced"
    );
    Ok(())
}

fn list_installed_plugin_ids(target_root: &Path) -> Result<Vec<String>, String> {
    if !target_root.exists() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<String> = std::fs::read_dir(target_root)
        .map_err(|e| format!("reading {}: {e}", target_root.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.join("plugin.toml").is_file() {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
            } else {
                None
            }
        })
        .collect();
    ids.sort();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Locks in the "promote succeeded, respawn failed, next tick"
    // contract. Without this, comparing only the disk version against
    // `latest` would short-circuit on the second tick — disk would
    // already be at `latest` from the prior promote, the running
    // subprocess would stay on the old image forever, and the only
    // recovery would be a manual gateway restart.

    #[test]
    fn noop_when_running_already_matches_latest() {
        // The fully-converged state. Disk and running both at latest.
        assert_eq!(
            decide_action("1.1.0", "1.1.0", "1.1.0"),
            AutoUpdateAction::NoOp,
        );
    }

    #[test]
    fn respawn_only_when_disk_already_promoted_but_running_stale() {
        // The retry case: a previous tick downloaded + promoted the
        // new bundle to disk, but the subsequent respawn failed
        // (spawn error, handshake timeout, etc.). The running
        // subprocess kept serving the OLD image. This tick must
        // notice the divergence and retry the respawn — without
        // burning bandwidth re-downloading the bundle that's already
        // sitting in the install root.
        assert_eq!(
            decide_action("1.0.0", "1.1.0", "1.1.0"),
            AutoUpdateAction::RespawnOnly,
        );
    }

    #[test]
    fn download_and_respawn_when_both_disk_and_running_stale() {
        // The common case: a brand-new version was just published
        // upstream, neither the disk bundle nor the running
        // subprocess has it yet. Full path: download + verify +
        // promote + respawn.
        assert_eq!(
            decide_action("1.0.0", "1.0.0", "1.1.0"),
            AutoUpdateAction::DownloadAndRespawn,
        );
    }

    #[test]
    fn download_when_disk_is_intermediate_version() {
        // Pathological but possible: a prior partial update left the
        // disk at an intermediate version (e.g. 1.0.5), with neither
        // the running subprocess (1.0.0) nor disk (1.0.5) matching
        // latest (1.1.0). Fall back to the full download path —
        // re-promoting from scratch is safer than trying to reuse
        // the half-installed bundle.
        assert_eq!(
            decide_action("1.0.0", "1.0.5", "1.1.0"),
            AutoUpdateAction::DownloadAndRespawn,
        );
    }
}

// End-to-end test for the "promote succeeds, respawn fails,
// next tick retries via RespawnOnly" path. Distinct from the
// pure `decide_action` matrix above because it exercises the
// full process_plugin wire-up: real HTTP server serves a real
// tarball, real subprocess plugin is registered, atomic promote
// happens on a temp filesystem, and a respawn failure is injected
// by a trigger file the new subprocess checks at initialize time.
//
// Gated on `python3` available + supported `current_target_alias`
// so the test skips cleanly on platforms that can't satisfy
// either prerequisite.
//
// All identifiers, hostnames, and URLs are deliberately synthetic
// (`auto-update-e2e-plugin`, `127.0.0.1:<random>`) so this test
// can never leak production names into an open-source repo.
#[cfg(test)]
mod e2e {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::{Router, body::Bytes, response::IntoResponse, routing::get};
    use garyx_channels::dispatcher::{ChannelDispatcherImpl, SwappableDispatcher};
    use garyx_channels::plugin::ChannelPluginManager;
    use garyx_channels::plugin_host::{
        AccountDescriptor, HostContext, InboundHandler, PluginManifest, SpawnOptions,
    };
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    use super::current_target_alias;

    const PLUGIN_ID: &str = "auto-update-e2e-plugin";

    fn python3_available() -> bool {
        Command::new("python3")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn fake_plugin_fixture_path() -> PathBuf {
        // Reuse the existing `fake_lifecycle_plugin.py` fixture from
        // garyx-channels' integration tests. It already exposes the
        // `FAKE_FAIL_INIT_IF_FILE` knob this test relies on — a path
        // that, when it exists at the moment `initialize` runs,
        // causes the subprocess to reject the lifecycle RPC with
        // ConfigRejected. That's exactly the §9.4 failure shape we
        // want to inject between tick 1 (which promotes) and tick 2
        // (which retries).
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("../garyx-channels/tests/fixtures/fake_lifecycle_plugin.py");
        p
    }

    fn host_ctx() -> HostContext {
        HostContext {
            version: "0.2.0-test".into(),
            public_url: "https://example.invalid".into(),
            data_dir: "/tmp/garyx-auto-update-e2e".into(),
            locale: Some("en".into()),
        }
    }

    struct NoopHandler;

    #[async_trait]
    impl InboundHandler for NoopHandler {
        async fn on_request(
            &self,
            _method: String,
            _params: Value,
        ) -> Result<Value, (i32, String)> {
            Err((-32601, "test host accepts no inbound requests".into()))
        }
        async fn on_notification(&self, _method: String, _params: Value) {}
    }

    /// Build a v2 plugin bundle as a gzipped tarball with the
    /// canonical `<id>/...` layout. The bundle's `plugin.toml`
    /// points its `entry.env.FAKE_FAIL_INIT_IF_FILE` at
    /// `trigger_file`, so the test can flip respawn outcomes by
    /// creating / removing that file between ticks.
    fn build_v2_bundle(
        plugin_id: &str,
        version: &str,
        trigger_file: &std::path::Path,
        manifest_url: &str,
        url_template: &str,
    ) -> Vec<u8> {
        let script = std::fs::read(fake_plugin_fixture_path()).expect("read fixture");
        let plugin_toml = format!(
            r#"
[plugin]
id = "{plugin_id}"
version = "{version}"
display_name = "Auto-Update E2E Plugin"

[entry]
binary = "./fake_lifecycle_plugin.py"

[entry.env]
FAKE_PLUGIN_ID = "{plugin_id}"
FAKE_FAIL_INIT_IF_FILE = "{trigger}"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = false
inbound = false
streaming = false
images = false
files = false
survives_respawn = true

[runtime]
stop_grace_ms = 1000
shutdown_grace_ms = 1000

[update]
manifest_url = "{manifest_url}"
url_template = "{url_template}"
binary_in_archive = "{{id}}/fake_lifecycle_plugin.py"

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
"#,
            trigger = trigger_file.display(),
        );

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut builder = tar::Builder::new(&mut gz);
            let mut script_hdr = tar::Header::new_gnu();
            script_hdr
                .set_path(format!("{plugin_id}/fake_lifecycle_plugin.py"))
                .unwrap();
            script_hdr.set_size(script.len() as u64);
            script_hdr.set_mode(0o755);
            script_hdr.set_cksum();
            builder
                .append(&script_hdr, std::io::Cursor::new(&script))
                .unwrap();

            let toml_bytes = plugin_toml.as_bytes();
            let mut toml_hdr = tar::Header::new_gnu();
            toml_hdr
                .set_path(format!("{plugin_id}/plugin.toml"))
                .unwrap();
            toml_hdr.set_size(toml_bytes.len() as u64);
            toml_hdr.set_mode(0o644);
            toml_hdr.set_cksum();
            builder
                .append(&toml_hdr, std::io::Cursor::new(toml_bytes))
                .unwrap();
            builder.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(bytes);
        let mut out = String::with_capacity(digest.len() * 2);
        for b in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{b:02x}");
        }
        out
    }

    /// Install a minimal v1 bundle on disk: script + plugin.toml
    /// declaring `version = 0.1.0` plus the `[update]` block that
    /// points at our test server. The plugin.toml here doesn't
    /// route through the test server's bundle endpoint — that's
    /// what the auto-updater will fetch FROM the server, then
    /// overwrite this on-disk plugin.toml with the v2 one.
    fn install_v1_bundle(
        install_root: &std::path::Path,
        trigger_file: &std::path::Path,
        manifest_url: &str,
        url_template: &str,
    ) -> PluginManifest {
        let dir = install_root.join(PLUGIN_ID);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("fake_lifecycle_plugin.py");
        std::fs::copy(fake_plugin_fixture_path(), &target).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&target).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&target, perms).unwrap();
        }

        let plugin_toml = format!(
            r#"
[plugin]
id = "{PLUGIN_ID}"
version = "0.1.0"
display_name = "Auto-Update E2E Plugin"

[entry]
binary = "./fake_lifecycle_plugin.py"

[entry.env]
FAKE_PLUGIN_ID = "{PLUGIN_ID}"
FAKE_FAIL_INIT_IF_FILE = "{trigger}"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = false
inbound = false
streaming = false
images = false
files = false
survives_respawn = true

[runtime]
stop_grace_ms = 1000
shutdown_grace_ms = 1000

[update]
manifest_url = "{manifest_url}"
url_template = "{url_template}"
binary_in_archive = "{{id}}/fake_lifecycle_plugin.py"

[schema]
"$schema" = "https://json-schema.org/draft/2020-12/schema"
type = "object"
"#,
            trigger = trigger_file.display(),
        );
        let manifest_path = dir.join("plugin.toml");
        std::fs::write(&manifest_path, plugin_toml).unwrap();
        PluginManifest::load(&manifest_path).unwrap()
    }

    #[tokio::test]
    async fn promote_then_respawn_failure_retries_on_next_tick() {
        if !python3_available() {
            eprintln!("skipping auto-update e2e: python3 not available");
            return;
        }
        if current_target_alias().is_err() {
            eprintln!("skipping auto-update e2e: unsupported target alias");
            return;
        }

        // Sandboxed install root + trigger file location, both
        // under tempdirs so the test never touches `~/.garyx`.
        let install_root = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let trigger_file = aux.path().join("fail-init-trigger");

        // Bind 127.0.0.1:0 once — kernel picks an ephemeral port.
        // Some CI sandboxes deny loopback bind outright (seccomp /
        // network namespace stripped); skip cleanly there instead
        // of failing the whole suite. A loopback-deny environment
        // also can't exercise the rest of this test (real HTTP
        // server is required), so there's nothing meaningful to
        // fall back to.
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(err) => {
                eprintln!(
                    "skipping auto-update e2e: 127.0.0.1 loopback bind \
                     denied by environment ({err})"
                );
                return;
            }
        };
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");

        let manifest_url = format!("{base}/latest.json");
        let url_template = format!("{base}/bundle.tar.gz");

        let v2_bundle = build_v2_bundle(
            PLUGIN_ID,
            "0.2.0",
            &trigger_file,
            &manifest_url,
            &url_template,
        );
        let v2_sha = sha256_hex(&v2_bundle);

        let server_app = Router::new()
            .route(
                "/latest.json",
                get(|| async { axum::Json(json!({"version": "0.2.0"})) }),
            )
            .route("/bundle.tar.gz", {
                let bytes = std::sync::Arc::new(v2_bundle.clone());
                get(move || {
                    let b = bytes.clone();
                    async move { Bytes::from(b.as_ref().clone()).into_response() }
                })
            })
            .route("/bundle.tar.gz.sha256", {
                let sha = std::sync::Arc::new(v2_sha.clone());
                get(move || {
                    let s = sha.clone();
                    async move { s.as_ref().clone() }
                })
            });
        tokio::spawn(async move {
            let _ = axum::serve(listener, server_app).await;
        });

        // Now install v1 of the plugin and register it with a real
        // ChannelPluginManager so the running version is observable
        // via `subprocess_plugin_version`.
        let v1_manifest = install_v1_bundle(
            install_root.path(),
            &trigger_file,
            &manifest_url,
            &url_template,
        );
        let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
        let mut manager = ChannelPluginManager::new();
        manager.attach_dispatcher(swap.clone());
        manager
            .register_subprocess_plugin(
                v1_manifest,
                SpawnOptions::default(),
                host_ctx(),
                vec![AccountDescriptor {
                    id: "acct-1".into(),
                    enabled: true,
                    config: json!({"token": "x"}),
                }],
                Arc::new(NoopHandler),
            )
            .await
            .expect("v1 should register cleanly");
        let manager = Arc::new(Mutex::new(manager));

        // Pre-tick sanity: running version equals what we just registered.
        {
            let mgr = manager.lock().await;
            assert_eq!(
                mgr.subprocess_plugin_version(PLUGIN_ID).as_deref(),
                Some("0.1.0"),
                "freshly-registered plugin should report v0.1.0"
            );
        }

        // Tick 1 setup: trigger file EXISTS, so when the v2
        // subprocess spawns and runs `initialize`, it sees the file
        // and returns ConfigRejected → §9.4 respawn fails on Step 1
        // (the OLD child keeps serving, the §9.4 path rolls back
        // the in-memory manifest).
        std::fs::write(&trigger_file, b"fail-please").unwrap();

        let tick1 = super::process_plugin(&manager, install_root.path(), PLUGIN_ID).await;
        assert!(
            tick1.is_err(),
            "tick 1 must fail at the respawn step: {tick1:?}"
        );
        let err = tick1.unwrap_err();
        assert!(
            err.contains("hot-replace failed"),
            "expected hot-replace failure surface, got: {err}"
        );

        // After tick 1 — disk is now v2 (atomic promote landed),
        // running stays v1 (respawn rolled back).
        let disk_after_tick1 =
            std::fs::read_to_string(install_root.path().join(PLUGIN_ID).join("plugin.toml"))
                .unwrap();
        assert!(
            disk_after_tick1.contains(r#"version = "0.2.0""#),
            "tick 1 must have promoted the new bundle to disk:\n{disk_after_tick1}"
        );
        {
            let mgr = manager.lock().await;
            assert_eq!(
                mgr.subprocess_plugin_version(PLUGIN_ID).as_deref(),
                Some("0.1.0"),
                "respawn failed → in-memory manifest rolled back to v0.1.0"
            );
        }

        // Tick 2 setup: remove trigger file so v2 init succeeds.
        std::fs::remove_file(&trigger_file).unwrap();

        let tick2 = super::process_plugin(&manager, install_root.path(), PLUGIN_ID).await;
        assert!(tick2.is_ok(), "tick 2 must succeed: {tick2:?}");

        // After tick 2 — running is now v2; disk also v2 (untouched
        // since tick 1 already promoted it). The Blocker 2 fix is
        // what makes this assertion meaningful: without
        // `manifest_to_metadata(&manifest)` in respawn_plugin, the
        // running adapter would still report 0.1.0 here even though
        // the subprocess + manifest both moved to v2.
        {
            let mgr = manager.lock().await;
            assert_eq!(
                mgr.subprocess_plugin_version(PLUGIN_ID).as_deref(),
                Some("0.2.0"),
                "RespawnOnly retry must lift running version to v2"
            );
        }

        // Cleanup: stop the live subprocess so the test process
        // doesn't leak a python child after the temp dirs are
        // dropped.
        let mut mgr = manager.lock().await;
        mgr.stop_all().await;
        mgr.cleanup_all().await;
        drop(mgr);

        // Give the OS a beat to flush before TempDir drops, otherwise
        // on slow CI we can race the python child's final write.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
