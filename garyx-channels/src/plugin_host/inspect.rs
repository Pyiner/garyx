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
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Optional `[update]` block carried over from the bundle's
    /// `plugin.toml`. Round-tripped verbatim into the synthesized
    /// manifest so `garyx plugins update` has a source without
    /// network round-trips.
    pub update: Option<super::manifest::PluginUpdate>,
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

    // Surface the bundle's [update] block, if any. The bundle layout
    // is "binary + plugin.toml in the same dir" — same convention the
    // installer relies on for icon files. A malformed bundle manifest
    // must not block installs, so load failures emit a `warn` trace
    // (observable from gateway logs) but do not propagate.
    let update = binary_path.parent().and_then(|dir| {
        let manifest_path = dir.join("plugin.toml");
        if !manifest_path.is_file() {
            return None;
        }
        match super::manifest::PluginManifest::load(&manifest_path) {
            Ok(m) => m.update,
            Err(err) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %err,
                    "failed to read bundle plugin.toml; [update] block will be omitted",
                );
                None
            }
        }
    });

    Ok(InspectReport {
        id: describe.plugin.id,
        version: describe.plugin.version,
        protocol_versions: describe.protocol_versions,
        capabilities: describe.capabilities,
        auth_flows: describe.auth_flows,
        schema: describe.schema,
        ui: describe.ui,
        update,
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
    out.push_str(&format!(
        "survives_respawn = {}\n",
        report.capabilities.survives_respawn
    ));
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

    if let Some(update) = &report.update {
        out.push_str("[update]\n");
        if let Some(m) = &update.manifest_url {
            out.push_str(&format!("manifest_url = {}\n", toml_string(m)));
        }
        out.push_str(&format!(
            "url_template = {}\n",
            toml_string(&update.url_template),
        ));
        if let Some(c) = &update.checksum_url_template {
            out.push_str(&format!("checksum_url_template = {}\n", toml_string(c),));
        }
        if let Some(b) = &update.binary_in_archive {
            out.push_str(&format!("binary_in_archive = {}\n", toml_string(b),));
        }
        out.push('\n');
    }

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
            dispatch_stream_event: false,
            images: false,
            files: false,
            hot_reload_accounts: false,
            requires_public_url: false,
            needs_host_ingress: false,
            survives_respawn: false,
            delivery_model: DeliveryModel::PullExplicitAck,
        },
        runtime: ManifestRuntime::default(),
        schema: Value::Object(Default::default()),
        auth_flows: Vec::new(),
        update: None,
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

/// Result of [`backfill_survives_respawn_in_place`].
#[derive(Debug, PartialEq, Eq)]
pub enum BackfillOutcome {
    /// Wrote `survives_respawn = true` into the on-disk `[capabilities]`
    /// block. The plugin manifest now permits silent self-update.
    Wrote,
    /// The field was already present (any value). Left untouched —
    /// an explicit `survives_respawn = false` is an operator opt-out
    /// and must be respected.
    AlreadyPresent,
}

/// Patch an installed `plugin.toml` to add `survives_respawn = true`
/// into its `[capabilities]` block when (a) the field is missing
/// entirely (synthesized by an old garyx where the renderer dropped
/// it) and (b) the running plugin advertises it via `describe`. Used
/// by the host startup self-heal path so existing installs unstick
/// themselves on first launch after upgrading garyx, without
/// requiring the operator to re-run `garyx plugins install --force`.
///
/// Caller is responsible for the (b) check — this function just does
/// the textual patch when (a) holds.
///
/// Write is atomic: emit to a sibling `.tmp` file then `rename`. If
/// any step fails the original is untouched.
pub fn backfill_survives_respawn_in_place(path: &Path) -> io::Result<BackfillOutcome> {
    let original = fs::read_to_string(path)?;
    let lines: Vec<&str> = original.lines().collect();

    // Locate the [capabilities] section: only the first occurrence.
    // A malformed file with duplicate `[capabilities]` headers would
    // already be rejected by the TOML loader downstream.
    let Some(caps_start) = lines.iter().position(|l| l.trim() == "[capabilities]") else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plugin.toml has no [capabilities] section to backfill",
        ));
    };

    // End of `[capabilities]` table = first subsequent line that's
    // a new section header (`[next_table]` or `[capabilities.sub]`),
    // or EOF. **Blank lines do NOT end a TOML table** — earlier
    // versions of this code treated them as boundaries and would
    // silently miss a `survives_respawn = false` opt-out that sat
    // after a stylistic blank inside the block, then write a
    // duplicate key and produce malformed TOML.
    let caps_end = lines
        .iter()
        .enumerate()
        .skip(caps_start + 1)
        .find(|(_, l)| l.trim().starts_with('['))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());

    // Look for an explicit `survives_respawn = …` key anywhere in
    // the block (not just comments mentioning the name). Operator
    // intent to OPT OUT (`survives_respawn = false`) must be
    // preserved verbatim.
    for line in &lines[caps_start + 1..caps_end] {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("survives_respawn") {
            // Require an actual `=` after optional whitespace so
            // unrelated identifiers don't false-match: e.g.
            // `survives_respawnish = true` leaves `ish = true`
            // (rejected), and `survives_respawn.subkey = true`
            // leaves `.subkey = true` (also rejected — dotted-key
            // forms target a different namespace and aren't the
            // scalar bool field we're managing).
            let rest = rest.trim_start();
            if rest.starts_with('=') {
                return Ok(BackfillOutcome::AlreadyPresent);
            }
        }
    }

    // Insertion point: directly after the last actual key=value
    // line in the block (skip trailing blanks / comments so the
    // new line lands flush with the existing keys, matching what
    // `synthesize_manifest_toml` would have written from scratch).
    // Default to `caps_start + 1` when the block has no content
    // (the header was the trailing line of the file, or the next
    // section header sits immediately below it).
    let mut insert_at = caps_start + 1;
    for (i, line) in lines.iter().enumerate().take(caps_end).skip(caps_start + 1) {
        let t = line.trim();
        if !t.is_empty() && !t.starts_with('#') {
            insert_at = i + 1;
        }
    }

    // Build output. Walk every original line, append our key
    // verbatim at `insert_at`, then continue. When `insert_at`
    // equals `lines.len()` (capabilities is trailing with no
    // following content) emit it after the loop.
    let mut out = String::with_capacity(original.len() + 32);
    for (i, line) in lines.iter().enumerate() {
        if i == insert_at {
            out.push_str("survives_respawn = true\n");
        }
        out.push_str(line);
        out.push('\n');
    }
    if insert_at == lines.len() {
        out.push_str("survives_respawn = true\n");
    }

    // Atomic write: stage at sibling .tmp then rename so a crash
    // mid-write can't leave a half-written plugin.toml that the
    // host would refuse to load on next startup. PID + nanosecond
    // suffix prevents two concurrent garyx instances (e.g. launchd
    // + manual `gateway run`) from racing on the same temp path,
    // and `create_new` (O_EXCL) makes a still-impossible collision
    // surface as EEXIST instead of silently overwriting a peer's
    // staging file.
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "plugin.toml path has no parent directory",
        )
    })?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("plugin.toml");
    // PID handles cross-process collisions; the in-process atomic
    // counter handles same-process concurrent calls (two threads
    // calling backfill at the same nanosecond would otherwise read
    // the same SystemTime and collide on `create_new` below).
    static BACKFILL_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = BACKFILL_SEQ.fetch_add(1, Ordering::Relaxed);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(
        ".{file_name}.backfill.{}.{}.{}.tmp",
        std::process::id(),
        seq,
        nonce,
    ));
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(out.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(BackfillOutcome::Wrote)
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
