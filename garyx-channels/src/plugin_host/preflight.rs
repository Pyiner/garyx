//! Dry-run preflight for plugins.
//!
//! Per §6.3a / §13 of the protocol doc, before the `ChannelPluginManager`
//! is wired to live routing we spawn each discovered plugin with
//! `initialize.dry_run = true`, call `describe`, then `shutdown`. This
//! gives us three guarantees for free before any user-visible account
//! is bound:
//! 1. The binary actually spawns (catches chmod / architecture issues).
//! 2. The plugin speaks the expected protocol version.
//! 3. The manifest's `schema`/`auth_flows` match what the plugin
//!    actually declares at runtime.
//!
//! If any check fails the caller gets a [`PreflightFailure`] with
//! enough context to surface as a channel-level error without blocking
//! unrelated plugins.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tracing::warn;

use super::manifest::{AuthFlowDescriptor, ManifestCapabilities, PluginManifest};
use super::protocol::{CapabilitiesResponse, DescribeResult, InitializeResult};
use super::subprocess::{SpawnOptions, SubprocessError, SubprocessPlugin};
use super::transport::{InboundHandler, RpcError};

/// The protocol version this host speaks. Bump whenever a breaking
/// wire change lands. See §2.1 for the compatibility matrix.
pub const PROTOCOL_VERSION: u32 = 1;

/// One RPC unreachable → preflight fails. This is intentionally stricter
/// than runtime: a plugin that can't answer describe during a preflight
/// won't survive a respawn either.
#[derive(Debug, Error)]
pub enum PreflightFailure {
    #[error("spawn: {0}")]
    Spawn(#[from] SubprocessError),
    #[error("initialize rpc failed: {0}")]
    Initialize(RpcError),
    #[error("describe rpc failed: {0}")]
    Describe(RpcError),
    #[error("plugin does not advertise protocol version {wanted}; advertises {got:?}")]
    UnsupportedProtocol { wanted: u32, got: Vec<u32> },
    #[error("plugin id in manifest ({manifest}) does not match describe response ({runtime})")]
    PluginIdMismatch { manifest: String, runtime: String },
    #[error(
        "schema drift: manifest and runtime disagree on the account config schema. \
         manifest_schema={manifest}, runtime_schema={runtime}"
    )]
    SchemaMismatch { manifest: Value, runtime: Value },
    #[error("auth flow drift: manifest declares {manifest:?}, runtime declares {runtime:?}")]
    AuthFlowMismatch {
        manifest: Vec<AuthFlowDescriptor>,
        runtime: Vec<AuthFlowDescriptor>,
    },
    #[error(
        "capability drift: manifest declares {manifest_capability}={manifest_value}, \
         runtime declares {runtime_value}"
    )]
    CapabilityMismatch {
        manifest_capability: &'static str,
        manifest_value: bool,
        runtime_value: bool,
    },
}

/// Summary of what a plugin declared during preflight. Used by the
/// desktop UI for schema-driven channel configuration (§11).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightSummary {
    pub id: String,
    pub version: String,
    pub protocol_versions: Vec<u32>,
    pub schema: Value,
    pub auth_flows: Vec<super::manifest::AuthFlowDescriptor>,
    pub capabilities: super::protocol::CapabilitiesResponse,
}

/// Preflight handler: ignores every inbound RPC. The plugin is in
/// `dry_run` mode so it MUST NOT send inbound traffic; if it does, we
/// log and drop.
struct PreflightHandler;

#[async_trait]
impl InboundHandler for PreflightHandler {
    async fn on_request(&self, method: String, _params: Value) -> Result<Value, (i32, String)> {
        warn!(method = %method, "preflight plugin sent inbound request during dry-run");
        Err((
            super::protocol::PluginErrorCode::InvalidRequest.as_i32(),
            "dry-run plugins must not emit inbound traffic".to_owned(),
        ))
    }

    async fn on_notification(&self, method: String, _params: Value) {
        warn!(method = %method, "preflight plugin sent notification during dry-run");
    }
}

/// How long a single preflight step may take. Preflight must be fast;
/// if a plugin sits on describe for more than a few seconds we'd
/// rather fail the bootstrap than stall channel wiring for everyone.
const PREFLIGHT_STEP_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn preflight(
    manifest: &PluginManifest,
    host_version: &str,
    data_dir: &str,
    public_url: &str,
) -> Result<PreflightSummary, PreflightFailure> {
    let plugin = SubprocessPlugin::spawn(
        manifest,
        SpawnOptions::default(),
        Arc::new(PreflightHandler),
    )?;

    let initialize_params = json!({
        "protocol_version": PROTOCOL_VERSION,
        "host": {
            "version": host_version,
            "public_url": public_url,
            "data_dir": data_dir,
        },
        "accounts": [],
        "dry_run": true,
    });
    let init: InitializeResult = plugin
        .client()
        .call_with_timeout(
            "initialize",
            &initialize_params,
            Some(PREFLIGHT_STEP_TIMEOUT),
        )
        .await
        .map_err(PreflightFailure::Initialize)?;

    let describe: DescribeResult = plugin
        .client()
        .call_with_timeout("describe", &json!({}), Some(PREFLIGHT_STEP_TIMEOUT))
        .await
        .map_err(PreflightFailure::Describe)?;

    if !describe.protocol_versions.contains(&PROTOCOL_VERSION) {
        let _ = plugin.shutdown_gracefully().await;
        return Err(PreflightFailure::UnsupportedProtocol {
            wanted: PROTOCOL_VERSION,
            got: describe.protocol_versions,
        });
    }

    if describe.plugin.id != manifest.plugin.id {
        let _ = plugin.shutdown_gracefully().await;
        return Err(PreflightFailure::PluginIdMismatch {
            manifest: manifest.plugin.id.clone(),
            runtime: describe.plugin.id,
        });
    }

    // §6.3a / §13: the whole point of the dry-run is to catch
    // manifest/runtime drift before the desktop UI or live lifecycle
    // cements the wrong shape. Compare the three observable surfaces:
    // schema, auth flows, and overlapping capability bits. A plugin
    // that ships a stale manifest has to fix one side before preflight
    // will pass.
    if !json_equivalent(&manifest.schema, &describe.schema) {
        let _ = plugin.shutdown_gracefully().await;
        return Err(PreflightFailure::SchemaMismatch {
            manifest: manifest.schema.clone(),
            runtime: describe.schema,
        });
    }
    if !auth_flows_equivalent(&manifest.auth_flows, &describe.auth_flows) {
        let _ = plugin.shutdown_gracefully().await;
        return Err(PreflightFailure::AuthFlowMismatch {
            manifest: manifest.auth_flows.clone(),
            runtime: describe.auth_flows,
        });
    }
    if let Some((name, manifest_value, runtime_value)) =
        capability_mismatch(&manifest.capabilities, &describe.capabilities)
    {
        let _ = plugin.shutdown_gracefully().await;
        return Err(PreflightFailure::CapabilityMismatch {
            manifest_capability: name,
            manifest_value,
            runtime_value,
        });
    }

    let summary = PreflightSummary {
        id: describe.plugin.id,
        version: describe.plugin.version,
        protocol_versions: describe.protocol_versions,
        schema: describe.schema,
        auth_flows: describe.auth_flows,
        capabilities: describe.capabilities,
    };

    // Drain plugin gracefully. We don't care about its exit report
    // after a successful preflight — if shutdown escalates to SIGKILL
    // the subprocess module warns about it, and the plugin binary
    // still told us what we needed to know.
    let _exit = plugin.shutdown_gracefully().await;
    // Silence unused `_init` warning while keeping the type-check.
    let _ = init;
    Ok(summary)
}

/// Compare two JSON values *structurally*, treating JSON object field
/// order as irrelevant (serde_json preserves insertion order so
/// `Value::eq` is order-sensitive). Arrays are compared positionally
/// since JSON Schema keywords like `required` and `prefixItems` are
/// meaningfully ordered.
fn json_equivalent(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            if x.len() != y.len() {
                return false;
            }
            x.iter().all(|(k, v)| match y.get(k) {
                Some(other) => json_equivalent(v, other),
                None => false,
            })
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(l, r)| json_equivalent(l, r))
        }
        _ => a == b,
    }
}

fn auth_flows_equivalent(manifest: &[AuthFlowDescriptor], runtime: &[AuthFlowDescriptor]) -> bool {
    if manifest.len() != runtime.len() {
        return false;
    }
    // Order-insensitive: the manifest's ordering is a presentation
    // preference, not a protocol commitment.
    let mut manifest_sorted: Vec<&AuthFlowDescriptor> = manifest.iter().collect();
    let mut runtime_sorted: Vec<&AuthFlowDescriptor> = runtime.iter().collect();
    manifest_sorted.sort_by(|a, b| a.id.cmp(&b.id));
    runtime_sorted.sort_by(|a, b| a.id.cmp(&b.id));
    manifest_sorted == runtime_sorted
}

/// Compare the five capability bits that appear in *both* manifest
/// (`ManifestCapabilities`) and runtime (`CapabilitiesResponse`). The
/// manifest has extra fields (`delivery_model`, `hot_reload_accounts`
/// etc.) that the runtime does not echo back, so we ignore those.
fn capability_mismatch(
    manifest: &ManifestCapabilities,
    runtime: &CapabilitiesResponse,
) -> Option<(&'static str, bool, bool)> {
    for (name, m, r) in [
        ("outbound", manifest.outbound, runtime.outbound),
        ("inbound", manifest.inbound, runtime.inbound),
        ("streaming", manifest.streaming, runtime.streaming),
        ("images", manifest.images, runtime.images),
        ("files", manifest.files, runtime.files),
    ] {
        if m != r {
            return Some((name, m, r));
        }
    }
    None
}
