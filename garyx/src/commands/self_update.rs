use super::*;

const GITHUB_RELEASE_REPO: &str = "Pyiner/garyx";

/// Re-exported under a `_DEFAULT` name so callers (the gateway
/// auto-update loop) can fall back to it when the config field is
/// blank. Keeping the const private and exposing the alias keeps
/// the surface explicit about which name is intended for fallback
/// vs internal use.
pub(crate) const GITHUB_RELEASE_REPO_DEFAULT: &str = GITHUB_RELEASE_REPO;

#[cfg(any(target_os = "macos", test))]
const MACOS_CLI_CODESIGN_IDENTIFIER: &str = "com.garyx.gateway";

const MACOS_CCTTY_CODESIGN_IDENTIFIER: &str = "com.garyx.cctty";

pub(super) const DEFAULT_CHANNEL_AGENT_ID: &str = "claude";

#[derive(Debug, Deserialize)]
struct GitHubReleaseSummary {
    tag_name: String,
}

fn normalize_release_version(value: &str) -> String {
    value.trim().trim_start_matches('v').to_owned()
}

/// Hard cap on the staged-binary version probe (B1). Auto-update must
/// never stall the gateway (`gateway_auto_update.rs` contract), so a
/// staged binary that hangs on `--version` is treated as a probe
/// failure rather than blocking the swap path forever.
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Typed failure of the pre-rename version verification (B1). Both
/// variants mean "do NOT swap": the caller returns `Err` and the
/// gateway loop takes its warn-and-retry branch instead of `exit(0)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SwapError {
    /// Staged binary self-reported a version that is not exactly the
    /// requested release tag. `==` (not `>=`) on purpose: the goal is
    /// "binary self-reports the tag it was published under"; a `>=`
    /// check would wave through a mis-tagged release.
    VersionMismatch { measured: String, expected: String },
    /// Probe could not produce a usable version string: timeout,
    /// nonzero exit, or empty/garbled stdout. Folded into the same
    /// "refuse to swap" outcome as a mismatch.
    ProbeFailed { reason: String },
}

impl std::fmt::Display for SwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwapError::VersionMismatch { measured, expected } => write!(
                f,
                "staged binary self-reported version {measured} != requested {expected}"
            ),
            SwapError::ProbeFailed { reason } => {
                write!(f, "staged binary version probe failed: {reason}")
            }
        }
    }
}

impl std::error::Error for SwapError {}

/// Extract the version token from `garyx --version` output.
///
/// The root command prints `garyx <version>` (clap default for
/// `#[command(name = "garyx", version = ...)]`, matched by the B0
/// short-circuit in `main.rs`). We take the last whitespace-separated
/// token of the first non-empty line and strip any leading `v` so the
/// comparison is apples-to-apples with the normalized requested tag.
/// Returns `None` for empty / malformed output.
fn parse_self_reported_version(stdout: &str) -> Option<String> {
    let line = stdout.lines().find(|l| !l.trim().is_empty())?;
    let token = line.split_whitespace().last()?;
    let normalized = normalize_release_version(token);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Probe the staged binary, verify its self-reported version, and on
/// ANY rejection delete the staged file before returning the typed
/// error (B1 cleanup).
///
/// `staged_path` has already been `fs::copy`'d into the install
/// directory by the time we get here, so a bare `?` propagation on a
/// bad release would leave a `.garyx-update-*.tmp` orphan in the
/// install dir on every auto-update tick — a slow disk leak. We instead
/// remove the staged file on the reject path (best-effort: cleanup
/// errors are swallowed since the caller already failed and will retry).
/// On success the file stays so the caller can `fs::rename` it into
/// place.
async fn probe_and_verify_staged_version(
    staged_path: &Path,
    requested_version: &str,
) -> Result<String, SwapError> {
    // A probe failure is already a `SwapError`; on success run the
    // exact-match verify. Either way this collapses to one typed result.
    let result = match probe_staged_binary_version(staged_path).await {
        Ok(measured) => verify_staged_version(Some(&measured), requested_version),
        Err(probe_err) => Err(probe_err),
    };

    if result.is_err() {
        // Best-effort cleanup so a bad release does not leak a staged
        // temp file into the install dir on every retry tick. The caller
        // already failed and will retry; a cleanup error is non-fatal.
        let _ = fs::remove_file(staged_path);
    }
    result
}

/// Pure decision point for B1: given the version a staged binary
/// self-reported (if any) and the requested tag, decide whether the
/// swap may proceed. Extracted so the accept/reject logic is unit
/// testable without spawning a process.
fn verify_staged_version(
    measured: Option<&str>,
    requested_version: &str,
) -> Result<String, SwapError> {
    match measured {
        Some(measured) if measured == requested_version => Ok(measured.to_owned()),
        Some(measured) => Err(SwapError::VersionMismatch {
            measured: measured.to_owned(),
            expected: requested_version.to_owned(),
        }),
        None => Err(SwapError::ProbeFailed {
            reason: "empty or malformed --version output".to_owned(),
        }),
    }
}

/// Run the staged binary's `--version` and return its self-reported
/// version, the side-effect-free way (B1).
///
/// * `staged_path` is absolute (UUID-named temp, already `0755`) to
///   avoid PATH injection.
/// * Executes under an isolated `HOME` (+ defensive `USERPROFILE`)
///   pointing at a throwaway tempdir. `local_paths::home_dir()` reads
///   `$HOME` then `$USERPROFILE` — it does NOT consult `GARYX_HOME` or
///   `dirs::home_dir()` — so overriding `HOME` is sufficient to keep an
///   un-B0'd old binary's `migrate_legacy_homes()` away from the real
///   `~/.garyx`.
/// * Bounded by `VERSION_PROBE_TIMEOUT`; timeout / nonzero exit /
///   unusable stdout all collapse into `SwapError::ProbeFailed`.
async fn probe_staged_binary_version(staged_path: &Path) -> Result<String, SwapError> {
    probe_staged_binary_version_with_timeout(staged_path, VERSION_PROBE_TIMEOUT).await
}

/// Inner implementation of [`probe_staged_binary_version`] with an
/// injectable timeout so the timeout branch is unit-testable without a
/// 10s wait.
async fn probe_staged_binary_version_with_timeout(
    staged_path: &Path,
    timeout: Duration,
) -> Result<String, SwapError> {
    let probe_home = tempfile::tempdir().map_err(|e| SwapError::ProbeFailed {
        reason: format!("could not create isolated probe HOME: {e}"),
    })?;

    // Honor the "absolute staged path" contract before spawning: a
    // relative `--path` (e.g. `garyx update --path ./garyx`) would
    // otherwise let us spawn the wrong binary from the probe's working
    // directory. The staged temp was just created, so canonicalize
    // succeeds and also collapses any symlink ambiguity.
    let staged_path = staged_path
        .canonicalize()
        .map_err(|e| SwapError::ProbeFailed {
            reason: format!(
                "could not resolve staged binary to an absolute path ({}): {e}",
                staged_path.display()
            ),
        })?;

    let mut command = Command::new(&staged_path);
    command
        .arg("--version")
        .env("HOME", probe_home.path())
        .env("USERPROFILE", probe_home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = match tokio::time::timeout(timeout, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(SwapError::ProbeFailed {
                reason: format!("failed to spawn staged binary for version probe: {e}"),
            });
        }
        Err(_) => {
            return Err(SwapError::ProbeFailed {
                reason: format!("version probe timed out after {timeout:?}"),
            });
        }
    };

    if !output.status.success() {
        return Err(SwapError::ProbeFailed {
            reason: format!("staged binary --version exited with {}", output.status),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_self_reported_version(&stdout).ok_or_else(|| SwapError::ProbeFailed {
        reason: "empty or malformed --version output".to_owned(),
    })
}

fn detect_release_target_for(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        _ => Err(format!("unsupported platform for self-update: {os}/{arch}")),
    }
}

fn detect_release_target() -> Result<&'static str, Box<dyn std::error::Error>> {
    detect_release_target_for(std::env::consts::OS, std::env::consts::ARCH).map_err(|e| e.into())
}

fn parse_sha256_checksum(contents: &str) -> Result<String, Box<dyn std::error::Error>> {
    let checksum = contents
        .lines()
        .find_map(|line| line.split_whitespace().next())
        .filter(|value| !value.is_empty())
        .ok_or("checksum file is empty or malformed")?;
    Ok(checksum.to_owned())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

pub(crate) async fn latest_release_version(
    client: &reqwest::Client,
) -> Result<String, Box<dyn std::error::Error>> {
    let token = github_token_from_env();
    latest_release_version_for_repo(client, GITHUB_RELEASE_REPO, token.as_deref()).await
}

/// Variant of [`latest_release_version`] that lets the caller
/// override the GitHub `owner/repo`. The gateway auto-update loop
/// reads its repo from `garyx.json::gateway.auto_update.github_repo`
/// so operators can point at a fork for testing; the manual
/// `garyx update` path keeps the compile-time default.
///
/// `token` is an optional GitHub personal access token. When set the
/// request is bearer-authenticated, lifting the unauthenticated
/// 60 req/h IP-rate-limit to the per-token 5000 req/h budget. Read
/// from the `GARYX_GITHUB_TOKEN` env var rather than `garyx.json` so
/// the secret never lands on disk in the config file.
pub(crate) async fn latest_release_version_for_repo(
    client: &reqwest::Client,
    repo: &str,
    token: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut request = client.get(format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ));
    if let Some(value) = token {
        request = request.bearer_auth(value);
    }
    let summary = request
        .send()
        .await?
        .error_for_status()?
        .json::<GitHubReleaseSummary>()
        .await?;
    Ok(normalize_release_version(&summary.tag_name))
}

/// Read `GARYX_GITHUB_TOKEN` and return `Some` only when non-empty
/// after trim. Lifts the GitHub unauthenticated rate limit when set.
/// Lives outside the config struct on purpose — secrets in env vars,
/// not in `garyx.json`.
pub(crate) fn github_token_from_env() -> Option<String> {
    std::env::var("GARYX_GITHUB_TOKEN")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub(crate) fn replacement_binary_path(
    install_path: Option<PathBuf>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = install_path {
        return Ok(path);
    }
    Ok(std::env::current_exe()?)
}

#[cfg(any(target_os = "macos", test))]
fn macos_cli_codesign_args_with_identifier(
    binary_path: &Path,
    identifier: &str,
) -> Vec<std::ffi::OsString> {
    let mut args = vec![
        std::ffi::OsString::from("--force"),
        std::ffi::OsString::from("--sign"),
        std::ffi::OsString::from("-"),
        std::ffi::OsString::from("--identifier"),
        std::ffi::OsString::from(identifier),
    ];
    args.push(binary_path.as_os_str().to_os_string());
    args
}

#[cfg(any(target_os = "macos", test))]
#[cfg(test)]
fn macos_cli_codesign_args(binary_path: &Path) -> Vec<std::ffi::OsString> {
    macos_cli_codesign_args_with_identifier(binary_path, MACOS_CLI_CODESIGN_IDENTIFIER)
}

#[cfg(target_os = "macos")]
fn ad_hoc_codesign_macos_binary(binary_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    ad_hoc_codesign_macos_binary_with_identifier(binary_path, MACOS_CLI_CODESIGN_IDENTIFIER)
}

#[cfg(target_os = "macos")]
fn ad_hoc_codesign_macos_binary_with_identifier(
    binary_path: &Path,
    identifier: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .args(macos_cli_codesign_args_with_identifier(
            binary_path,
            identifier,
        ))
        .output()?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "codesign failed for {} with identifier {}: {}{}",
        binary_path.display(),
        MACOS_CLI_CODESIGN_IDENTIFIER,
        stdout,
        stderr
    )
    .into())
}

#[cfg(not(target_os = "macos"))]
fn ad_hoc_codesign_macos_binary(_binary_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn ad_hoc_codesign_macos_binary_with_identifier(
    _binary_path: &Path,
    _identifier: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// Outcome of [`try_swap_garyx_binary`] — the unattended sibling to
/// the user-facing `cmd_update` path. Used by both callers (manual
/// CLI + background auto-update loop) to log a "from→to" line after
/// a successful swap, and by the loop to decide whether to SIGTERM
/// self for restart.
#[derive(Debug)]
pub(crate) struct SwapOutcome {
    /// Version that was installed pre-swap.
    pub from_version: String,
    /// Version that was installed post-swap — the staged binary's
    /// MEASURED self-reported version (B1), which the pre-rename gate
    /// guarantees equals the requested tag.
    pub to_version: String,
    /// Final path of the installed binary on disk.
    pub install_path: PathBuf,
}

/// Download a specific garyx release from GitHub, verify it, codesign
/// it (macOS), and atomically swap it into `destination_path`. Used
/// by both `cmd_update` (manual CLI) and the gateway auto-update loop
/// (background tick). The function does NOT print anything — callers
/// log/print as appropriate for their context.
///
/// `requested_version` must already be normalized (no leading `v`).
/// `repo` is the GitHub `owner/repo` to download from; the gateway
/// loop passes `gateway.auto_update.github_repo` so fork-testing
/// retrieves both the tag AND the binary from the same fork (codex
/// review caught the asymmetry on landing — the "latest" lookup
/// honored the override but the asset download went to the const).
/// `destination_path` is where the new binary lands on success.
pub(crate) async fn try_swap_garyx_binary(
    requested_version: &str,
    repo: &str,
    destination_path: &Path,
) -> Result<SwapOutcome, Box<dyn std::error::Error>> {
    let target = detect_release_target()?;
    let parent = destination_path
        .parent()
        .ok_or_else(|| {
            format!(
                "update target has no parent directory: {}",
                destination_path.display()
            )
        })?
        .to_path_buf();

    let archive_name = format!("garyx-{requested_version}-{target}.tar.gz");
    let base_url = format!("https://github.com/{repo}/releases/download/v{requested_version}");

    let client = reqwest::Client::builder()
        .user_agent(format!("garyx-cli/{VERSION}"))
        .build()?;
    let archive_bytes = client
        .get(format!("{base_url}/{archive_name}"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let checksum_text = client
        .get(format!("{base_url}/{archive_name}.sha256"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let expected_sha = parse_sha256_checksum(&checksum_text)?;
    let actual_sha = sha256_hex(&archive_bytes);
    if expected_sha != actual_sha {
        return Err(format!(
            "download checksum mismatch for {archive_name}: expected {expected_sha}, got {actual_sha}"
        )
        .into());
    }

    let tempdir = tempfile::tempdir()?;
    let decoder = GzDecoder::new(std::io::Cursor::new(archive_bytes));
    let mut archive = Archive::new(decoder);
    archive.unpack(tempdir.path())?;

    let extracted_binary = tempdir
        .path()
        .join(format!("garyx-{requested_version}-{target}"))
        .join("garyx");
    let extracted_cctty = tempdir
        .path()
        .join(format!("garyx-{requested_version}-{target}"))
        .join("cctty");
    if !extracted_binary.is_file() {
        return Err(format!(
            "release archive did not contain expected binary at {}",
            extracted_binary.display()
        )
        .into());
    }

    fs::create_dir_all(&parent)?;
    let staged_path = parent.join(format!(".garyx-update-{}.tmp", Uuid::new_v4().simple()));
    fs::copy(&extracted_binary, &staged_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&staged_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&staged_path, perms)?;
    }
    ad_hoc_codesign_macos_binary(&staged_path)?;

    // B1: verify the staged binary self-reports EXACTLY the requested
    // tag BEFORE it lands. Guards against the "version loop" where a
    // mis-tagged release (tag advanced, `CARGO_PKG_VERSION` stale) is
    // forever judged "newer" by `should_upgrade`, swapped in, and then
    // re-detected as out of date — driving the gateway into an endless
    // swap + exit(0) + relaunch cycle. The probe runs against the
    // absolute, already-codesigned staged path under an isolated HOME
    // with a timeout, so it is PATH-injection-safe, side-effect-free,
    // and cannot stall the auto-update loop. On any mismatch / probe
    // failure we bail out via `Err` (no rename, no exit) — the caller's
    // loop then just warn-logs and retries next tick. The reject path
    // also deletes the staged temp file so a bad release does not leak a
    // `.garyx-update-*.tmp` into the install dir on every tick.
    let measured_version = probe_and_verify_staged_version(&staged_path, requested_version).await?;

    let cctty_destination = parent.join("cctty");
    let cctty_staged_path = if extracted_cctty.is_file() {
        let staged = parent.join(format!(".cctty-update-{}.tmp", Uuid::new_v4().simple()));
        fs::copy(&extracted_cctty, &staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&staged)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&staged, perms)?;
        }
        ad_hoc_codesign_macos_binary_with_identifier(&staged, MACOS_CCTTY_CODESIGN_IDENTIFIER)?;
        Some(staged)
    } else {
        None
    };
    fs::rename(&staged_path, destination_path)?;
    if let Some(staged) = cctty_staged_path {
        fs::rename(staged, cctty_destination)?;
    }

    Ok(SwapOutcome {
        from_version: VERSION.to_owned(),
        // B1: report the MEASURED self-reported version, not the
        // requested tag. Previously this echoed the request, which
        // produced a "swapped from X to Y" log line even when the new
        // binary still self-reported X — masking the very version loop
        // this fix targets. After the `verify_staged_version` gate
        // above, `measured_version == requested_version`, but we record
        // the measured value to keep the log honest at the source.
        to_version: measured_version,
        install_path: destination_path.to_path_buf(),
    })
}

pub(crate) async fn cmd_update(
    version: Option<String>,
    install_path: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .user_agent(format!("garyx-cli/{VERSION}"))
        .build()?;

    let requested_version = match version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(value) => normalize_release_version(value),
        None => latest_release_version(&client).await?,
    };
    let destination = replacement_binary_path(install_path)?;
    let target = detect_release_target()?;

    // Validate destination has a parent BEFORE the short-circuit so
    // `garyx update --path /nonexistent/dir/garyx` still surfaces the
    // missing-parent error even on the "already up to date" path.
    // Pre-refactor behavior was the same; codex review #4 caught the
    // accidental ordering change when this got pulled into a thin
    // wrapper around `try_swap_garyx_binary`.
    if destination.parent().is_none() {
        return Err(format!(
            "update target has no parent directory: {}",
            destination.display()
        )
        .into());
    }

    if version.is_none() && requested_version == VERSION {
        println!(
            "garyx is already up to date at v{} ({})",
            VERSION,
            destination.display()
        );
        return Ok(());
    }

    println!("Updating garyx to v{requested_version} for {target}...");
    let outcome =
        try_swap_garyx_binary(&requested_version, GITHUB_RELEASE_REPO, &destination).await?;
    println!(
        "Updated garyx from v{} to v{} at {}",
        outcome.from_version,
        outcome.to_version,
        outcome.install_path.display()
    );
    Ok(())
}

/// Print current auto-update state for `auto-update status`. Reads
/// the on-disk config (not the running gateway's in-memory state)
/// because the gateway may not be running, and a freshly-edited
/// config that hasn't been reloaded yet is what the user cares about.
pub(crate) async fn cmd_auto_update_status(
    config_path: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, Default::default())?;
    let gw = &loaded.config.gateway.auto_update;
    let plugins = &loaded.config.plugins;

    let latest = match reqwest::Client::builder()
        .user_agent(format!("garyx-cli/{VERSION}"))
        .build()
    {
        Ok(client) => latest_release_version(&client).await.ok(),
        Err(_) => None,
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "installed_version": VERSION,
                "latest_known_version": latest,
                "gateway": {
                    "enabled": gw.enabled,
                    "check_interval_secs": gw.check_interval_secs,
                    "github_repo": gw.github_repo,
                },
                "plugin": {
                    "enabled": plugins.auto_update,
                    "check_interval_secs": plugins.auto_update_check_interval_secs,
                },
            }))?
        );
    } else {
        println!("auto-update status (from {})", loaded.path.display());
        println!("  installed:   v{VERSION}");
        match latest.as_deref() {
            Some(v) => println!("  latest:      v{v}"),
            None => println!("  latest:      <fetch failed>"),
        }
        println!(
            "  gateway:     {} (every {}s, repo={})",
            if gw.enabled { "ENABLED" } else { "disabled" },
            gw.check_interval_secs,
            gw.github_repo,
        );
        println!(
            "  plugin:      {} (every {}s)",
            if plugins.auto_update {
                "ENABLED"
            } else {
                "disabled"
            },
            plugins.auto_update_check_interval_secs,
        );
    }
    Ok(())
}

/// Implementation shared by `cmd_auto_update_disable` and
/// `cmd_auto_update_enable`. `target_gateway` and `target_plugin`
/// describe which loops to touch (both true means "all"); `enabled`
/// is the new value. Returns the post-mutation tuple `(gateway,
/// plugin)` so the caller can print a sensible summary.
async fn set_auto_update_flags(
    config_path: &str,
    target_gateway: bool,
    target_plugin: bool,
    enabled: bool,
) -> Result<(bool, bool), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, Default::default())?;
    let resolved_config_path = loaded.path;
    let mut config = loaded.config;

    // No explicit target → touch both. Matches the help text on the
    // CLI subcommands.
    let (touch_gateway, touch_plugin) = if !target_gateway && !target_plugin {
        (true, true)
    } else {
        (target_gateway, target_plugin)
    };

    if touch_gateway {
        config.gateway.auto_update.enabled = enabled;
    }
    if touch_plugin {
        config.plugins.auto_update = enabled;
    }

    save_config_struct(&resolved_config_path, &config)?;
    notify_gateway_reload_quiet(&resolved_config_path).await;

    Ok((
        config.gateway.auto_update.enabled,
        config.plugins.auto_update,
    ))
}

pub(crate) async fn cmd_auto_update_disable(
    config_path: &str,
    gateway: bool,
    plugin: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (gw_after, plugin_after) =
        set_auto_update_flags(config_path, gateway, plugin, false).await?;
    println!(
        "auto-update updated: gateway={} plugin={}",
        if gw_after { "ENABLED" } else { "disabled" },
        if plugin_after { "ENABLED" } else { "disabled" },
    );
    Ok(())
}

pub(crate) async fn cmd_auto_update_enable(
    config_path: &str,
    gateway: bool,
    plugin: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (gw_after, plugin_after) =
        set_auto_update_flags(config_path, gateway, plugin, true).await?;
    println!(
        "auto-update updated: gateway={} plugin={}",
        if gw_after { "ENABLED" } else { "disabled" },
        if plugin_after { "ENABLED" } else { "disabled" },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::commands::test_support::*;
    use tempfile::tempdir;

    #[test]
    fn normalize_release_version_strips_leading_v() {
        assert_eq!(normalize_release_version("v0.1.6"), "0.1.6");
        assert_eq!(normalize_release_version("0.1.6"), "0.1.6");
        assert_eq!(normalize_release_version("  v1.2.3-rc.1  "), "1.2.3-rc.1");
    }

    #[test]
    fn detect_release_target_for_supported_platforms() {
        assert_eq!(
            detect_release_target_for("macos", "aarch64").expect("mac arm64 target"),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            detect_release_target_for("linux", "x86_64").expect("linux x64 target"),
            "x86_64-unknown-linux-gnu"
        );
        assert!(detect_release_target_for("windows", "x86_64").is_err());
    }

    #[test]
    fn macos_cli_codesign_args_use_stable_identifier() {
        let args = macos_cli_codesign_args(Path::new("/tmp/garyx"));
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            args,
            vec![
                "--force",
                "--sign",
                "-",
                "--identifier",
                "com.garyx.gateway",
                "/tmp/garyx"
            ]
        );
    }

    #[test]
    fn parse_sha256_checksum_accepts_standard_release_file() {
        let checksum = parse_sha256_checksum(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  garyx-0.1.6-aarch64-apple-darwin.tar.gz\n",
        )
        .expect("checksum");
        assert_eq!(
            checksum,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    // ---------------------------------------------------------------------------
    // B1: staged-binary version verification (pre-rename gate).
    // ---------------------------------------------------------------------------
    #[test]
    fn parse_self_reported_version_extracts_from_clap_output() {
        // `garyx <version>` is the clap default printed by the root command
        // and the B0 short-circuit. We take the trailing token of the first
        // non-empty line and strip any leading `v`.
        assert_eq!(
            parse_self_reported_version("garyx 0.1.32\n").as_deref(),
            Some("0.1.32")
        );
        assert_eq!(
            parse_self_reported_version("garyx v0.1.32").as_deref(),
            Some("0.1.32")
        );
        assert_eq!(
            parse_self_reported_version("garyx 0.1.33-rc.1\n").as_deref(),
            Some("0.1.33-rc.1")
        );
        // Leading blank line(s) skipped to the first real line.
        assert_eq!(
            parse_self_reported_version("\n\ngaryx 0.1.32\n").as_deref(),
            Some("0.1.32")
        );
    }

    #[test]
    fn parse_self_reported_version_rejects_empty_or_blank() {
        assert_eq!(parse_self_reported_version(""), None);
        assert_eq!(parse_self_reported_version("   \n\t\n"), None);
        // A lone `v` normalizes to empty and is rejected.
        assert_eq!(parse_self_reported_version("garyx v"), None);
    }

    #[test]
    fn verify_staged_version_accepts_exact_match() {
        assert_eq!(
            verify_staged_version(Some("0.1.32"), "0.1.32"),
            Ok("0.1.32".to_owned())
        );
    }

    #[test]
    fn verify_staged_version_rejects_mismatch_with_typed_error() {
        // The canonical "version loop" shape: requested tag advanced but
        // the staged binary still self-reports the old version.
        assert_eq!(
            verify_staged_version(Some("0.1.29"), "0.1.32"),
            Err(SwapError::VersionMismatch {
                measured: "0.1.29".to_owned(),
                expected: "0.1.32".to_owned(),
            })
        );
    }

    #[test]
    fn verify_staged_version_uses_exact_equality_not_greater_or_equal() {
        // A binary that self-reports a HIGHER version than the requested
        // tag must still be rejected: the contract is "self-reports the tag
        // it was published under", not ">=". This is the exact-match
        // prerelease guard (spec test 6).
        assert_eq!(
            verify_staged_version(Some("0.1.33"), "0.1.32"),
            Err(SwapError::VersionMismatch {
                measured: "0.1.33".to_owned(),
                expected: "0.1.32".to_owned(),
            })
        );
        // And prerelease exact-match passes.
        assert_eq!(
            verify_staged_version(Some("0.1.33-rc.1"), "0.1.33-rc.1"),
            Ok("0.1.33-rc.1".to_owned())
        );
        assert_eq!(
            verify_staged_version(Some("0.1.33"), "0.1.33-rc.1"),
            Err(SwapError::VersionMismatch {
                measured: "0.1.33".to_owned(),
                expected: "0.1.33-rc.1".to_owned(),
            })
        );
    }

    #[test]
    fn verify_staged_version_missing_measurement_is_probe_failure() {
        assert_eq!(
            verify_staged_version(None, "0.1.32"),
            Err(SwapError::ProbeFailed {
                reason: "empty or malformed --version output".to_owned(),
            })
        );
    }

    /// Write an executable fake "staged binary" shell script that prints
    /// `body` to stdout, exits with `exit_code`, and — to prove HOME
    /// isolation — creates `$HOME/.garyx/probe-marker` as a side effect
    /// before exiting. The marker lets a test assert the probe ran the
    /// binary under an isolated HOME rather than the caller's real home.
    #[cfg(unix)]
    fn write_fake_staged_binary(dir: &Path, body: &str, exit_code: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(format!(".garyx-update-{}.tmp", Uuid::new_v4().simple()));
        let script = format!(
            "#!/bin/sh\nmkdir -p \"$HOME/.garyx\"\n: > \"$HOME/.garyx/probe-marker\"\n{body}\nexit {exit_code}\n"
        );
        std::fs::write(&path, script).expect("write fake staged binary");
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod fake staged binary");
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_staged_binary_version_reads_self_reported_version() {
        let dir = tempdir().expect("tempdir");
        let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.32'", 0);

        let measured = probe_staged_binary_version(&staged)
            .await
            .expect("probe should succeed");
        assert_eq!(measured, "0.1.32");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_staged_binary_version_isolates_home() {
        // The probe must NOT touch the caller's real HOME. We point HOME at
        // a sentinel dir the fake binary is forbidden to write under (the
        // probe sets its own isolated HOME), then assert no `.garyx` shows
        // up there even though the fake binary tries to create one.
        let _guard = ENV_LOCK.lock().expect("env lock");
        let real_home = tempdir().expect("real home");
        let _home = ScopedEnvVar::set_path("HOME", real_home.path());

        let staged_dir = tempdir().expect("staged dir");
        let staged = write_fake_staged_binary(staged_dir.path(), "echo 'garyx 0.1.32'", 0);

        let measured = probe_staged_binary_version(&staged)
            .await
            .expect("probe should succeed");
        assert_eq!(measured, "0.1.32");

        // The fake binary creates `$HOME/.garyx/probe-marker`. If isolation
        // works, that landed in the probe's throwaway HOME (already dropped)
        // and NOT under the caller's HOME.
        assert!(
            !real_home.path().join(".garyx").exists(),
            "version probe leaked into the caller's HOME"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_staged_binary_version_nonzero_exit_is_probe_failure() {
        let dir = tempdir().expect("tempdir");
        let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.32'", 3);

        let err = probe_staged_binary_version(&staged)
            .await
            .expect_err("nonzero exit should fail the probe");
        assert!(matches!(err, SwapError::ProbeFailed { .. }), "got {err:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_staged_binary_version_empty_stdout_is_probe_failure() {
        let dir = tempdir().expect("tempdir");
        // Exit 0 but print nothing usable.
        let staged = write_fake_staged_binary(dir.path(), "true", 0);

        let err = probe_staged_binary_version(&staged)
            .await
            .expect_err("empty stdout should fail the probe");
        assert!(matches!(err, SwapError::ProbeFailed { .. }), "got {err:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_staged_binary_version_times_out() {
        // A binary that hangs must be killed by the timeout and surfaced as
        // a probe failure, never stalling the auto-update loop. We drive the
        // injectable-timeout inner fn with a tiny timeout so the test is
        // fast; the slow binary sleeps far longer than that.
        let dir = tempdir().expect("tempdir");
        let staged = write_fake_staged_binary(dir.path(), "sleep 30", 0);

        let err = probe_staged_binary_version_with_timeout(&staged, Duration::from_millis(150))
            .await
            .expect_err("hanging binary should fail the probe");
        assert!(matches!(err, SwapError::ProbeFailed { .. }), "got {err:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_and_verify_removes_staged_file_on_version_mismatch() {
        // The staged binary is already `fs::copy`'d into the install dir by
        // the time verification runs. A bad release (tag advanced, binary
        // self-reports the old version) must NOT leave a `.garyx-update-*`
        // orphan behind — otherwise every auto-update retry tick leaks one.
        let dir = tempdir().expect("tempdir");
        let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.29'", 0);
        assert!(
            staged.exists(),
            "fake staged binary should exist pre-verify"
        );

        let err = probe_and_verify_staged_version(&staged, "0.1.32")
            .await
            .expect_err("version mismatch should be rejected");
        assert!(
            matches!(err, SwapError::VersionMismatch { .. }),
            "got {err:?}"
        );
        assert!(
            !staged.exists(),
            "staged temp file must be cleaned up on the reject path"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_and_verify_removes_staged_file_on_probe_failure() {
        // Probe failures (nonzero exit here) take the same cleanup path.
        let dir = tempdir().expect("tempdir");
        let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.32'", 3);
        assert!(staged.exists());

        let err = probe_and_verify_staged_version(&staged, "0.1.32")
            .await
            .expect_err("probe failure should be rejected");
        assert!(matches!(err, SwapError::ProbeFailed { .. }), "got {err:?}");
        assert!(
            !staged.exists(),
            "staged temp file must be cleaned up on probe failure"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_and_verify_keeps_staged_file_on_success() {
        // On the happy path the staged file must SURVIVE so the caller can
        // `fs::rename` it into the install path.
        let dir = tempdir().expect("tempdir");
        let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.32'", 0);

        let measured = probe_and_verify_staged_version(&staged, "0.1.32")
            .await
            .expect("matching version should pass");
        assert_eq!(measured, "0.1.32");
        assert!(
            staged.exists(),
            "staged temp file must remain for the caller to rename on success"
        );
    }
}
