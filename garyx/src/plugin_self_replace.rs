//! Host-side handler for the `request_self_replace` plugin→host RPC
//! (Architecture C — plugin owns the upgrade timer + source, host
//! owns the safe-swap primitives).
//!
//! The plugin polls its own update server on its own schedule. When
//! it decides to upgrade, it calls `request_self_replace { archive_url,
//! expected_sha256, version, request_id }` over the existing
//! plugin↔host RPC transport. This module:
//!
//!  1. (`B1`) per-plugin single-flight: refuses a concurrent retry
//!     for the same plugin while a swap is in progress, returning
//!     `{ decision: "in_progress", active_request_id }`.
//!  2. (`B2`) sets a "swap barrier" on the plugin's
//!     `HostInboundHandler` so any new `deliver_inbound` arriving
//!     during the critical section is rejected with a
//!     `plugin_swapping` error code (`retry_after_ms` hint). The
//!     barrier is set *before* the stream-idle gate begins and
//!     cleared on every exit path (success / refusal / failure /
//!     panic via `Drop`).
//!  3. (`B3`) does NOT expose a way to bypass the stream-idle gate.
//!     The CLI escape hatch (`garyx plugins update <name>
//!     --break-active-streams`) goes through a separate code path,
//!     not this RPC handler.
//!  4. (`B4`) `caller_plugin_id` is taken from the transport context
//!     (the manager-registered handler's plugin_id) — never from
//!     params, so a plugin can't spoof another plugin's id. After
//!     download the archived `plugin.toml` is parsed and its
//!     `plugin.id` AND `plugin.version` must match the caller and
//!     the request params respectively; otherwise the swap is
//!     refused as `id_mismatch` / `version_mismatch`. The archive
//!     is extracted into a tempdir, never directly over an arbitrary
//!     path, and the atomic rename target is scoped to the caller's
//!     own install dir.
//!
//! Decision taxonomy (response body):
//!
//!   ```jsonc
//!   { "decision": "applied",        "from_version", "to_version", "elapsed_ms" }
//!   { "decision": "refused",        "reason": one_of(
//!         "downgrade",              // installed >= advertised
//!         "already_current",        // installed == advertised exactly
//!         "master_disabled",        // plugins.auto_update = false
//!         "no_survives_respawn",    // plugin didn't opt in
//!         "id_mismatch",            // archive's plugin.toml.id != caller
//!         "version_mismatch",       // archive's plugin.toml.version != params.version
//!         "invalid_params",
//!         "plugin_not_registered"
//!     ), "detail" }
//!   { "decision": "deferred",       "reason": "stream_active", "waited_secs", "max_wait_secs" }
//!   { "decision": "swap_failed",    "stage": "download"|"sha256"|"extract"|"manifest"|"promote"|"respawn",
//!                                   "error" }
//!   { "decision": "in_progress",    "active_request_id" }
//!   ```
//!
//! In the `applied` path, `respawn_plugin_with_fresh_manifest` kills
//! the caller's plugin process as part of the swap; the RPC response
//! is written into a transport whose remote endpoint is dead, so the
//! payload is observable only in host-side tracing. All non-applied
//! decisions DO make it back to the caller.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use garyx_channels::plugin::{ChannelPluginManager, SubprocessPluginInstallSnapshot};
use garyx_channels::plugin_host::PluginManifest;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::auto_update_common::{
    IdleGateConfig, IdleWaitError, should_upgrade, wait_for_stream_idle,
};

/// How long the host suggests the plugin should wait before retrying
/// after hitting the swap barrier. Sized to comfortably outlast the
/// stream-idle gate poll + a typical small swap (~10–30s). The plugin
/// SDK is free to ignore this and back off independently.
const SWAP_BARRIER_RETRY_AFTER_MS: u64 = 15_000;

const DEFAULT_IDLE_REQUIRED_SECS: u64 = 60;
const DEFAULT_IDLE_POLL_SECS: u64 = 5;
const DEFAULT_IDLE_MAX_WAIT_SECS: u64 = 24 * 60 * 60;

/// Per-`HostInboundHandler` state that survives across RPC calls. One
/// instance per registered subprocess plugin. Held by the
/// `HostInboundHandler` itself (channel_plugin_host.rs).
#[derive(Default)]
pub struct SelfReplaceState {
    /// (B1) per-plugin single-flight latch. `Some(request_id)` when a
    /// swap is currently in progress; concurrent calls return
    /// `in_progress` without touching state. Cleared on every exit
    /// path via the `InFlightGuard` Drop impl.
    in_flight: StdMutex<Option<String>>,
    /// (B2) swap barrier. `true` between just-before-idle-gate and
    /// swap completion; `deliver_inbound` checks this and rejects
    /// during the critical section. Always cleared by the
    /// `InFlightGuard` Drop impl so a panic mid-swap doesn't leave
    /// the plugin permanently barricaded.
    swap_barrier_active: AtomicBool,
}

impl SelfReplaceState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Callers (deliver_inbound entry) use this to decide whether
    /// to short-circuit an incoming dispatch. Acquire is paired with
    /// the Release in `InFlightGuard::drop`.
    pub fn is_swapping(&self) -> bool {
        self.swap_barrier_active.load(Ordering::Acquire)
    }

    /// Static — what `deliver_inbound` should suggest as `retry_after_ms`
    /// when it rejects.
    pub fn swap_barrier_retry_after_ms() -> u64 {
        SWAP_BARRIER_RETRY_AFTER_MS
    }
}

#[derive(Debug, Deserialize)]
struct RequestSelfReplaceParams {
    archive_url: String,
    expected_sha256: String,
    version: String,
    request_id: String,
    #[serde(default)]
    #[allow(dead_code)] // host detects target itself; field reserved for cross-build cases
    target: Option<String>,
}

/// Entry point. Called by `HostInboundHandler::on_request` when method
/// is `"request_self_replace"`. Never panics; every failure is mapped
/// to a decision body.
pub async fn handle(
    caller_plugin_id: &str,
    state: &Arc<SelfReplaceState>,
    plugin_manager: Weak<Mutex<ChannelPluginManager>>,
    master_enabled: bool,
    params: Value,
) -> Value {
    let started = Instant::now();
    let parsed = match serde_json::from_value::<RequestSelfReplaceParams>(params) {
        Ok(p) => p,
        Err(err) => return refused("invalid_params", err.to_string()),
    };

    // (B1) Single-flight: hold the latch for the rest of the call.
    {
        let mut guard = state.in_flight.lock().expect("in_flight poisoned");
        if let Some(active) = guard.as_ref() {
            return json!({
                "decision": "in_progress",
                "active_request_id": active,
            });
        }
        *guard = Some(parsed.request_id.clone());
    }
    let _flight_guard = InFlightGuard::new(state);

    // Master kill switch (gated at the RPC layer, not at spawn time).
    if !master_enabled {
        return refused("master_disabled", String::new());
    }

    // Resolve the manager. If gateway is shutting down, the Weak
    // can't upgrade — bail without touching anything.
    let Some(manager) = plugin_manager.upgrade() else {
        warn!(
            plugin_id = %caller_plugin_id,
            "request_self_replace: plugin_manager dropped; host is shutting down"
        );
        return swap_failed("host_shutdown", "manager weakref expired".to_owned(), "init");
    };

    // (B4) caller_plugin_id is from the transport, not params. Read
    // the installed snapshot under the manager lock, then release.
    let snapshot: SubprocessPluginInstallSnapshot = {
        let guard = manager.lock().await;
        match guard.subprocess_plugin_install_snapshot(caller_plugin_id) {
            Some(s) => s,
            None => {
                return refused(
                    "plugin_not_registered",
                    format!("caller_plugin_id={caller_plugin_id}"),
                );
            }
        }
    };

    if !snapshot.survives_respawn {
        return refused(
            "no_survives_respawn",
            "plugin.toml [capabilities] survives_respawn = false".to_owned(),
        );
    }

    if snapshot.version == parsed.version {
        return refused("already_current", format!("v{}", snapshot.version));
    }
    if !should_upgrade(&snapshot.version, &parsed.version) {
        return refused(
            "downgrade",
            format!(
                "installed v{} >= advertised v{}",
                snapshot.version, parsed.version
            ),
        );
    }

    info!(
        plugin_id = %caller_plugin_id,
        request_id = %parsed.request_id,
        from_version = %snapshot.version,
        to_version = %parsed.version,
        archive_url = %parsed.archive_url,
        "request_self_replace: accepted; downloading"
    );

    // Download. Off-thread the sha256 hash so we don't block the
    // tokio thread on a large archive.
    let client = match reqwest::Client::builder()
        .user_agent(format!("garyx-host/{}", crate::commands::VERSION))
        .build()
    {
        Ok(c) => c,
        Err(err) => return swap_failed("download_client_init", err.to_string(), "download"),
    };
    let archive_bytes = match download_archive(&client, &parsed.archive_url).await {
        Ok(b) => b,
        Err(err) => return swap_failed("download", err, "download"),
    };

    // Verify sha256 BEFORE touching disk or running tar.
    let bytes_for_hash = archive_bytes.clone();
    let expected_sha = parsed.expected_sha256.clone();
    let actual_sha = tokio::task::spawn_blocking(move || hex_sha256(&bytes_for_hash))
        .await
        .unwrap_or_default();
    if actual_sha != expected_sha {
        return swap_failed(
            "sha256_mismatch",
            format!("expected={expected_sha} actual={actual_sha}"),
            "sha256",
        );
    }

    // Extract into a tempdir scoped to this swap. Tempdir auto-cleans
    // on drop, including on every error return below.
    let tempdir = match tempfile::tempdir() {
        Ok(t) => t,
        Err(err) => return swap_failed("tempdir", err.to_string(), "extract"),
    };
    let bytes_for_extract = archive_bytes;
    let tempdir_path = tempdir.path().to_path_buf();
    if let Err(err) =
        tokio::task::spawn_blocking(move || extract_tar_gz(&bytes_for_extract, &tempdir_path))
            .await
            .unwrap_or_else(|join_err| Err(format!("extract task panicked: {join_err}")))
    {
        return swap_failed("extract", err, "extract");
    }

    // (B4) Find the archived plugin.toml + validate id/version match
    // the caller's identity and the request params.
    let archived_manifest_path = match find_plugin_toml(tempdir.path()) {
        Some(p) => p,
        None => {
            return swap_failed(
                "plugin_toml_missing",
                format!(
                    "no plugin.toml found in extracted archive at {}",
                    tempdir.path().display()
                ),
                "manifest",
            );
        }
    };
    let archived_manifest = match PluginManifest::load(&archived_manifest_path) {
        Ok(m) => m,
        Err(err) => {
            return swap_failed(
                "plugin_toml_parse",
                format!("{}: {err}", archived_manifest_path.display()),
                "manifest",
            );
        }
    };
    if archived_manifest.plugin.id != caller_plugin_id {
        return refused(
            "id_mismatch",
            format!(
                "archive plugin.toml id={} but caller is {}",
                archived_manifest.plugin.id, caller_plugin_id
            ),
        );
    }
    if archived_manifest.plugin.version != parsed.version {
        return refused(
            "version_mismatch",
            format!(
                "archive plugin.toml version={} but params.version={}",
                archived_manifest.plugin.version, parsed.version
            ),
        );
    }

    // The new binary path inside the extracted tree.
    let archived_binary_path = archived_manifest
        .manifest_dir
        .join(&archived_manifest.entry.binary);
    if !archived_binary_path.is_file() {
        return swap_failed(
            "binary_missing",
            format!(
                "expected new binary at {}",
                archived_binary_path.display()
            ),
            "manifest",
        );
    }

    // The on-disk install location for the caller's binary. We
    // explicitly use the installed snapshot's path, NOT anything
    // derivable from the archive — so the swap can only ever target
    // the caller's own install dir (B4).
    let install_binary_path = snapshot.manifest_dir.join(&snapshot.binary_relative);

    // (B2) Set the swap barrier. From here until `_flight_guard`
    // drops, any incoming `deliver_inbound` for this plugin is
    // rejected at the transport layer.
    state.swap_barrier_active.store(true, Ordering::Release);

    // Stream-idle gate. With the barrier above, no NEW streams can
    // sneak in; this gate drains whatever is already in flight.
    let idle_config = IdleGateConfig {
        required_idle_secs: DEFAULT_IDLE_REQUIRED_SECS,
        poll_interval_secs: DEFAULT_IDLE_POLL_SECS,
        max_wait_secs: DEFAULT_IDLE_MAX_WAIT_SECS,
    };
    if let Err(IdleWaitError::Timeout {
        waited_secs,
        max_wait_secs,
    }) = wait_for_stream_idle(&manager, idle_config).await
    {
        return deferred("stream_active", waited_secs, max_wait_secs);
    }

    // Promote the new binary atomically. We rename a staged tmp file
    // (in the install dir, so rename is same-filesystem-atomic) over
    // the live binary path.
    if let Err(err) = atomic_promote(&archived_binary_path, &install_binary_path).await {
        return swap_failed("promote", err, "promote");
    }
    // Also copy the archived plugin.toml over the installed one so
    // the manifest reflects the new version. Same-fs atomic rename.
    let install_manifest_path = snapshot.manifest_dir.join("plugin.toml");
    if let Err(err) = atomic_promote(&archived_manifest_path, &install_manifest_path).await {
        return swap_failed("promote_manifest", err, "promote");
    }

    // Respawn through the existing §9.4 path. This kills the caller's
    // plugin process; the RPC response below never reaches it.
    let plugin_id_owned = caller_plugin_id.to_owned();
    let respawn_result = {
        let mut guard = manager.lock().await;
        guard.respawn_plugin_with_fresh_manifest(&plugin_id_owned).await
    };
    if let Err(err) = respawn_result {
        return swap_failed("respawn", err.to_string(), "respawn");
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(
        plugin_id = %caller_plugin_id,
        request_id = %parsed.request_id,
        from_version = %snapshot.version,
        to_version = %parsed.version,
        elapsed_ms = elapsed_ms,
        "request_self_replace: applied; caller process is being torn down by respawn"
    );
    json!({
        "decision": "applied",
        "from_version": snapshot.version,
        "to_version": parsed.version,
        "elapsed_ms": elapsed_ms,
    })
}

/// RAII guard that clears the single-flight latch + the swap barrier
/// on every exit path, including panics. Without this a panic between
/// barrier-set and respawn would leave the plugin permanently
/// barricaded from receiving any new dispatch.
struct InFlightGuard<'a>(&'a SelfReplaceState);

impl<'a> InFlightGuard<'a> {
    fn new(state: &'a SelfReplaceState) -> Self {
        Self(state)
    }
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        *self.0.in_flight.lock().expect("in_flight poisoned") = None;
        self.0.swap_barrier_active.store(false, Ordering::Release);
    }
}

// ----- helpers -----------------------------------------------------

async fn download_archive(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| format!("send: {e}"))?
        .error_for_status()
        .map_err(|e| format!("status: {e}"))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("body: {e}"))?;
    Ok(bytes.to_vec())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    use std::fmt::Write as _;
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn extract_tar_gz(bytes: &[u8], dest: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar::Archive;
    let decoder = GzDecoder::new(std::io::Cursor::new(bytes));
    let mut archive = Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| format!("unpack into {}: {e}", dest.display()))
}

/// Walk the extracted archive looking for any `plugin.toml`. Caller
/// (B4) validates `[plugin].id` against the registered plugin_id, so
/// we accept whatever layout the archive has as long as exactly one
/// plugin.toml shows up.
fn find_plugin_toml(root: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.file_name().and_then(|s| s.to_str()) == Some("plugin.toml") {
                    out.push(path);
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    if found.len() == 1 { found.pop() } else { None }
}

async fn atomic_promote(src: &Path, dest: &Path) -> Result<(), String> {
    let dest_parent = dest
        .parent()
        .ok_or_else(|| format!("dest has no parent: {}", dest.display()))?
        .to_path_buf();
    let dest_filename = dest
        .file_name()
        .ok_or_else(|| format!("dest has no filename: {}", dest.display()))?
        .to_os_string();
    // Stage in the dest's parent so rename is same-filesystem-atomic.
    let staged = dest_parent.join(format!(
        ".{}.tmp.{}",
        dest_filename.to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));
    let src = src.to_path_buf();
    let staged_for_blocking = staged.clone();
    let dest_for_blocking = dest.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        std::fs::copy(&src, &staged_for_blocking)
            .map_err(|e| format!("copy {} -> {}: {e}", src.display(), staged_for_blocking.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&staged_for_blocking) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(&staged_for_blocking, perms);
            }
        }
        std::fs::rename(&staged_for_blocking, &dest_for_blocking).map_err(|e| {
            format!(
                "rename {} -> {}: {e}",
                staged_for_blocking.display(),
                dest_for_blocking.display()
            )
        })
    })
    .await
    .map_err(|e| format!("promote task panicked: {e}"))?
}

// ----- decision builders ------------------------------------------

fn refused(reason: &str, detail: String) -> Value {
    json!({ "decision": "refused", "reason": reason, "detail": detail })
}
fn deferred(reason: &str, waited_secs: u64, max_wait_secs: u64) -> Value {
    json!({
        "decision": "deferred",
        "reason": reason,
        "waited_secs": waited_secs,
        "max_wait_secs": max_wait_secs,
    })
}
fn swap_failed(reason: &str, error: String, stage: &str) -> Value {
    json!({
        "decision": "swap_failed",
        "reason": reason,
        "stage": stage,
        "error": error,
    })
}

// ----- tests -------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_sha256_matches_known_vector() {
        // SHA-256 of empty input
        assert_eq!(
            hex_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[tokio::test]
    async fn single_flight_blocks_concurrent_call() {
        let state = SelfReplaceState::new();
        // First call enters the latch.
        {
            let mut guard = state.in_flight.lock().unwrap();
            *guard = Some("first".to_owned());
        }
        // Second call observes the latch and returns in_progress.
        let body = json!({
            "archive_url": "https://example.test/x.tar.gz",
            "expected_sha256": "deadbeef",
            "version": "9.9.9",
            "request_id": "second",
        });
        let weak: Weak<Mutex<ChannelPluginManager>> = Weak::new();
        let response = handle("test-plugin", &state, weak, true, body).await;
        assert_eq!(response["decision"], "in_progress");
        assert_eq!(response["active_request_id"], "first");
    }

    #[test]
    fn in_flight_guard_clears_state_on_drop() {
        let state = SelfReplaceState::new();
        *state.in_flight.lock().unwrap() = Some("req-1".to_owned());
        state.swap_barrier_active.store(true, Ordering::Release);
        {
            let _guard = InFlightGuard::new(&state);
        }
        assert!(state.in_flight.lock().unwrap().is_none());
        assert!(!state.is_swapping());
    }

    #[test]
    fn find_plugin_toml_walks_one_level() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("minolab");
        std::fs::create_dir(&inner).unwrap();
        let target = inner.join("plugin.toml");
        std::fs::write(&target, "[plugin]\nid=\"minolab\"\nversion=\"0.0.1\"\n").unwrap();
        assert_eq!(find_plugin_toml(tmp.path()), Some(target));
    }

    #[test]
    fn find_plugin_toml_returns_none_when_two() {
        let tmp = tempfile::tempdir().unwrap();
        for dir in ["a", "b"] {
            let sub = tmp.path().join(dir);
            std::fs::create_dir(&sub).unwrap();
            std::fs::write(sub.join("plugin.toml"), "").unwrap();
        }
        assert!(find_plugin_toml(tmp.path()).is_none());
    }

    fn valid_params_body() -> Value {
        json!({
            "archive_url": "https://example.test/x.tar.gz",
            "expected_sha256": "deadbeef",
            "version": "9.9.9",
            "request_id": "req-test",
        })
    }

    #[tokio::test]
    async fn handle_refuses_when_master_disabled() {
        // master switch off → refusal before any network or
        // filesystem activity. The handler MUST NOT touch the
        // manager weakref or attempt a download in this branch,
        // so a never-upgradable Weak is fine here.
        let state = SelfReplaceState::new();
        let weak: Weak<Mutex<ChannelPluginManager>> = Weak::new();
        let response = handle("test-plugin", &state, weak, false, valid_params_body()).await;
        assert_eq!(response["decision"], "refused");
        assert_eq!(response["reason"], "master_disabled");
    }

    #[tokio::test]
    async fn handle_refuses_when_params_malformed() {
        // serde failure happens before single-flight latch is
        // acquired, so the state stays untouched and a follow-up
        // call with valid params can still acquire the latch.
        let state = SelfReplaceState::new();
        let weak: Weak<Mutex<ChannelPluginManager>> = Weak::new();
        let response = handle(
            "test-plugin",
            &state,
            weak,
            true,
            json!({ "garbage": 1 }),
        )
        .await;
        assert_eq!(response["decision"], "refused");
        assert_eq!(response["reason"], "invalid_params");
        assert!(state.in_flight.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn handle_swap_failed_when_manager_weak_expired() {
        // Manager weak doesn't upgrade — host is shutting down or
        // wasn't wired correctly. Should NOT panic; report
        // host_shutdown so the calling plugin retries next tick.
        // master switch must be true to get past the kill switch.
        let state = SelfReplaceState::new();
        let weak: Weak<Mutex<ChannelPluginManager>> = Weak::new();
        let response = handle("test-plugin", &state, weak, true, valid_params_body()).await;
        assert_eq!(response["decision"], "swap_failed");
        assert_eq!(response["reason"], "host_shutdown");
    }

    #[tokio::test]
    async fn handle_refuses_plugin_not_registered() {
        // Manager exists but doesn't have an entry for
        // "ghost-plugin" — caller_plugin_id should be defended:
        // refusing here means the swap path can't be tricked into
        // looking up another plugin's install root.
        use garyx_channels::plugin::ChannelPluginManager;
        let manager = Arc::new(Mutex::new(ChannelPluginManager::new()));
        let state = SelfReplaceState::new();
        let weak = Arc::downgrade(&manager);
        let response =
            handle("ghost-plugin", &state, weak, true, valid_params_body()).await;
        assert_eq!(response["decision"], "refused");
        assert_eq!(response["reason"], "plugin_not_registered");
    }
}
