//! `garyx plugins {install,list,uninstall}` subcommand implementations.
//!
//! Install flow (the friendly path the user asked for):
//!
//! 1. Accept a binary path on disk.
//! 2. Run the plugin's own `initialize(dry_run=true)` + `describe`
//!    handshake via [`garyx_channels::plugin_host::inspect`] to pull
//!    its self-reported id / version / capabilities / schema.
//! 3. Synthesize a production `plugin.toml` from the describe result
//!    so the operator never has to hand-write it.
//! 4. Stage the binary + manifest into a temp subdir of the install
//!    root, then atomically rename into `<root>/<id>/` — either the
//!    whole install lands or none of it does.
//!
//! The idempotency contract: running `garyx plugins install` on the
//! same binary again with `--force` replaces the previous install in
//! place. Without `--force` we refuse rather than silently overwrite
//! a manifest the user might have hand-tuned.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use garyx_channels::plugin_host::{
    InspectError, InspectReport, ManifestDiscoverer, PluginManifest, inspect,
    synthesize_manifest_toml,
};

use crate::channel_plugin_host::default_plugin_install_root;

/// Top-level error surface for `plugins install` / `list` /
/// `uninstall`. Kept as a unified enum so `main.rs` can match once.
#[derive(Debug, thiserror::Error)]
pub enum PluginsCliError {
    #[error("plugin inspection failed: {0}")]
    Inspect(#[from] InspectError),
    #[error("a plugin is already installed at {dest}. Re-run with --force to overwrite.")]
    AlreadyInstalled { dest: PathBuf },
    #[error("no plugin installed at {0}")]
    NotInstalled(PathBuf),
    #[error("filesystem error: {context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
}

fn io_err(context: impl Into<String>, source: std::io::Error) -> PluginsCliError {
    PluginsCliError::Io {
        context: context.into(),
        source,
    }
}

/// Install the plugin at `binary_path` into `target_root` (defaults
/// to `~/.garyx/plugins/`). Overwrites an existing install only if
/// `force` is true. Prints user-friendly progress to stdout and
/// returns on success with the resolved install directory.
pub async fn install(
    binary_path: &Path,
    target_root: Option<PathBuf>,
    force: bool,
) -> Result<PathBuf, PluginsCliError> {
    let root = target_root.unwrap_or_else(default_plugin_install_root);
    let started = Instant::now();

    println!("Inspecting plugin at {}", binary_path.display());
    let report = inspect(binary_path).await?;
    println!(
        "  ✓ {} v{} (protocol {:?})",
        report.id, report.version, report.protocol_versions
    );
    let cap_summary = capability_summary(&report);
    if !cap_summary.is_empty() {
        println!("  ✓ capabilities: {cap_summary}");
    }
    if !report.auth_flows.is_empty() {
        let flows: Vec<&str> = report.auth_flows.iter().map(|f| f.id.as_str()).collect();
        println!("  ✓ auth flows:   {}", flows.join(", "));
    }

    let dest = root.join(&report.id);
    if dest.exists() && !force {
        return Err(PluginsCliError::AlreadyInstalled { dest });
    }

    // Stage into a sibling temp dir under the same root so rename is
    // atomic (same filesystem). The staging dir is cleaned up on
    // every error path below.
    std::fs::create_dir_all(&root).map_err(|e| io_err("creating install root", e))?;
    let staging = tempfile::tempdir_in(&root).map_err(|e| io_err("creating staging dir", e))?;
    let binary_filename = binary_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("garyx-plugin-{}", report.id));
    let staged_binary = staging.path().join(&binary_filename);
    std::fs::copy(binary_path, &staged_binary)
        .map_err(|e| io_err("copying binary into staging dir", e))?;
    set_executable(&staged_binary)?;

    // Auto-detect a brand icon next to the binary. The user doesn't
    // have to mention it — we probe for the conventional names so
    // plugins just drop an `icon.svg` or `icon.png` alongside their
    // binary and the UI picks it up for the bot logo.
    let icon_staged = stage_plugin_icon(binary_path, staging.path())?;
    if let Some(name) = icon_staged.as_deref() {
        println!("  ✓ icon:         {name}");
    }
    let manifest_toml = synthesize_manifest_toml(&report, &binary_filename, icon_staged.as_deref());
    // Sanity: make sure the generated manifest parses before we let
    // it out of the staging dir. A malformed manifest would leave
    // the user with a non-discoverable plugin directory.
    PluginManifest::load(&write_temp_manifest(staging.path(), &manifest_toml)?).map_err(
        |err| {
            PluginsCliError::Io {
                context: format!(
                    "synthesized plugin.toml failed to parse; this is a garyx bug. manifest:\n{manifest_toml}"
                ),
                source: std::io::Error::other(err.to_string()),
            }
        },
    )?;

    // Atomic promotion: remove existing install (if --force) then
    // rename staging into place.
    if dest.exists() {
        std::fs::remove_dir_all(&dest)
            .map_err(|e| io_err(format!("removing old install at {}", dest.display()), e))?;
    }
    // `tempfile::TempDir::into_path` disarms the auto-cleanup so the
    // following rename can take ownership of the underlying path.
    let staging_path = staging.keep();
    std::fs::rename(&staging_path, &dest).map_err(|e| {
        io_err(
            format!(
                "renaming staging {} into {}",
                staging_path.display(),
                dest.display()
            ),
            e,
        )
    })?;

    let elapsed = started.elapsed();
    println!(
        "\nInstalled `{}` to {} ({:.2}s)",
        report.id,
        dest.display(),
        elapsed.as_secs_f64()
    );
    println!(
        "Next steps:\n  \
         1. Add an account for this channel (e.g. edit your garyx.json or use the desktop UI).\n  \
         2. Restart the gateway so the new plugin is picked up: `garyx gateway start`."
    );
    Ok(dest)
}

/// Pretty `list` / `list --json` output.
pub fn list(target_root: Option<PathBuf>, as_json: bool) -> Result<(), PluginsCliError> {
    let root = target_root.unwrap_or_else(default_plugin_install_root);
    let manifests = if root.exists() {
        ManifestDiscoverer::new([root.clone()])
            .discover()
            .map_err(|e| PluginsCliError::Io {
                context: format!("scanning {}", root.display()),
                source: std::io::Error::other(e.to_string()),
            })?
    } else {
        Default::default()
    };

    if as_json {
        let entries: Vec<serde_json::Value> = manifests
            .plugins
            .iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.plugin.id,
                    "version": m.plugin.version,
                    "display_name": m.plugin.display_name,
                    "install_dir": m.manifest_dir.clone(),
                    "capabilities": {
                        "outbound": m.capabilities.outbound,
                        "inbound": m.capabilities.inbound,
                        "streaming": m.capabilities.streaming,
                        "images": m.capabilities.images,
                        "files": m.capabilities.files,
                    }
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries).unwrap());
        return Ok(());
    }

    if manifests.plugins.is_empty() {
        println!("No plugins installed under {}.", root.display());
        return Ok(());
    }

    println!("Installed plugins (in {}):", root.display());
    for manifest in &manifests.plugins {
        println!(
            "  {}  v{}  ({})",
            manifest.plugin.id,
            manifest.plugin.version,
            capability_summary_from_manifest(manifest)
        );
    }
    if !manifests.errors.is_empty() {
        eprintln!("\nWarnings (manifests failed to parse):");
        for err in &manifests.errors {
            eprintln!("  {err}");
        }
    }
    Ok(())
}

pub fn uninstall(id: &str, target_root: Option<PathBuf>) -> Result<(), PluginsCliError> {
    let root = target_root.unwrap_or_else(default_plugin_install_root);
    let dest = root.join(id);
    if !dest.exists() {
        return Err(PluginsCliError::NotInstalled(dest));
    }
    std::fs::remove_dir_all(&dest)
        .map_err(|e| io_err(format!("removing {}", dest.display()), e))?;
    println!("Uninstalled `{id}` from {}.", dest.display());
    println!("Restart the gateway to stop the plugin's running child process if any.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn capability_summary(report: &InspectReport) -> String {
    let caps = &report.capabilities;
    let flags = [
        ("outbound", caps.outbound),
        ("inbound", caps.inbound),
        ("streaming", caps.streaming),
        ("images", caps.images),
        ("files", caps.files),
    ];
    flags
        .iter()
        .filter_map(|(name, flag)| flag.then_some(*name))
        .collect::<Vec<_>>()
        .join(" + ")
}

fn capability_summary_from_manifest(manifest: &PluginManifest) -> String {
    let caps = &manifest.capabilities;
    let flags = [
        ("outbound", caps.outbound),
        ("inbound", caps.inbound),
        ("streaming", caps.streaming),
        ("images", caps.images),
        ("files", caps.files),
    ];
    flags
        .iter()
        .filter_map(|(name, flag)| flag.then_some(*name))
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), PluginsCliError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).map_err(|e| io_err("stat", e))?;
    let mut perms = metadata.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| io_err("chmod +x", e))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), PluginsCliError> {
    Ok(())
}

/// Probe the directory holding `binary_path` for a conventional
/// icon file (`icon.svg` → `icon.png` → `icon.webp` → `icon.jpg`),
/// copy the first one into `staging_dir`, and return its filename.
/// Returns `Ok(None)` if no icon is found — plugins are free to ship
/// without branding.
fn stage_plugin_icon(
    binary_path: &Path,
    staging_dir: &Path,
) -> Result<Option<String>, PluginsCliError> {
    let Some(source_dir) = binary_path.parent() else {
        return Ok(None);
    };
    // Priority order: SVG > PNG > WebP > JPG. SVG wins because it
    // scales cleanly on macOS retina displays; PNG is the lossless
    // fallback. Anything else is a last-resort.
    for candidate in ["icon.svg", "icon.png", "icon.webp", "icon.jpg"] {
        let src = source_dir.join(candidate);
        if src.is_file() {
            let dst = staging_dir.join(candidate);
            std::fs::copy(&src, &dst).map_err(|e| {
                io_err(format!("copying icon from {} to staging", src.display()), e)
            })?;
            return Ok(Some(candidate.to_owned()));
        }
    }
    Ok(None)
}

/// Write the manifest string to `plugin.toml` inside `dir` and
/// return the file path. Used as a validation step before atomic
/// promotion.
fn write_temp_manifest(dir: &Path, contents: &str) -> Result<PathBuf, PluginsCliError> {
    let target = dir.join("plugin.toml");
    let mut file = std::fs::File::create(&target)
        .map_err(|e| io_err(format!("writing {}", target.display()), e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| io_err(format!("writing {}", target.display()), e))?;
    file.flush()
        .map_err(|e| io_err(format!("flushing {}", target.display()), e))?;
    Ok(target)
}
