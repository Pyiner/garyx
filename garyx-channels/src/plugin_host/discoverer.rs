//! Discover plugin manifests from the filesystem.
//!
//! The host looks in two places, union'd:
//! - `GARYX_PLUGIN_DIR` (if set): a platform-native PATH-style list of
//!   directories (`:`-separated on Unix, `;`-separated on Windows; see
//!   `std::env::split_paths`). Each directory is scanned one level
//!   deep.
//! - The installed plugin root (defaults to `$data_dir/plugins/`).
//!
//! Each candidate directory is expected to contain a `plugin.toml` at
//! its root. Subdirectories may each be a plugin. We do not recurse
//! further than one level to avoid pulling in vendored sub-plugins by
//! accident.
//!
//! Manifest discovery is intentionally read-only and side-effect-free:
//! validation of the binary's existence and a dry-run `initialize`
//! happen at preflight time (see [`super::preflight`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::manifest::{ManifestError, PluginManifest};

/// Errors surfaced by the discovery pass. Parse failures for a single
/// plugin are non-fatal to the overall scan — we collect them and let
/// the caller decide.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("failed to read plugin directory {path}: {source}")]
    ReadDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("duplicate plugin id `{id}`: found at {first} and {second}")]
    DuplicateId {
        id: String,
        first: PathBuf,
        second: PathBuf,
    },
}

/// Outcome of a discovery sweep. `plugins` always contains the
/// successfully parsed manifests; `errors` lists per-directory failures
/// that the operator should see but that should not block starting the
/// healthy ones.
#[derive(Debug, Default)]
pub struct DiscoveryOutcome {
    pub plugins: Vec<PluginManifest>,
    pub errors: Vec<ManifestError>,
}

/// Reads manifests from a list of root directories. Each root is
/// scanned one level deep.
pub struct ManifestDiscoverer {
    roots: Vec<PathBuf>,
}

impl ManifestDiscoverer {
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
        }
    }

    /// Build a discoverer from the standard search path:
    /// `$GARYX_PLUGIN_DIR` (PATH-style list: `:` on Unix, `;` on
    /// Windows) + `$data_dir/plugins`.
    pub fn from_env(default_root: &Path) -> Self {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(raw) = std::env::var_os("GARYX_PLUGIN_DIR") {
            // `split_paths` is platform-aware: `:`-separated on Unix,
            // `;`-separated on Windows. Empty entries are dropped.
            for part in std::env::split_paths(&raw) {
                if !part.as_os_str().is_empty() {
                    roots.push(part);
                }
            }
        }
        roots.push(default_root.to_path_buf());
        Self::new(roots)
    }

    pub fn discover(&self) -> Result<DiscoveryOutcome, DiscoveryError> {
        let mut outcome = DiscoveryOutcome::default();
        let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();

        for root in &self.roots {
            if !root.exists() {
                // Missing roots are not an error — the host might be
                // running in an environment where only one of the two
                // paths is populated.
                continue;
            }
            let entries = std::fs::read_dir(root).map_err(|source| DiscoveryError::ReadDir {
                path: root.clone(),
                source,
            })?;
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let manifest_path = path.join("plugin.toml");
                if !manifest_path.exists() {
                    continue;
                }
                match PluginManifest::load(&manifest_path) {
                    Ok(manifest) => {
                        let id = manifest.plugin.id.clone();
                        if let Some(first) = seen.get(&id) {
                            return Err(DiscoveryError::DuplicateId {
                                id,
                                first: first.clone(),
                                second: manifest_path,
                            });
                        }
                        seen.insert(id, manifest_path.clone());
                        outcome.plugins.push(manifest);
                    }
                    Err(err) => outcome.errors.push(err),
                }
            }
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests;
