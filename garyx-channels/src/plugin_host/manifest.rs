//! Parse `plugin.toml` into a validated [`PluginManifest`].
//!
//! The manifest is the *only* file the host reads to learn about a
//! plugin at discovery time. Secrets never appear here — credentials
//! live in `garyx.toml` under
//! `channels.<id>.accounts.<name>.config.*` and the plugin
//! validates them at `initialize` time.
//!
//! The manifest is intentionally data-only and safe to inspect before spawn.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Supported `[capabilities].delivery_model` values.
///
/// See design doc §11.3 for the semantics of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryModel {
    /// Strongest guarantee. Plugin ACKs upstream only after
    /// `inbound/end.ok`.
    PullExplicitAck,
    /// Plugin owns its HTTP listener; holds the upstream response
    /// open until `inbound/end.ok`.
    PushNegativeAck,
    /// Best-effort only. Plugin loses the message on crash.
    PushAtMostOnce,
}

impl DeliveryModel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PullExplicitAck => "pull_explicit_ack",
            Self::PushNegativeAck => "push_negative_ack",
            Self::PushAtMostOnce => "push_at_most_once",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestCapabilities {
    #[serde(default = "yes")]
    pub outbound: bool,
    #[serde(default = "yes")]
    pub inbound: bool,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub images: bool,
    #[serde(default)]
    pub files: bool,
    /// §6.4: currently always `false` in v0.2; reserved for a v2
    /// in-place reload protocol.
    #[serde(default)]
    pub hot_reload_accounts: bool,
    /// §15.2: channels that need the host's `public_url` set a
    /// fail-fast flag.
    #[serde(default)]
    pub requires_public_url: bool,
    /// §12.6: future host-proxied ingress opt-in. Always false in
    /// v0.2 — plugins with a push model own their own listener.
    #[serde(default)]
    pub needs_host_ingress: bool,
    pub delivery_model: DeliveryModel,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRuntime {
    /// §6.2 default 5000ms, host-enforced ceiling 60000ms.
    #[serde(default = "default_stop_grace")]
    pub stop_grace_ms: u64,
    /// §6.3 default 3000ms, host-enforced ceiling 30000ms.
    #[serde(default = "default_shutdown_grace")]
    pub shutdown_grace_ms: u64,
    /// §11.2 frame-size limit. Always clamped to the hard cap.
    #[serde(default = "default_max_frame_bytes")]
    pub max_frame_bytes: usize,
    /// §11.2 concurrent in-flight `deliver_inbound` cap.
    #[serde(default = "default_max_inflight_inbound")]
    pub max_inflight_inbound: u32,
}

impl Default for ManifestRuntime {
    fn default() -> Self {
        Self {
            stop_grace_ms: default_stop_grace(),
            shutdown_grace_ms: default_shutdown_grace(),
            max_frame_bytes: default_max_frame_bytes(),
            max_inflight_inbound: default_max_inflight_inbound(),
        }
    }
}

fn default_stop_grace() -> u64 {
    5_000
}
fn default_shutdown_grace() -> u64 {
    3_000
}
fn default_max_frame_bytes() -> usize {
    super::codec::MAX_FRAME_BYTES_DEFAULT
}
fn default_max_inflight_inbound() -> u32 {
    32
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthFlowDescriptor {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AccountRootBehavior {
    #[default]
    OpenDefault,
    ExpandOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginUi {
    #[serde(default)]
    pub account_root_behavior: AccountRootBehavior,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Absolute path to the directory containing `plugin.toml`. The
    /// binary entry-point is resolved relative to this. Set by the
    /// loader, not written in the file.
    #[serde(skip)]
    pub manifest_dir: PathBuf,
    pub plugin: PluginHeader,
    pub entry: PluginEntry,
    pub capabilities: ManifestCapabilities,
    #[serde(default)]
    pub runtime: ManifestRuntime,
    /// JSON Schema subset describing the `config` payload for each
    /// account. Kept as `serde_json::Value` because we don't validate
    /// schema structure ourselves — the plugin is authoritative.
    #[serde(default = "empty_schema")]
    pub schema: Value,
    #[serde(default)]
    pub auth_flows: Vec<AuthFlowDescriptor>,
    #[serde(default)]
    pub ui: PluginUi,
    #[serde(default)]
    pub min_host_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginHeader {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub version: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    /// Path (relative to `manifest_dir`) of the plugin's brand icon.
    /// PNG / JPG / SVG / WebP. `garyx plugins install` copies it
    /// next to the binary. At catalog-build time the gateway reads
    /// the file and bakes it inline as a `data:` URL in the
    /// `SubprocessPluginCatalogEntry.icon_data_url` field — the
    /// desktop UI never touches the filesystem directly. Paths
    /// are validated against `..`-traversal and absolute-path
    /// tricks before any read (see
    /// `crate::plugin::resolve_plugin_icon_path`). `None` means the
    /// UI should fall back to a generic logo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEntry {
    /// Path to the child-process binary, relative to `manifest_dir`.
    pub binary: String,
    /// Extra environment variables to set on the child, on top of
    /// the host-provided baseline.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Optional CLI args passed before any host-supplied args.
    #[serde(default)]
    pub args: Vec<String>,
}

fn empty_schema() -> Value {
    Value::Object(Default::default())
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to read manifest at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse manifest at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("manifest `{path}` has no `[plugin]` section")]
    MissingHeader { path: PathBuf },
    #[error("manifest `{path}`: plugin.id is empty")]
    EmptyId { path: PathBuf },
    #[error("manifest `{path}`: entry.binary `{binary}` does not exist")]
    MissingBinary { path: PathBuf, binary: PathBuf },
    #[error("manifest `{path}`: runtime.stop_grace_ms ({got}) exceeds host ceiling of 60000")]
    StopGraceTooLarge { path: PathBuf, got: u64 },
    #[error("manifest `{path}`: runtime.shutdown_grace_ms ({got}) exceeds host ceiling of 30000")]
    ShutdownGraceTooLarge { path: PathBuf, got: u64 },
}

impl PluginManifest {
    /// Parse and validate a manifest. Does *not* spawn the plugin —
    /// that is [`crate::plugin_host::transport::Transport::spawn`]'s
    /// job.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ManifestError::Read {
            path: path.to_owned(),
            source,
        })?;
        let mut manifest: PluginManifest =
            toml::from_str(&raw).map_err(|source| ManifestError::Parse {
                path: path.to_owned(),
                source,
            })?;
        manifest.manifest_dir = path
            .parent()
            .map(Path::to_owned)
            .unwrap_or_else(|| PathBuf::from("."));
        manifest.validate(path)?;
        Ok(manifest)
    }

    fn validate(&self, path: &Path) -> Result<(), ManifestError> {
        if self.plugin.id.trim().is_empty() {
            return Err(ManifestError::EmptyId {
                path: path.to_owned(),
            });
        }
        if self.runtime.stop_grace_ms > 60_000 {
            return Err(ManifestError::StopGraceTooLarge {
                path: path.to_owned(),
                got: self.runtime.stop_grace_ms,
            });
        }
        if self.runtime.shutdown_grace_ms > 30_000 {
            return Err(ManifestError::ShutdownGraceTooLarge {
                path: path.to_owned(),
                got: self.runtime.shutdown_grace_ms,
            });
        }
        Ok(())
    }

    pub fn binary_path(&self) -> PathBuf {
        self.manifest_dir.join(&self.entry.binary)
    }

    /// Check that the entry point exists and is executable. Called
    /// before spawn; kept separate from [`load`] so that `garyx
    /// doctor` can distinguish "manifest is malformed" from "binary is
    /// missing".
    pub fn verify_binary(&self, manifest_path: &Path) -> Result<(), ManifestError> {
        let bin = self.binary_path();
        if !bin.exists() {
            return Err(ManifestError::MissingBinary {
                path: manifest_path.to_owned(),
                binary: bin,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
