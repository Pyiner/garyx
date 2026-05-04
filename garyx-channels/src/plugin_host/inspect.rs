//! Install-time plugin inspection.
//!
//! The `garyx plugins install` CLI hands us a raw binary path and
//! expects back everything it needs to drop the plugin into the user's
//! install root — an auto-generated `plugin.toml`, the verified binary
//! filename, and the `DescribeResult` the operator might want to show
//! in logs.
//!
//! We achieve this by synthesising a *minimal* [`PluginManifest`]
//! that points at the user-provided binary (so `SubprocessPlugin::spawn`
//! can launch it) and then running the §6.3a handshake —
//! `initialize(dry_run=true)` → `describe` → `shutdown`. The describe
//! response alone carries enough metadata to generate a production
//! `plugin.toml` whose fields are guaranteed in sync with what the
//! plugin actually reports at runtime.
//!
//! Deliberately separate from [`super::preflight::preflight`]:
//! preflight *compares* runtime against an existing manifest, install
//! *produces* a new manifest from runtime. They share the wire dance
//! but neither is useful as the other's primitive.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tracing::warn;

use super::manifest::{
    AuthFlowDescriptor, DeliveryModel, ManifestCapabilities, ManifestRuntime, PluginEntry,
    PluginHeader, PluginManifest, PluginUi,
};
use super::preflight::PROTOCOL_VERSION;
use super::protocol::{
    CapabilitiesResponse, DescribeResult, InitializeResult, PluginErrorCode, PluginUiResponse,
};
use super::subprocess::{SpawnOptions, SubprocessError, SubprocessPlugin};
use super::transport::{InboundHandler, RpcError};

/// Longest a single inspect RPC may take. Matches preflight — a
/// plugin that stalls on describe during install won't survive a
/// real lifecycle either, and making the operator wait is worse.
const INSPECT_STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors surfaced by [`inspect`]. Deliberately explicit (rather than
/// reusing `PreflightFailure`) because install-time errors are about
/// the binary itself, not about manifest drift.
#[derive(Debug, Error)]
pub enum InspectError {
    #[error("binary not found at {0}")]
    BinaryMissing(PathBuf),
    #[error("binary at {0} is not a regular file")]
    BinaryNotAFile(PathBuf),
    #[error("binary at {path} is not executable and chmod failed: {source}")]
    BinaryNotExecutable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("spawn: {0}")]
    Spawn(#[from] SubprocessError),
    #[error("initialize rpc failed: {0}")]
    Initialize(#[source] RpcError),
    #[error("describe rpc failed: {0}")]
    Describe(#[source] RpcError),
    #[error(
        "plugin does not advertise protocol version {wanted}; advertises {got:?}. \
         Install anyway with --ignore-protocol if you know what you are doing."
    )]
    UnsupportedProtocol { wanted: u32, got: Vec<u32> },
    #[error(
        "plugin reports id '{describe}' in `describe` but id '{initialize}' in `initialize` — \
         the binary is inconsistent and cannot be safely installed"
    )]
    IdMismatch {
        initialize: String,
        describe: String,
    },
    #[error("plugin reports empty id")]
    EmptyId,
}

/// Everything `garyx plugins install` needs to stage a plugin into
/// the install root. Returned by [`inspect`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectReport {
    /// Plugin id the binary self-reports. The install root directory
    /// is named after this and the manifest `plugin.id` echoes it.
    pub id: String,
    /// Plugin version the binary self-reports (matches both
    /// `initialize.plugin.version` and `describe.plugin.version`).
    pub version: String,
    /// Protocol versions the plugin supports.
    pub protocol_versions: Vec<u32>,
    /// Capability bits — mirrors `describe.capabilities`.
    pub capabilities: CapabilitiesResponse,
    /// Auth flows the plugin advertises. Empty is legal (no
    /// device-code / OAuth flow is required).
    pub auth_flows: Vec<AuthFlowDescriptor>,
    /// Account-config schema. Copied verbatim into the synthesised
    /// manifest and later used by the desktop UI for schema-driven
    /// account forms (§11).
    pub schema: Value,
    /// Bot-console interaction hints that should survive install
    /// into the generated `plugin.toml`.
    pub ui: PluginUiResponse,
}

/// Install-time handler: the plugin is in dry-run mode, so inbound
/// traffic is a protocol violation. Drop anything we see with a loud
/// log.
struct InspectHandler;

#[async_trait]
impl InboundHandler for InspectHandler {
    async fn on_request(&self, method: String, _params: Value) -> Result<Value, (i32, String)> {
        warn!(method = %method, "inspect plugin sent inbound request during dry-run install");
        Err((
            PluginErrorCode::InvalidRequest.as_i32(),
            "dry-run plugins must not emit inbound traffic during install".into(),
        ))
    }

    async fn on_notification(&self, method: String, _params: Value) {
        warn!(method = %method, "inspect plugin sent notification during dry-run install");
    }
}

/// Spawn the binary, run `initialize(dry_run=true)` + `describe`, tear
/// it down, and return an [`InspectReport`].
///
/// On any failure we make a best-effort `shutdown_gracefully` before
/// returning so an install error doesn't leak child processes.
pub async fn inspect(binary_path: &Path) -> Result<InspectReport, InspectError> {
    // Resolve + sanity-check the binary BEFORE spawning so the error
    // message blames the right thing.
    let binary_path = binary_path
        .canonicalize()
        .map_err(|_| InspectError::BinaryMissing(binary_path.to_path_buf()))?;
    let metadata = std::fs::metadata(&binary_path)
        .map_err(|_| InspectError::BinaryMissing(binary_path.clone()))?;
    if !metadata.is_file() {
        return Err(InspectError::BinaryNotAFile(binary_path));
    }
    ensure_executable(&binary_path)?;

    let manifest = minimal_manifest(&binary_path);
    let plugin =
        SubprocessPlugin::spawn(&manifest, SpawnOptions::default(), Arc::new(InspectHandler))?;

    let initialize_params = json!({
        "protocol_version": PROTOCOL_VERSION,
        "host": {
            "version": env!("CARGO_PKG_VERSION"),
            "public_url": "",
            "data_dir": "",
        },
        "accounts": [],
        "dry_run": true,
    });
    let init: InitializeResult = match plugin
        .client()
        .call_with_timeout("initialize", &initialize_params, Some(INSPECT_STEP_TIMEOUT))
        .await
    {
        Ok(value) => value,
        Err(err) => {
            let _ = plugin.shutdown_gracefully().await;
            return Err(InspectError::Initialize(err));
        }
    };

    let describe: DescribeResult = match plugin
        .client()
        .call_with_timeout("describe", &json!({}), Some(INSPECT_STEP_TIMEOUT))
        .await
    {
        Ok(value) => value,
        Err(err) => {
            let _ = plugin.shutdown_gracefully().await;
            return Err(InspectError::Describe(err));
        }
    };

    let _ = plugin.shutdown_gracefully().await;

    if init.plugin.id != describe.plugin.id {
        return Err(InspectError::IdMismatch {
            initialize: init.plugin.id,
            describe: describe.plugin.id,
        });
    }
    if describe.plugin.id.trim().is_empty() {
        return Err(InspectError::EmptyId);
    }
    if !describe.protocol_versions.contains(&PROTOCOL_VERSION) {
        return Err(InspectError::UnsupportedProtocol {
            wanted: PROTOCOL_VERSION,
            got: describe.protocol_versions,
        });
    }

    Ok(InspectReport {
        id: describe.plugin.id,
        version: describe.plugin.version,
        protocol_versions: describe.protocol_versions,
        capabilities: describe.capabilities,
        auth_flows: describe.auth_flows,
        schema: describe.schema,
        ui: describe.ui,
    })
}

/// Produce a production-ready `plugin.toml` string for `report`.
/// The caller is expected to drop this next to the installed binary
/// (which keeps the filename `binary_filename` inside the install
/// root). If the plugin also ships a brand icon, pass its installed
/// filename (relative to the manifest dir) as `icon_filename`;
/// synthesize_manifest_toml emits it into `[plugin].icon` so the
/// gateway can surface it in `/api/channels/plugins/<id>/icon`.
///
/// Defaults for the sections `describe` doesn't populate:
/// - `runtime.*`: the ceilings the host accepts (5s / 3s).
/// - `capabilities.delivery_model`: `pull_explicit_ack` — the only
///   model production plugins use today; explicit override happens
///   by hand-editing the manifest post-install if needed.
pub fn synthesize_manifest_toml(
    report: &InspectReport,
    binary_filename: &str,
    icon_filename: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str("# Auto-generated by `garyx plugins install`. Hand-edit only if you know what you are doing.\n\
         # `plugin.id`, `plugin.version`, `capabilities.*`, `schema.*` and `[[auth_flows]]`\n\
         # are derived from the plugin's own `describe` response; keeping them in sync with\n\
         # the binary is the operator's responsibility. Re-running `garyx plugins install`\n\
         # regenerates this file idempotently.\n\n");
    out.push_str("[plugin]\n");
    out.push_str(&format!("id = {}\n", toml_string(&report.id)));
    out.push_str(&format!("version = {}\n", toml_string(&report.version)));
    out.push_str(&format!(
        "display_name = {}\n",
        toml_string(&title_case(&report.id))
    ));
    if let Some(icon) = icon_filename {
        out.push_str(&format!("icon = {}\n", toml_string(&format!("./{icon}"))));
    }
    out.push('\n');

    out.push_str("[entry]\n");
    out.push_str(&format!(
        "binary = {}\n",
        toml_string(&format!("./{binary_filename}"))
    ));
    out.push('\n');

    out.push_str("[capabilities]\n");
    out.push_str("delivery_model = \"pull_explicit_ack\"\n");
    out.push_str(&format!("outbound = {}\n", report.capabilities.outbound));
    out.push_str(&format!("inbound = {}\n", report.capabilities.inbound));
    out.push_str(&format!("streaming = {}\n", report.capabilities.streaming));
    out.push_str(&format!("images = {}\n", report.capabilities.images));
    out.push_str(&format!("files = {}\n", report.capabilities.files));
    out.push('\n');

    out.push_str("[runtime]\n");
    out.push_str("stop_grace_ms = 5000\n");
    out.push_str("shutdown_grace_ms = 3000\n");
    out.push('\n');

    for flow in &report.auth_flows {
        out.push_str("[[auth_flows]]\n");
        out.push_str(&format!("id = {}\n", toml_string(&flow.id)));
        out.push_str(&format!("label = {}\n", toml_string(&flow.label)));
        if !flow.prompt.is_empty() {
            out.push_str(&format!("prompt = {}\n", toml_string(&flow.prompt)));
        }
        out.push('\n');
    }

    out.push_str("[ui]\n");
    out.push_str(&format!(
        "account_root_behavior = {}\n\n",
        toml_string(match report.ui.account_root_behavior {
            super::manifest::AccountRootBehavior::OpenDefault => "open_default",
            super::manifest::AccountRootBehavior::ExpandOnly => "expand_only",
        })
    ));

    // Embed the schema verbatim. Going through serde_json → toml
    // keeps nested objects, arrays, and numeric types intact; a hand-
    // written TOML emitter would drift from what the plugin reports.
    out.push_str("[schema]\n");
    emit_schema_into_toml(&report.schema, "schema", &mut out);
    out
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Smallest PluginManifest that lets `SubprocessPlugin::spawn` resolve
/// the user-provided binary. Everything the handshake doesn't need is
/// defaulted — we'll throw this manifest away once `describe` returns.
fn minimal_manifest(binary_path: &Path) -> PluginManifest {
    let manifest_dir = binary_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let binary_file = binary_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    PluginManifest {
        manifest_dir,
        plugin: PluginHeader {
            // The id is about to be replaced from describe; the string
            // here is only used in host-side logs while inspect is
            // in flight and never touches disk.
            id: "__installing__".to_owned(),
            aliases: Vec::new(),
            version: "0.0.0".to_owned(),
            display_name: "pending-install".to_owned(),
            description: String::new(),
            icon: None,
        },
        ui: PluginUi::default(),
        entry: PluginEntry {
            binary: binary_file,
            env: BTreeMap::new(),
            args: Vec::new(),
        },
        capabilities: ManifestCapabilities {
            outbound: false,
            inbound: false,
            streaming: false,
            images: false,
            files: false,
            hot_reload_accounts: false,
            requires_public_url: false,
            needs_host_ingress: false,
            delivery_model: DeliveryModel::PullExplicitAck,
        },
        runtime: ManifestRuntime::default(),
        schema: Value::Object(Default::default()),
        auth_flows: Vec::new(),
        min_host_version: None,
    }
}

#[cfg(unix)]
fn ensure_executable(path: &Path) -> Result<(), InspectError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).map_err(|source| InspectError::BinaryNotExecutable {
        path: path.to_path_buf(),
        source,
    })?;
    let mode = metadata.permissions().mode();
    if mode & 0o111 == 0 {
        let mut perms = metadata.permissions();
        perms.set_mode(mode | 0o755);
        std::fs::set_permissions(path, perms).map_err(|source| {
            InspectError::BinaryNotExecutable {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_executable(_path: &Path) -> Result<(), InspectError> {
    // Windows: the OS doesn't carry a POSIX execute bit — trust the
    // caller. If the binary is wrong the subsequent spawn will fail
    // with a clearer error than we could synthesise here.
    Ok(())
}

/// Minimal TOML string quoter: escapes `\` and `"` so an
/// "unfriendly" plugin id (slashes, quotes, newlines) survives
/// round-trip. Deliberately narrower than `toml::to_string` to keep
/// this module dependency-light.
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn title_case(s: &str) -> String {
    let mut first = true;
    s.chars()
        .map(|c| {
            if first {
                first = false;
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect()
}

/// Walk `value` and emit its members under the TOML table rooted at
/// `path`. Nested objects become sub-tables (`[schema.properties.token]`);
/// arrays of scalars become inline arrays; arrays of tables use
/// `[[path.name]]`.
fn emit_schema_into_toml(value: &Value, path: &str, out: &mut String) {
    let Some(obj) = value.as_object() else {
        // Non-object schema (weird but legal) — emit as a single
        // inline value under `[schema]`.
        out.push_str(&format!("value = {}\n", json_to_toml_value(value)));
        return;
    };
    // First pass: emit scalars and inline arrays under the current
    // table header.
    for (k, v) in obj {
        match v {
            Value::Object(_) => {}
            _ => {
                out.push_str(&format!("{} = {}\n", quote_key(k), json_to_toml_value(v)));
            }
        }
    }
    // Second pass: recurse into nested objects as sub-tables.
    for (k, v) in obj {
        if let Value::Object(_) = v {
            let sub = format!("{path}.{}", quote_key(k));
            out.push_str(&format!("\n[{sub}]\n"));
            emit_schema_into_toml(v, &sub, out);
        }
    }
}

fn json_to_toml_value(value: &Value) -> String {
    match value {
        Value::Null => "\"\"".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => toml_string(s),
        Value::Array(items) => {
            let encoded: Vec<String> = items.iter().map(json_to_toml_value).collect();
            format!("[{}]", encoded.join(", "))
        }
        Value::Object(_) => {
            // Inline object — rarely appears in JSON Schema but
            // keeps round-trip total-ish.
            format!("{{ {} }}", inline_object(value))
        }
    }
}

fn inline_object(value: &Value) -> String {
    let Some(obj) = value.as_object() else {
        return String::new();
    };
    obj.iter()
        .map(|(k, v)| format!("{} = {}", quote_key(k), json_to_toml_value(v)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn quote_key(key: &str) -> String {
    // Bare keys are ASCII letters/digits/underscore/hyphen. Anything
    // else gets quoted — matches TOML 1.0 §2.1.
    if key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !key.is_empty()
    {
        key.to_owned()
    } else {
        toml_string(key)
    }
}

#[cfg(test)]
mod tests;
