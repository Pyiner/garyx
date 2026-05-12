//! `garyx plugins update` implementation.
//!
//! Resolves a release source for an installed subprocess plugin,
//! downloads + sha256-verifies a tarball, then reuses the existing
//! `super::install` flow with `force=true` for atomic promotion.
//!
//! See `docs/superpowers/specs/2026-05-11-garyx-plugins-update-design.md`.

use garyx_channels::builtin_catalog::builtin_channel_descriptor;
use tracing::warn;

use crate::channel_plugin_host::default_plugin_install_root;

use super::{PluginsCliError, UpdateOptions};

/// Structured per-plugin result. `update_one` and `update_all` collect
/// these and decide whether to render them as JSON or text — keeping
/// printing out of the helpers means `update_all` can aggregate
/// outcomes without each iteration writing duplicate JSON objects to
/// stdout (the original bug this refactor addresses).
#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct UpdateOutcome {
    pub id: String,
    pub previous: Option<String>,
    pub next: Option<String>,
    /// One of: `updated`, `already_current`, `update_available`,
    /// `no_source`, `failed`. `&'static str` because each call site
    /// hard-codes the value at construction time.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl UpdateOutcome {
    fn already_current(id: &str, version: &str) -> Self {
        Self {
            id: id.to_string(),
            previous: Some(version.to_string()),
            next: Some(version.to_string()),
            status: "already_current",
            source: None,
            install_dir: None,
            elapsed_ms: None,
            error: None,
        }
    }

    fn check_report(id: &str, installed: &str, latest: &str, source: &str) -> Self {
        let status = if installed == latest {
            "already_current"
        } else {
            "update_available"
        };
        Self {
            id: id.to_string(),
            previous: Some(installed.to_string()),
            next: Some(latest.to_string()),
            status,
            source: Some(source.to_string()),
            install_dir: None,
            elapsed_ms: None,
            error: None,
        }
    }

    fn no_source(id: &str) -> Self {
        Self {
            id: id.to_string(),
            previous: None,
            next: None,
            status: "no_source",
            source: None,
            install_dir: None,
            elapsed_ms: None,
            error: None,
        }
    }
}

/// Render an `UpdateOutcome` to stdout. Single source of truth so the
/// JSON shape and the text shape never drift between code paths.
fn render_outcome(outcome: &UpdateOutcome, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(outcome).unwrap());
        return;
    }
    match outcome.status {
        "updated" => {
            let name = &outcome.id;
            let next = outcome.next.as_deref();
            match (outcome.previous.as_deref(), next) {
                (Some(prev), Some(next)) => {
                    println!("\nUpdated `{name}` from v{prev} to v{next}.");
                }
                (None, Some(next)) => {
                    println!("\nUpdated `{name}` to v{next}.");
                }
                (Some(prev), None) => {
                    println!("\nUpdated `{name}` from v{prev} from local bundle.");
                }
                (None, None) => {
                    println!("\nUpdated `{name}` from local bundle.");
                }
            }
            if let Some(source) = &outcome.source {
                println!("  source:       {source}");
            }
            if let Some(dir) = &outcome.install_dir {
                println!("  install root: {dir}");
            }
            if let Some(ms) = outcome.elapsed_ms {
                println!("  elapsed:      {:.2}s\n", (ms as f64) / 1000.0);
            } else {
                println!();
            }
            println!("Restart the gateway so the new binary is picked up:");
            println!("  garyx gateway restart");
        }
        "already_current" => {
            let name = &outcome.id;
            let version = outcome.previous.as_deref().unwrap_or("?");
            println!(
                "plugin `{name}` is already at version {version}. Pass --force to reinstall anyway.",
            );
        }
        "update_available" => {
            let name = &outcome.id;
            let installed = outcome.previous.as_deref().unwrap_or("?");
            let latest = outcome.next.as_deref().unwrap_or("?");
            let source = outcome.source.as_deref().unwrap_or("");
            println!("plugin `{name}`:");
            println!("  current: v{installed}");
            println!("  latest:  v{latest}");
            println!("  source:  {source}");
            println!("  status:  update_available");
        }
        "no_source" => {
            let name = &outcome.id;
            println!(
                "plugin `{name}` has no update source declared in its manifest and no built-in fallback.",
            );
        }
        "failed" => {
            let name = &outcome.id;
            let err = outcome.error.as_deref().unwrap_or("unknown error");
            eprintln!("  ! {name}: {err}");
        }
        other => {
            // Defensive — every constructor sets one of the strings
            // above. Surface anything unexpected rather than swallow it.
            eprintln!("plugin `{}`: unrecognized status `{}`", outcome.id, other);
        }
    }
}

/// Hardcoded update source. Mirrors the optional `[update]` block
/// in `plugin.toml`. The host-side built-in fallback table uses the
/// same shape so the renderer is shared.
#[derive(Debug, Clone)]
pub(super) struct UpdateSource {
    pub manifest_url: Option<&'static str>,
    pub url_template: &'static str,
    pub checksum_url_template: &'static str,
    pub binary_in_archive: &'static str,
}

/// **Transitional** fallback for plugins whose installed bundle predates
/// the `[update]` block. Lookup is by plugin id (no aliases).
///
/// This table is **not** a registry for new plugins. The long-term
/// answer is for the plugin author to declare an `[update]` block in
/// their `plugin.toml`; the host preserves it into the installed
/// manifest at install time, after which lookup never reaches here.
///
/// Each entry MUST carry an inline retirement condition. If this table
/// grows past a handful of entries, the host has drifted into curating
/// a plugin allow-list — reconsider before adding more. The
/// compile-time cap below enforces a hard ceiling.
pub(super) const BUILTIN_PLUGIN_UPDATE_SOURCES: &[(&str, UpdateSource)] = &[(
    // Retire once `example-plugin` ships an [update] block in its
    // plugin.toml and a subsequent `garyx plugins install` replaces
    // the legacy bundle.
    "example-plugin",
    UpdateSource {
        // Set to `Some("...")` once an upstream `latest.json` is
        // published for this plugin.
        manifest_url: None,
        url_template:
            "https://example.com/garyx/plugins/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz",
        checksum_url_template: "{url}.sha256",
        binary_in_archive: "{id}/garyx-plugin-{id}",
    },
)];

// Compile-time cap: fallback is for existing installs, not a permanent
// plugin registry. Bumping this triggers a deliberate design review.
const _: () = assert!(
    BUILTIN_PLUGIN_UPDATE_SOURCES.len() <= 2,
    "BUILTIN_PLUGIN_UPDATE_SOURCES must not grow beyond a transitional \
     handful; see the doc comment on the constant for retirement policy."
);

/// Map `(std::env::consts::OS, std::env::consts::ARCH)` to the
/// `<os>-<arch>` alias the plugin URL format uses. Diverges from
/// `garyx update`'s Rust-target-triple format by design — plugins
/// publish under the simpler alias.
pub(super) fn resolve_target_alias(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("macos", "x86_64") => Some("mac-x86_64"),
        ("macos", "aarch64") => Some("mac-aarch64"),
        _ => None,
    }
}

/// Render a template by substituting `{id}` / `{version}` / `{target}`,
/// and `{url}` when `rendered_url` is `Some` (used by checksum URL
/// templates). Unknown placeholders return `InvalidTemplate`.
pub(super) fn render_template(
    template: &str,
    id: &str,
    version: &str,
    target: &str,
    rendered_url: Option<&str>,
) -> Result<String, PluginsCliError> {
    let mut out = String::with_capacity(template.len() + 32);
    let mut chars = template.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch != '{' {
            out.push(ch);
            continue;
        }
        // `{{` is an escape for a literal `{`.
        if matches!(chars.peek(), Some((_, '{'))) {
            chars.next();
            out.push('{');
            continue;
        }
        // Find the matching `}` from the position immediately after this `{`.
        // `i + 1` is safe because `{` is ASCII (1 byte).
        let rest = &template[i + 1..];
        let Some(end_rel) = rest.find('}') else {
            return Err(PluginsCliError::InvalidTemplate {
                template: template.to_string(),
                reason: "unterminated `{` placeholder".to_string(),
            });
        };
        let name = &rest[..end_rel];
        let replacement = match name {
            "id" => id,
            "version" => version,
            "target" => target,
            "url" => rendered_url.ok_or_else(|| PluginsCliError::InvalidTemplate {
                template: template.to_string(),
                reason: "`{url}` only valid in checksum_url_template".to_string(),
            })?,
            other => {
                return Err(PluginsCliError::InvalidTemplate {
                    template: template.to_string(),
                    reason: format!("unknown placeholder `{other}`"),
                });
            }
        };
        out.push_str(replacement);
        // Skip past the matched `}`. The `find` returned a byte offset
        // into `rest`, so advance the iterator until we're past the `}`.
        let absolute_end = i + 1 + end_rel + 1; // position AFTER `}`
        while let Some(&(pos, _)) = chars.peek() {
            if pos < absolute_end {
                chars.next();
            } else {
                break;
            }
        }
    }
    Ok(out)
}

/// Owned shape for `UpdateSource` so we can return both static
/// (built-in) and manifest-sourced entries from the same fn.
///
/// `checksum_url_template` is `Option<String>` to preserve three
/// distinct states (the manifest spec treats them differently):
///   - `None`            → caller should default to `"{url}.sha256"`;
///   - `Some(String::new())` → operator explicitly disabled checksums;
///   - `Some(template)`  → use that template.
#[derive(Debug, Clone)]
pub(super) struct EffectiveSource {
    pub manifest_url: Option<String>,
    pub url_template: String,
    pub checksum_url_template: Option<String>,
    pub binary_in_archive: String,
}

impl From<&UpdateSource> for EffectiveSource {
    fn from(src: &UpdateSource) -> Self {
        // The static built-in table always specifies a value (even if
        // empty-string to disable), so we always carry `Some(_)` here.
        Self {
            manifest_url: src.manifest_url.map(str::to_string),
            url_template: src.url_template.to_string(),
            checksum_url_template: Some(src.checksum_url_template.to_string()),
            binary_in_archive: src.binary_in_archive.to_string(),
        }
    }
}

impl From<garyx_channels::plugin_host::PluginUpdate> for EffectiveSource {
    fn from(value: garyx_channels::plugin_host::PluginUpdate) -> Self {
        // Pass `checksum_url_template` through as-is so the three
        // states (None / Some("") / Some(t)) reach the caller intact.
        Self {
            manifest_url: value.manifest_url,
            url_template: value.url_template,
            checksum_url_template: value.checksum_url_template,
            binary_in_archive: value
                .binary_in_archive
                .unwrap_or_else(|| "{id}/garyx-plugin-{id}".to_string()),
        }
    }
}

/// Resolve the effective update source for `id` from
///   1. `--from` (returned by the caller, not handled here);
///   2. the installed manifest's `[update]` block;
///   3. `BUILTIN_PLUGIN_UPDATE_SOURCES`.
///
/// Returns `(source, installed_version)`.
pub(super) fn resolve_source(
    id: &str,
    target_root: &std::path::PathBuf,
    _from_override: Option<&str>,
) -> Result<(EffectiveSource, String), PluginsCliError> {
    let manifest_path = target_root.join(id).join("plugin.toml");
    if !manifest_path.is_file() {
        return Err(PluginsCliError::NotInstalled(target_root.join(id)));
    }
    let manifest = garyx_channels::plugin_host::PluginManifest::load(&manifest_path)
        .map_err(|e| PluginsCliError::Io {
            context: format!("loading {}", manifest_path.display()),
            source: std::io::Error::other(e.to_string()),
        })?;
    let installed_version = manifest.plugin.version.clone();

    if let Some(update) = manifest.update {
        return Ok((update.into(), installed_version));
    }

    if let Some((_, src)) = BUILTIN_PLUGIN_UPDATE_SOURCES
        .iter()
        .find(|(name, _)| *name == id)
    {
        return Ok((EffectiveSource::from(src), installed_version));
    }

    Err(PluginsCliError::NoUpdateSource { id: id.to_string() })
}

/// HTTP-GET `url` and parse `{"version": "..."}` from the response.
///
/// Used when the operator omits `--version`: the plugin's `[update]`
/// block (or built-in fallback) declares a `manifest_url`, and this
/// function fetches it and extracts the latest version string.
///
/// Honors the spec's 30s timeout. On any failure — network, non-2xx
/// status, malformed JSON, missing `version` field — returns
/// `PluginsCliError::VersionDiscoveryFailed` whose message includes
/// the URL and the underlying reason so operators can diagnose
/// without re-running with verbose flags.
///
/// A leading `v` on the version string is stripped (some publishers
/// emit `v0.1.16`); whitespace is also trimmed.
pub(super) async fn discover_latest_version(
    client: &reqwest::Client,
    url: &str,
    id: &str,
) -> Result<String, PluginsCliError> {
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| PluginsCliError::VersionDiscoveryFailed {
            id: id.to_string(),
            detail: format!("GET {url}: {e}"),
        })?
        .error_for_status()
        .map_err(|e| PluginsCliError::VersionDiscoveryFailed {
            id: id.to_string(),
            detail: format!("GET {url}: {e}"),
        })?;
    let body: serde_json::Value =
        resp.json().await.map_err(|e| PluginsCliError::VersionDiscoveryFailed {
            id: id.to_string(),
            detail: format!("parsing JSON from {url}: {e}"),
        })?;
    body.get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().trim_start_matches('v').to_string())
        .ok_or_else(|| PluginsCliError::VersionDiscoveryFailed {
            id: id.to_string(),
            detail: format!("response from {url} has no `version` field"),
        })
}

/// HTTP-GET a release archive, fetch its sha256 sidecar, verify the
/// digest, and return the raw archive bytes.
///
/// The checksum file is expected to look like the output of
/// `shasum -a 256 file.tar.gz`, i.e. `<sha>  <filename>\n`, but we
/// only consume the first whitespace-delimited token on the first
/// non-empty line, so a bare `<sha>\n` is also accepted.
///
/// Timeouts mirror the spec: 5 minutes for the archive (large payload
/// over potentially slow networks), 30 seconds for the small text
/// checksum file.
pub(super) async fn download_archive(
    client: &reqwest::Client,
    archive_url: &str,
    checksum_url: &str,
) -> Result<Vec<u8>, PluginsCliError> {
    let archive_bytes = client
        .get(archive_url)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| PluginsCliError::DownloadFailed {
            url: archive_url.to_string(),
            detail: e.to_string(),
        })?
        .bytes()
        .await
        .map_err(|e| PluginsCliError::DownloadFailed {
            url: archive_url.to_string(),
            detail: e.to_string(),
        })?;

    let checksum_text = client
        .get(checksum_url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| PluginsCliError::DownloadFailed {
            url: checksum_url.to_string(),
            detail: e.to_string(),
        })?
        .text()
        .await
        .map_err(|e| PluginsCliError::DownloadFailed {
            url: checksum_url.to_string(),
            detail: e.to_string(),
        })?;
    let expected = checksum_text
        .lines()
        .find_map(|line| line.split_whitespace().next())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| PluginsCliError::ArchiveLayout {
            hint: format!("checksum file at {checksum_url} is empty or malformed"),
        })?
        .to_string();

    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&archive_bytes);
    let mut actual = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut actual, "{b:02x}");
    }
    if actual != expected {
        return Err(PluginsCliError::ChecksumMismatch {
            url: archive_url.to_string(),
            expected,
            actual,
        });
    }
    Ok(archive_bytes.to_vec())
}

/// Unpack `archive_bytes` (a gzipped tarball) into `out_dir`, then
/// resolve the plugin binary's path inside the extracted tree by
/// rendering `binary_in_archive` against `id`.
///
/// `binary_in_archive` only references `{id}` per the documented
/// contract — `version` and `target` are passed empty so a stray
/// `{version}`/`{target}` in the template surfaces as
/// `InvalidTemplate` (wrapped in `ArchiveLayout` for the caller).
///
/// On unix, the resolved binary is chmod'd to `0o755` so a tarball
/// that didn't preserve the executable bit still runs after install.
pub(super) fn extract_and_locate_binary(
    archive_bytes: &[u8],
    out_dir: &std::path::Path,
    id: &str,
    binary_in_archive: &str,
) -> Result<std::path::PathBuf, PluginsCliError> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let decoder = GzDecoder::new(std::io::Cursor::new(archive_bytes));
    let mut archive = Archive::new(decoder);
    archive.unpack(out_dir).map_err(|e| PluginsCliError::ArchiveLayout {
        hint: format!("unpacking archive into {}: {e}", out_dir.display()),
    })?;

    let relative = render_template(binary_in_archive, id, "", "", None).map_err(|e| {
        PluginsCliError::ArchiveLayout {
            hint: format!("rendering binary_in_archive `{binary_in_archive}`: {e}"),
        }
    })?;
    let bin = out_dir.join(&relative);
    if !bin.is_file() {
        return Err(PluginsCliError::ArchiveLayout {
            hint: format!(
                "archive layout did not contain `{relative}` (expected the plugin binary). Extracted root: {}",
                out_dir.display(),
            ),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin)
            .map_err(|e| PluginsCliError::Io {
                context: format!("stat {}", bin.display()),
                source: e,
            })?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin, perms).map_err(|e| PluginsCliError::Io {
            context: format!("chmod +x {}", bin.display()),
            source: e,
        })?;
    }

    Ok(bin)
}

pub async fn update(
    name: Option<&str>,
    opts: UpdateOptions,
) -> Result<(), PluginsCliError> {
    match name {
        Some(id) => update_one(id, opts).await,
        None => update_all(opts).await,
    }
}

pub(super) async fn update_one(
    name: &str,
    opts: UpdateOptions,
) -> Result<(), PluginsCliError> {
    let json = opts.json;
    let outcome = compute_update_outcome(name, opts).await?;
    render_outcome(&outcome, json);
    Ok(())
}

/// Heart of `garyx plugins update <id>`: does everything except print.
/// Returns an `UpdateOutcome` so that both `update_one` (one plugin at
/// a time) and `update_all` (many plugins, aggregated output) can
/// render the result however they like.
async fn compute_update_outcome(
    name: &str,
    opts: UpdateOptions,
) -> Result<UpdateOutcome, PluginsCliError> {
    // Step 1 — built-in guard (runs before any IO). Reuses the
    // canonical id-or-alias lookup from `garyx_channels::builtin_catalog`
    // so there is one source of truth for "what counts as built-in."
    if builtin_channel_descriptor(name).is_some() {
        let root = opts
            .target
            .clone()
            .unwrap_or_else(default_plugin_install_root);
        return Err(PluginsCliError::BuiltinChannelNotPluggable {
            id: name.to_string(),
            root,
        });
    }
    let target_root = opts
        .target
        .clone()
        .unwrap_or_else(default_plugin_install_root);

    // Step 2 — `--from` short-circuit. A local path skips both manifest
    // resolution and version discovery: the operator already pointed at
    // exactly the bundle they want promoted. A `http(s)` URL still goes
    // through download + extract, but with no checksum verification —
    // the operator vouches for the URL.
    if let Some(from) = opts.from.as_deref() {
        if !from.starts_with("http://") && !from.starts_with("https://") {
            let bundle_binary = resolve_local_bundle(from, name)?;
            return promote_local(&bundle_binary, name, &target_root).await;
        }
        return download_and_install_url(name, from, &target_root).await;
    }

    // Step 3 — resolve effective source + currently installed version
    // from the on-disk manifest (or the built-in fallback table).
    let (source, installed_version) = match resolve_source(name, &target_root, None) {
        Ok(pair) => pair,
        // Spec: `--check` against a plugin with no declared update
        // source emits a `no_source` outcome instead of erroring, so
        // operators can `--check` every plugin in a loop without
        // special-casing.
        Err(PluginsCliError::NoUpdateSource { id }) if opts.check => {
            return Ok(UpdateOutcome::no_source(&id));
        }
        Err(e) => return Err(e),
    };

    // Step 4 — resolve the target version. Either taken from
    // `--version` or fetched from the manifest_url.
    let client = http_client()?;
    let version = match opts.version.as_deref() {
        Some(v) => v.trim().trim_start_matches('v').to_string(),
        None => {
            let manifest_url = source
                .manifest_url
                .clone()
                .ok_or_else(|| PluginsCliError::VersionDiscoveryFailed {
                    id: name.to_string(),
                    detail: "no manifest_url declared; pass --version explicitly".to_string(),
                })?;
            let target = current_target_alias()?;
            let rendered = render_template(&manifest_url, name, "", target, None)?;
            discover_latest_version(&client, &rendered, name).await?
        }
    };

    // Step 5 — same-version short-circuit. `--force` overrides this so
    // operators can reinstall on top of an existing version.
    if !opts.force && installed_version == version {
        return Ok(UpdateOutcome::already_current(name, &installed_version));
    }

    // Step 6 — render the archive URL up front. `--check` exits here
    // so the caller can preview without paying for the download.
    let target = current_target_alias()?;
    let archive_url = render_template(&source.url_template, name, &version, target, None)?;
    if opts.check {
        return Ok(UpdateOutcome::check_report(
            name,
            &installed_version,
            &version,
            &archive_url,
        ));
    }

    // Step 7 — resolve checksum URL. Three-state per the manifest spec:
    //   * `None`        → default to "{url}.sha256"
    //   * `Some("")`    → operator disabled checksums explicitly
    //   * `Some(t)`     → render `t` against the current archive URL
    let checksum_url = match source.checksum_url_template.as_deref() {
        Some("") => None,
        Some(template) => Some(render_template(
            template,
            name,
            &version,
            target,
            Some(&archive_url),
        )?),
        None => Some(render_template(
            "{url}.sha256",
            name,
            &version,
            target,
            Some(&archive_url),
        )?),
    };

    promote_url(
        name,
        &version,
        &installed_version,
        &archive_url,
        checksum_url.as_deref(),
        &source.binary_in_archive,
        &target_root,
    )
    .await
}

/// `--from <URL>` path: GET the URL, extract, promote via the shared
/// install() flow. No checksum verification — the operator vouches for
/// the URL they pasted on the command line.
async fn download_and_install_url(
    name: &str,
    url: &str,
    target_root: &std::path::Path,
) -> Result<UpdateOutcome, PluginsCliError> {
    let client = http_client()?;
    let bytes = client
        .get(url)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| PluginsCliError::DownloadFailed {
            url: url.to_string(),
            detail: e.to_string(),
        })?
        .bytes()
        .await
        .map_err(|e| PluginsCliError::DownloadFailed {
            url: url.to_string(),
            detail: e.to_string(),
        })?;
    let staging = tempfile::tempdir().map_err(|e| PluginsCliError::Io {
        context: "creating extraction tempdir".to_string(),
        source: e,
    })?;
    // `--from URL` doesn't know the publisher's archive layout, so we
    // assume the conventional `{id}/garyx-plugin-{id}` shape used by
    // every first-party plugin tarball.
    let bin = extract_and_locate_binary(&bytes, staging.path(), name, "{id}/garyx-plugin-{id}")?;
    // `--from URL` skips the installed-manifest lookup, so we don't
    // know what version was installed previously. The success outcome
    // carries `previous: None` to reflect that.
    promote_into_install_root(name, &bin, target_root, url, None, None).await
}

/// Manifest-driven URL path: download (optionally verifying the
/// checksum sidecar), extract, then promote via the shared install()
/// flow. The caller has already computed both URLs.
#[allow(clippy::too_many_arguments)]
async fn promote_url(
    name: &str,
    version: &str,
    installed_version: &str,
    archive_url: &str,
    checksum_url: Option<&str>,
    binary_in_archive: &str,
    target_root: &std::path::Path,
) -> Result<UpdateOutcome, PluginsCliError> {
    let client = http_client()?;
    let archive_bytes = if let Some(checksum_url) = checksum_url {
        download_archive(&client, archive_url, checksum_url).await?
    } else {
        client
            .get(archive_url)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| PluginsCliError::DownloadFailed {
                url: archive_url.to_string(),
                detail: e.to_string(),
            })?
            .bytes()
            .await
            .map_err(|e| PluginsCliError::DownloadFailed {
                url: archive_url.to_string(),
                detail: e.to_string(),
            })?
            .to_vec()
    };
    let staging = tempfile::tempdir().map_err(|e| PluginsCliError::Io {
        context: "creating extraction tempdir".to_string(),
        source: e,
    })?;
    let bin = extract_and_locate_binary(&archive_bytes, staging.path(), name, binary_in_archive)?;
    promote_into_install_root(
        name,
        &bin,
        target_root,
        archive_url,
        Some(version),
        Some(installed_version),
    )
    .await
}

/// The shared final step: hand the staged binary off to
/// `super::install(force=true)`, then verify the resulting install
/// directory name matches the requested plugin id. A mismatch means
/// the bundle's self-reported id (via the describe handshake) didn't
/// match what the operator asked for, so we roll the rogue install
/// back rather than leave the wrong directory in place.
async fn promote_into_install_root(
    name: &str,
    binary_path: &std::path::Path,
    target_root: &std::path::Path,
    source: &str,
    expected_version: Option<&str>,
    previous_version: Option<&str>,
) -> Result<UpdateOutcome, PluginsCliError> {
    let started = std::time::Instant::now();
    let dest = super::install(binary_path, Some(target_root.to_path_buf()), true).await?;
    let installed_id = dest.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if installed_id != name {
        if let Err(e) = std::fs::remove_dir_all(&dest) {
            warn!(
                path = %dest.display(),
                error = %e,
                "failed to roll back rogue install directory after bundle id mismatch",
            );
        }
        return Err(PluginsCliError::BundleIdMismatch {
            expected: name.to_string(),
            got: installed_id.to_string(),
        });
    }
    let elapsed = started.elapsed();
    Ok(UpdateOutcome {
        id: name.to_string(),
        previous: previous_version.map(str::to_string),
        next: expected_version.map(str::to_string),
        status: "updated",
        source: Some(source.to_string()),
        install_dir: Some(dest.display().to_string()),
        elapsed_ms: Some(elapsed.as_millis()),
        error: None,
    })
}

/// `--from <local-path-or-dir>`: skip download/extract entirely and
/// promote the bundle binary directly.
async fn promote_local(
    bundle_binary: &std::path::Path,
    name: &str,
    target_root: &std::path::Path,
) -> Result<UpdateOutcome, PluginsCliError> {
    promote_into_install_root(
        name,
        bundle_binary,
        target_root,
        &bundle_binary.display().to_string(),
        // Local bundles don't carry version metadata before install, so
        // we can't surface a `next` version. The text-mode renderer
        // shows "Updated `name` from local bundle." in this case.
        None,
        // Same story for `previous` — we don't read the installed
        // manifest on this path.
        None,
    )
    .await
}

/// Turn an operator-supplied local path into the plugin binary on disk.
/// A file path is taken as-is; a directory is treated as a bundle and
/// resolved through its `plugin.toml` (preferred) or a conventional
/// `garyx-plugin-<id>` filename next to it.
fn resolve_local_bundle(from: &str, name: &str) -> Result<std::path::PathBuf, PluginsCliError> {
    let p = std::path::PathBuf::from(from);
    if p.is_file() {
        return Ok(p);
    }
    if p.is_dir() {
        let manifest_path = p.join("plugin.toml");
        if manifest_path.is_file() {
            let manifest = garyx_channels::plugin_host::PluginManifest::load(&manifest_path)
                .map_err(|e| PluginsCliError::Io {
                    context: format!("loading {}", manifest_path.display()),
                    source: std::io::Error::other(e.to_string()),
                })?;
            return Ok(manifest.binary_path());
        }
        let fallback = p.join(format!("garyx-plugin-{name}"));
        if fallback.is_file() {
            return Ok(fallback);
        }
    }
    Err(PluginsCliError::Io {
        context: format!("--from `{from}` is not a binary or bundle directory"),
        source: std::io::Error::other("path does not exist or has no plugin.toml"),
    })
}

/// Build the `reqwest::Client` used for all plugin-update network I/O.
/// Sets the same `User-Agent` the `garyx update` (self-update) flow uses
/// so server-side request logs can distinguish the CLI from a browser.
pub(super) fn http_client() -> Result<reqwest::Client, PluginsCliError> {
    reqwest::Client::builder()
        .user_agent(format!("garyx-cli/{}", crate::commands::VERSION))
        .build()
        .map_err(|e| PluginsCliError::Io {
            context: "building HTTP client".to_string(),
            source: std::io::Error::other(e.to_string()),
        })
}

/// Resolve the current host's plugin-release target alias, surfacing a
/// readable error for platforms we don't publish artifacts for.
pub(super) fn current_target_alias() -> Result<&'static str, PluginsCliError> {
    resolve_target_alias(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        PluginsCliError::Io {
            context: "resolving plugin release target".to_string(),
            source: std::io::Error::other(format!(
                "unsupported platform: {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH,
            )),
        }
    })
}

pub(super) async fn update_all(opts: UpdateOptions) -> Result<(), PluginsCliError> {
    if opts.from.is_some() {
        return Err(PluginsCliError::ConflictingFlags {
            reason: "`--from` requires a plugin name".to_string(),
        });
    }
    let target_root = opts.target.clone().unwrap_or_else(default_plugin_install_root);
    let ids = list_installed_plugin_ids(&target_root)?;
    if ids.is_empty() {
        if opts.json {
            println!("[]");
        } else {
            println!(
                "No subprocess plugins installed under {}.",
                target_root.display(),
            );
        }
        return Ok(());
    }

    let mut any_failed = false;
    let mut outcomes: Vec<UpdateOutcome> = Vec::with_capacity(ids.len());

    for id in ids {
        // Per-iteration options inherit the global flags but force
        // an explicit target_root so a relative `--target` argument
        // doesn't get reinterpreted between iterations. `json: false`
        // suppresses per-iteration printing — we aggregate into a
        // single JSON array at the end.
        let per = UpdateOptions {
            version: opts.version.clone(),
            from: None,
            target: Some(target_root.clone()),
            check: opts.check,
            force: opts.force,
            json: false,
        };
        match compute_update_outcome(&id, per).await {
            Ok(outcome) => {
                if !opts.json {
                    render_outcome(&outcome, false);
                }
                outcomes.push(outcome);
            }
            Err(e) => {
                any_failed = true;
                let outcome = UpdateOutcome {
                    id: id.clone(),
                    previous: None,
                    next: None,
                    status: "failed",
                    source: None,
                    install_dir: None,
                    elapsed_ms: None,
                    error: Some(e.to_string()),
                };
                if !opts.json {
                    render_outcome(&outcome, false);
                }
                outcomes.push(outcome);
            }
        }
    }

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&outcomes).unwrap());
    }
    if any_failed {
        Err(PluginsCliError::Io {
            context: "one or more plugins failed to update".to_string(),
            source: std::io::Error::other("see previous errors"),
        })
    } else {
        Ok(())
    }
}

pub(super) fn list_installed_plugin_ids(
    target_root: &std::path::Path,
) -> Result<Vec<String>, PluginsCliError> {
    if !target_root.exists() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<String> = std::fs::read_dir(target_root)
        .map_err(|e| PluginsCliError::Io {
            context: format!("reading {}", target_root.display()),
            source: e,
        })?
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
    use sha2::{Digest, Sha256};

    /// Build an in-memory gzipped tarball for tests. Reused by the
    /// download/extract tests in Tasks 11 and 12 so they don't need
    /// to ship fixture binaries on disk.
    fn make_tarball(contents: &[(&str, &[u8])]) -> Vec<u8> {
        let mut gz =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut builder = tar::Builder::new(&mut gz);
            for (path, data) in contents {
                let mut header = tar::Header::new_gnu();
                header.set_path(path).unwrap();
                header.set_size(data.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append(&header, *data).unwrap();
            }
            builder.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    /// Lowercase-hex SHA-256 digest of `bytes`.
    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let mut out = String::with_capacity(digest.len() * 2);
        for b in digest {
            use std::fmt::Write as _;
            write!(&mut out, "{b:02x}").unwrap();
        }
        out
    }

    #[tokio::test]
    async fn rejects_builtin_channel_id() {
        let result = update_one("telegram", UpdateOptions::default()).await;
        match result {
            Err(PluginsCliError::BuiltinChannelNotPluggable { id, .. }) => {
                assert_eq!(id, "telegram");
            }
            other => panic!("expected BuiltinChannelNotPluggable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_builtin_channel_alias() {
        // `wechat` is a documented alias of `weixin` in
        // garyx_channels::builtin_catalog.
        let result = update_one("wechat", UpdateOptions::default()).await;
        assert!(
            matches!(
                result,
                Err(PluginsCliError::BuiltinChannelNotPluggable { ref id, .. }) if id == "wechat",
            ),
            "expected BuiltinChannelNotPluggable for alias `wechat`, got {result:?}",
        );
    }

    // ---- Clap-level flag-conflict matrix ----
    //
    // These tests lock in the `conflicts_with_all` attributes declared
    // on `PluginsAction::Update` in `cli.rs` so a future flag-rename
    // can't silently drop the documented matrix from the spec's
    // "Flag interactions" section.
    use clap::Parser;

    // `crate::cli::Cli` doesn't derive `Debug`, so `expect_err` isn't
    // available; pattern-match the `Result` instead.
    fn expect_clap_err(argv: &[&str], label: &str) -> clap::Error {
        match crate::cli::Cli::try_parse_from(argv) {
            Ok(_) => panic!("clap should reject {label}"),
            Err(e) => e,
        }
    }

    #[test]
    fn clap_rejects_from_with_version() {
        let err = expect_clap_err(
            &[
                "garyx", "plugins", "update", "demo", "--from", "./bundle", "--version", "0.1.0",
            ],
            "--from + --version",
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("--from") || msg.contains("--version"),
            "clap error should mention the conflict: {msg}",
        );
    }

    #[test]
    fn clap_rejects_check_with_force() {
        let err = expect_clap_err(
            &["garyx", "plugins", "update", "demo", "--check", "--force"],
            "--check + --force",
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("--check") || msg.contains("--force"),
            "clap error should mention the conflict: {msg}",
        );
    }

    #[test]
    fn clap_rejects_check_with_from() {
        let err = expect_clap_err(
            &[
                "garyx", "plugins", "update", "demo", "--check", "--from", "./bundle",
            ],
            "--check + --from",
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("--check") || msg.contains("--from"),
            "clap error should mention the conflict: {msg}",
        );
    }

    use super::{
        BUILTIN_PLUGIN_UPDATE_SOURCES, UpdateSource, render_template, resolve_target_alias,
    };

    #[test]
    fn renderer_substitutes_known_placeholders() {
        let out = render_template(
            "https://x.test/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz",
            "example-plugin",
            "0.1.16",
            "linux-x86_64",
            None,
        )
        .unwrap();
        assert_eq!(
            out,
            "https://x.test/example-plugin/0.1.16/garyx-plugin-example-plugin-0.1.16-linux-x86_64.tar.gz",
        );
    }

    #[test]
    fn renderer_substitutes_url_in_checksum_template() {
        let out = render_template(
            "{url}.sha256",
            "example-plugin",
            "0.1.16",
            "linux-x86_64",
            Some("https://x.test/foo.tar.gz"),
        )
        .unwrap();
        assert_eq!(out, "https://x.test/foo.tar.gz.sha256");
    }

    #[test]
    fn renderer_rejects_unknown_placeholder() {
        let err = render_template("https://x.test/{nope}", "x", "1", "linux-x86_64", None)
            .expect_err("unknown placeholder rejected");
        assert!(format!("{err:?}").contains("nope"));
    }

    #[test]
    fn target_alias_matches_release_format() {
        assert_eq!(resolve_target_alias("linux", "x86_64"), Some("linux-x86_64"));
        assert_eq!(resolve_target_alias("linux", "aarch64"), Some("linux-aarch64"));
        assert_eq!(resolve_target_alias("macos", "aarch64"), Some("mac-aarch64"));
        assert_eq!(resolve_target_alias("macos", "x86_64"), Some("mac-x86_64"));
        assert_eq!(resolve_target_alias("windows", "x86_64"), None);
    }

    #[test]
    fn builtin_fallback_includes_example_plugin() {
        let m: &UpdateSource = BUILTIN_PLUGIN_UPDATE_SOURCES
            .iter()
            .find(|(id, _)| *id == "example-plugin")
            .map(|(_, src)| src)
            .expect("example-plugin present in fallback");
        assert!(m.manifest_url.is_none(), "manifest_url is None during transitional period");
        assert!(m.url_template.contains("{id}"));
        assert!(m.url_template.contains("{version}"));
        assert!(m.url_template.contains("{target}"));
    }

    #[test]
    fn renderer_emits_literal_brace_on_escape() {
        let out = render_template("x{{y", "id", "ver", "tgt", None).unwrap();
        assert_eq!(out, "x{y");
    }

    #[test]
    fn renderer_rejects_unterminated_brace() {
        let err = render_template("prefix/{id", "x", "1", "linux-x86_64", None)
            .expect_err("unterminated placeholder rejected");
        assert!(
            format!("{err:?}").contains("unterminated"),
            "expected unterminated reason: {err:?}",
        );
    }

    #[test]
    fn renderer_rejects_url_when_rendered_url_missing() {
        let err = render_template("prefix/{url}", "x", "1", "linux-x86_64", None)
            .expect_err("`{url}` without rendered_url rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("url") && msg.contains("checksum_url_template"),
            "expected url-only-in-checksum reason: {msg}",
        );
    }

    use garyx_channels::plugin_host::PluginUpdate;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn write_test_manifest(
        dir: &std::path::Path,
        id: &str,
        version: &str,
        update: Option<PluginUpdate>,
    ) {
        let binary = dir.join(format!("garyx-plugin-{id}"));
        std::fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            let mut p = std::fs::metadata(&binary).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&binary, p).unwrap();
        }
        let mut toml = format!(
            "[plugin]\nid = \"{id}\"\nversion = \"{version}\"\ndisplay_name = \"X\"\n\n\
             [entry]\nbinary = \"./garyx-plugin-{id}\"\n\n\
             [capabilities]\ndelivery_model = \"pull_explicit_ack\"\n",
        );
        if let Some(u) = update {
            toml.push_str("\n[update]\n");
            if let Some(m) = &u.manifest_url {
                toml.push_str(&format!("manifest_url = \"{m}\"\n"));
            }
            toml.push_str(&format!("url_template = \"{}\"\n", u.url_template));
            if let Some(c) = &u.checksum_url_template {
                toml.push_str(&format!("checksum_url_template = \"{c}\"\n"));
            }
            if let Some(b) = &u.binary_in_archive {
                toml.push_str(&format!("binary_in_archive = \"{b}\"\n"));
            }
        }
        std::fs::write(dir.join("plugin.toml"), toml).unwrap();
    }

    #[test]
    fn resolves_source_from_installed_manifest() {
        let root = tempfile::tempdir().unwrap();
        let plugin_dir = root.path().join("demo");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        write_test_manifest(
            &plugin_dir,
            "demo",
            "0.1.0",
            Some(PluginUpdate {
                manifest_url: Some("https://x.test/{id}/latest.json".into()),
                url_template: "https://x.test/{id}/{version}/x.tar.gz".into(),
                checksum_url_template: Some("{url}.sha256".into()),
                binary_in_archive: Some("{id}/garyx-plugin-{id}".into()),
            }),
        );
        let (src, installed_version) =
            super::resolve_source("demo", &root.path().to_path_buf(), None).unwrap();
        assert_eq!(installed_version, "0.1.0");
        assert_eq!(
            src.manifest_url.as_deref(),
            Some("https://x.test/{id}/latest.json"),
        );
    }

    #[test]
    fn resolves_source_from_builtin_fallback_when_manifest_missing_update() {
        let root = tempfile::tempdir().unwrap();
        let plugin_dir = root.path().join("example-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        write_test_manifest(&plugin_dir, "example-plugin", "0.1.15", None);
        let (src, _) =
            super::resolve_source("example-plugin", &root.path().to_path_buf(), None).unwrap();
        assert!(
            src.url_template.contains("example.com"),
            "expected the example-plugin built-in fallback to provide a URL template",
        );
    }

    #[test]
    fn resolves_source_returns_no_source_for_unknown_plugin_without_block() {
        let root = tempfile::tempdir().unwrap();
        let plugin_dir = root.path().join("randomplugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        write_test_manifest(&plugin_dir, "randomplugin", "0.1.0", None);
        let err =
            super::resolve_source("randomplugin", &root.path().to_path_buf(), None)
                .expect_err("no source");
        assert!(matches!(err, PluginsCliError::NoUpdateSource { ref id } if id == "randomplugin"));
    }

    #[test]
    fn resolve_source_returns_not_installed_for_missing_dir() {
        let root = tempfile::tempdir().unwrap();
        let err =
            super::resolve_source("ghost", &root.path().to_path_buf(), None)
                .expect_err("not installed");
        assert!(matches!(err, PluginsCliError::NotInstalled(_)));
    }

    // ---- resolve_local_bundle branches ----
    //
    // `--from <local-path>` has three resolution paths and each one
    // matters for a different operator workflow:
    //   * file → take as-is (e.g. `--from ./target/release/garyx-plugin-x`);
    //   * dir with plugin.toml → read manifest (e.g. installed bundle);
    //   * dir without plugin.toml → fall back to `garyx-plugin-<id>`
    //     next to it (matches the conventional release layout).

    #[test]
    fn resolve_local_bundle_accepts_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("garyx-plugin-demo");
        std::fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&binary).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&binary, p).unwrap();
        }
        let resolved = super::resolve_local_bundle(binary.to_str().unwrap(), "demo").unwrap();
        assert_eq!(resolved, binary);
    }

    #[test]
    fn resolve_local_bundle_reads_plugin_toml_in_directory() {
        let dir = tempfile::tempdir().unwrap();
        write_test_manifest(dir.path(), "demo", "0.1.0", None);
        let resolved = super::resolve_local_bundle(dir.path().to_str().unwrap(), "demo").unwrap();
        assert_eq!(resolved, dir.path().join("garyx-plugin-demo"));
    }

    #[test]
    fn resolve_local_bundle_falls_back_to_named_binary_when_no_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("garyx-plugin-demo");
        std::fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        let resolved = super::resolve_local_bundle(dir.path().to_str().unwrap(), "demo").unwrap();
        assert_eq!(resolved, binary);
    }

    // `IntoResponse` isn't used directly here (axum picks it up via the
    // `get(...)` handler return-type machinery), but importing it keeps
    // the trait in scope for any future handler that names it.
    #[allow(unused_imports)]
    use axum::{Router, response::IntoResponse, routing::get};
    use std::sync::Arc;

    /// Spin up a one-shot HTTP server on 127.0.0.1:0 that serves the
    /// given routes. Returns the base URL string. The server keeps
    /// running for the lifetime of the returned JoinHandle.
    async fn spin_server(routes: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, routes).await.unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn discover_latest_version_parses_manifest_json() {
        let app = Router::new().route(
            "/latest.json",
            get(|| async {
                axum::Json(serde_json::json!({
                    "version": "0.1.16",
                    "released_at": "2026-05-11T00:00:00Z",
                }))
            }),
        );
        let (base, _h) = spin_server(app).await;
        let client = reqwest::Client::builder()
            .user_agent("test-agent/1")
            .build()
            .unwrap();
        let v = super::discover_latest_version(&client, &format!("{base}/latest.json"), "demo")
            .await
            .unwrap();
        assert_eq!(v, "0.1.16");
    }

    #[tokio::test]
    async fn discover_latest_version_errors_on_missing_version_field() {
        let app = Router::new().route(
            "/latest.json",
            get(|| async { axum::Json(serde_json::json!({"foo": "bar"})) }),
        );
        let (base, _h) = spin_server(app).await;
        let client = reqwest::Client::new();
        let err = super::discover_latest_version(&client, &format!("{base}/latest.json"), "demo")
            .await
            .expect_err("missing version field");
        assert!(matches!(err, PluginsCliError::VersionDiscoveryFailed { .. }));
    }

    #[tokio::test]
    async fn download_archive_verifies_checksum() {
        let tar = make_tarball(&[(
            "demo/garyx-plugin-demo",
            b"#!/bin/sh\nexit 0\n",
        )]);
        let sha = sha256_hex(&tar);
        let tar_bytes = Arc::new(tar);
        let sha_text = format!("{sha}  demo.tar.gz\n");
        let tar_clone = tar_bytes.clone();
        let app = Router::new()
            .route(
                "/demo.tar.gz",
                get(move || {
                    let bytes = tar_clone.clone();
                    async move { (*bytes).clone() }
                }),
            )
            .route(
                "/demo.tar.gz.sha256",
                get(move || async move { sha_text.clone() }),
            );
        let (base, _h) = spin_server(app).await;
        let client = reqwest::Client::new();
        let bytes = super::download_archive(
            &client,
            &format!("{base}/demo.tar.gz"),
            &format!("{base}/demo.tar.gz.sha256"),
        )
        .await
        .unwrap();
        assert_eq!(&bytes[..], &tar_bytes[..]);
    }

    #[test]
    fn extract_and_locate_binary_default_layout() {
        let tar = make_tarball(&[(
            "demo/garyx-plugin-demo",
            b"#!/bin/sh\nexit 0\n",
        )]);
        let dir = tempfile::tempdir().unwrap();
        let bin = super::extract_and_locate_binary(
            &tar,
            dir.path(),
            "demo",
            "{id}/garyx-plugin-{id}",
        )
        .unwrap();
        assert_eq!(bin, dir.path().join("demo").join("garyx-plugin-demo"));
        assert!(bin.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&bin).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "extracted binary should be executable");
        }
    }

    #[test]
    fn extract_and_locate_binary_missing_path_errors() {
        let tar = make_tarball(&[("other/file", b"hello")]);
        let dir = tempfile::tempdir().unwrap();
        let err = super::extract_and_locate_binary(
            &tar,
            dir.path(),
            "demo",
            "{id}/garyx-plugin-{id}",
        )
        .expect_err("missing binary");
        assert!(matches!(err, PluginsCliError::ArchiveLayout { .. }));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn update_one_resolves_url_from_installed_manifest() {
        use garyx_channels::plugin_host::PluginUpdate;
        let root = tempfile::tempdir().unwrap();
        let plugin_dir = root.path().join("demo");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        write_test_manifest(
            &plugin_dir,
            "demo",
            "0.1.0",
            Some(PluginUpdate {
                manifest_url: Some("https://x.test/{id}/latest.json".into()),
                url_template:
                    "https://x.test/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz"
                        .into(),
                checksum_url_template: None,
                binary_in_archive: None,
            }),
        );
        let (src, installed) =
            super::resolve_source("demo", &root.path().to_path_buf(), None).unwrap();
        assert_eq!(installed, "0.1.0");

        let archive_url = super::render_template(
            &src.url_template, "demo", "0.1.1", "linux-x86_64", None,
        )
        .unwrap();
        assert_eq!(
            archive_url,
            "https://x.test/demo/0.1.1/garyx-plugin-demo-0.1.1-linux-x86_64.tar.gz",
        );
        // Checksum template defaults to "{url}.sha256" when manifest omits it.
        let checksum_template = src
            .checksum_url_template
            .clone()
            .unwrap_or_else(|| "{url}.sha256".to_string());
        let checksum_url = super::render_template(
            &checksum_template,
            "demo",
            "0.1.1",
            "linux-x86_64",
            Some(&archive_url),
        )
        .unwrap();
        assert_eq!(checksum_url, format!("{archive_url}.sha256"));
    }

    #[tokio::test]
    async fn update_one_promotes_from_url_source_resolves_urls() {
        // End-to-end install requires a real plugin SDK binary; this
        // test instead verifies that the URL resolver renders the
        // archive + checksum URLs correctly from a manifest [update]
        // block. The actual `super::install` step is exercised by the
        // crate's pre-existing plugin install test in tests/plugins/.
        let root = tempfile::tempdir().unwrap();
        let plugin_dir = root.path().join("demo");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        write_test_manifest(
            &plugin_dir,
            "demo",
            "0.1.0",
            Some(PluginUpdate {
                manifest_url: Some("https://x.test/{id}/latest.json".into()),
                url_template:
                    "https://x.test/{id}/{version}/garyx-plugin-{id}-{version}-{target}.tar.gz"
                        .into(),
                checksum_url_template: None,
                binary_in_archive: None,
            }),
        );
        let (src, installed) =
            super::resolve_source("demo", &root.path().to_path_buf(), None).unwrap();
        assert_eq!(installed, "0.1.0");

        let archive_url = super::render_template(
            &src.url_template,
            "demo",
            "0.1.1",
            "linux-x86_64",
            None,
        )
        .unwrap();
        assert_eq!(
            archive_url,
            "https://x.test/demo/0.1.1/garyx-plugin-demo-0.1.1-linux-x86_64.tar.gz",
        );
        // checksum_url_template was None in the manifest, so the EffectiveSource
        // got the "{url}.sha256" default; render that against `{url}`.
        let checksum_url = super::render_template(
            src.checksum_url_template.as_deref().unwrap_or("{url}.sha256"),
            "demo",
            "0.1.1",
            "linux-x86_64",
            Some(&archive_url),
        )
        .unwrap();
        assert_eq!(checksum_url, format!("{archive_url}.sha256"));
    }

    #[tokio::test]
    async fn update_all_iterates_installed_plugins() {
        let root = tempfile::tempdir().unwrap();
        for id in ["alpha", "beta"] {
            let dir = root.path().join(id);
            std::fs::create_dir_all(&dir).unwrap();
            write_test_manifest(&dir, id, "0.1.0", None);
        }
        let ids = super::list_installed_plugin_ids(root.path()).unwrap();
        assert_eq!(ids, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[tokio::test]
    async fn download_archive_rejects_bad_checksum() {
        let tar = make_tarball(&[("x", b"hello")]);
        let bad_sha = "0000000000000000000000000000000000000000000000000000000000000000";
        let tar_bytes = Arc::new(tar);
        let tar_clone = tar_bytes.clone();
        let app = Router::new()
            .route(
                "/x.tar.gz",
                get(move || {
                    let b = tar_clone.clone();
                    async move { (*b).clone() }
                }),
            )
            .route(
                "/x.tar.gz.sha256",
                get(move || async move { format!("{bad_sha}\n") }),
            );
        let (base, _h) = spin_server(app).await;
        let client = reqwest::Client::new();
        let err = super::download_archive(
            &client,
            &format!("{base}/x.tar.gz"),
            &format!("{base}/x.tar.gz.sha256"),
        )
        .await
        .expect_err("checksum mismatch");
        assert!(matches!(err, PluginsCliError::ChecksumMismatch { .. }));
    }
}
