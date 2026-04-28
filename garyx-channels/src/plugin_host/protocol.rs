//! Wire types for the channel-plugin JSON-RPC protocol.
//!
//! One type per RPC method, plus the error-code catalogue. Every type
//! derives `Serialize` + `Deserialize` and matches the JSON shapes used by
//! the subprocess protocol.
//!
//! Deliberate choices:
//! - The public-URL/data-dir shape in [`InitializeParams::host`] is the
//!   only place the plugin sees host context; nothing else leaks.
//! - Inbound attachments carry either inline bytes *or* a path; §11.2
//!   normatively pushes large media through `file_paths`.
//! - `StreamEventFrame` is `#[serde(tag = "type")]` so adding a new
//!   event variant is source-compatible for plugins that handle the
//!   existing ones with a `match _` arm.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::manifest::AccountRootBehavior;

// -- Error codes ------------------------------------------------------------

/// Numeric error codes carried in JSON-RPC `error.code` per §5.3 of
/// the protocol. Exposed as `i32` because that is what the wire
/// format uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PluginErrorCode {
    // Reserved JSON-RPC range.
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,

    // garyx-specific range.
    NotInitialized = -32000,
    AlreadyInitialized = -32001,
    AccountNotFound = -32002,
    HostShuttingDown = -32003,
    PluginShuttingDown = -32004,
    /// Lifecycle-time refusal (initialize / describe). Fatal.
    ConfigRejected = -32005,
    /// Retryable: receiver at capacity.
    Busy = -32006,
    /// Per-message refusal from dispatch_outbound. Non-fatal; caller's
    /// retry policy applies.
    ChannelConfigRejected = -32007,
    /// Fatal, non-retryable: frame exceeds `max_frame_bytes` or inline
    /// attachment exceeds its cap.
    PayloadTooLarge = -32008,
}

impl PluginErrorCode {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    pub fn from_i32(code: i32) -> Option<Self> {
        Some(match code {
            -32700 => Self::ParseError,
            -32600 => Self::InvalidRequest,
            -32601 => Self::MethodNotFound,
            -32602 => Self::InvalidParams,
            -32603 => Self::InternalError,
            -32000 => Self::NotInitialized,
            -32001 => Self::AlreadyInitialized,
            -32002 => Self::AccountNotFound,
            -32003 => Self::HostShuttingDown,
            -32004 => Self::PluginShuttingDown,
            -32005 => Self::ConfigRejected,
            -32006 => Self::Busy,
            -32007 => Self::ChannelConfigRejected,
            -32008 => Self::PayloadTooLarge,
            _ => return None,
        })
    }

    /// Is this error retryable by the caller? See §5.3 / §9.4 mapping
    /// table.
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Busy)
    }
}

// -- Initialize (host → plugin) --------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    pub protocol_version: u32,
    pub host: HostContext,
    #[serde(default)]
    pub accounts: Vec<AccountDescriptor>,
    /// §6.3a dry-run mode. When true, `accounts` MUST be empty and the
    /// plugin MUST answer `start`/`stop`/etc. with NotInitialized.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dry_run: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostContext {
    pub version: String,
    #[serde(default)]
    pub public_url: String,
    pub data_dir: String,
    #[serde(default)]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountDescriptor {
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub config: Value,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub plugin: PluginIdentity,
    pub capabilities: CapabilitiesResponse,
    #[serde(default)]
    pub ui: PluginUiResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginIdentity {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitiesResponse {
    #[serde(default)]
    pub outbound: bool,
    #[serde(default)]
    pub inbound: bool,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub images: bool,
    #[serde(default)]
    pub files: bool,
}

// -- Describe (host → plugin, dry-run) -------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeResult {
    pub plugin: PluginIdentity,
    pub protocol_versions: Vec<u32>,
    pub schema: Value,
    #[serde(default)]
    pub auth_flows: Vec<super::manifest::AuthFlowDescriptor>,
    pub capabilities: CapabilitiesResponse,
    #[serde(default)]
    pub ui: PluginUiResponse,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginUiResponse {
    #[serde(default)]
    pub account_root_behavior: AccountRootBehavior,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveAccountUiParams {
    pub account_id: String,
    #[serde(default)]
    pub endpoints: Vec<UiEndpointDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiEndpointDescriptor {
    pub endpoint_key: String,
    pub channel: String,
    pub account_id: String,
    pub binding_key: String,
    pub chat_id: String,
    pub delivery_target_type: String,
    pub delivery_target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_thread_id: Option<String>,
    pub display_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_inbound_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_delivery_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolveAccountUiResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_open_endpoint_key: Option<String>,
    #[serde(default)]
    pub conversation_nodes: Vec<UiConversationNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConversationNode {
    pub id: String,
    pub endpoint_key: String,
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_activity: Option<String>,
    #[serde(default)]
    pub openable: bool,
}

// -- Inbound (plugin → host) -----------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundRequestPayload {
    pub account_id: String,
    pub from_id: String,
    #[serde(default)]
    pub is_group: bool,
    /// Key that pins this message to a specific thread. May be
    /// `issue_id`, `chat_id`, or any plugin-chosen stable string.
    pub thread_binding_key: String,
    pub message: String,
    /// Plugin-scoped id used in logs and for `abandon_inbound` /
    /// record_outbound bookkeeping on the plugin side.
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub images: Vec<AttachmentRef>,
    #[serde(default)]
    pub file_paths: Vec<String>,
    #[serde(default)]
    pub extra_metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttachmentRef {
    /// Inline bytes; limited to 4 MiB per attachment, 8 MiB per
    /// frame (clamped to `max_frame_bytes`).
    Inline { data: String, media_type: String },
    /// Path into the shared attachments directory. Path MUST be
    /// inside `host.data_dir/attachments/inbound/`.
    Path { path: String, media_type: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundRequestResult {
    pub stream_id: String,
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_reply: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamFrameParams {
    pub stream_id: String,
    pub seq: u64,
    pub event: StreamEventFrame,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEventFrame {
    Delta {
        text: String,
    },
    Boundary {
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// Tool-call narration or other host-emitted metadata.
    Meta {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        detail: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundEnd {
    pub stream_id: String,
    pub seq: u64,
    pub status: InboundEndStatus,
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub final_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InboundEndStatus {
    Ok(OkStatus),
    Err { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkStatus {
    Ok,
}

impl InboundEndStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }

    pub fn ok() -> Self {
        Self::Ok(OkStatus::Ok)
    }

    pub fn error(reason: impl Into<String>) -> Self {
        Self::Err {
            error: reason.into(),
        }
    }
}

// -- abandon_inbound (plugin → host) ---------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbandonInboundParams {
    pub stream_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbandonInboundResult {
    pub ok: bool,
}

// -- dispatch_outbound (host → plugin) -------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchOutbound {
    pub account_id: String,
    pub chat_id: String,
    pub delivery_target_type: String,
    pub delivery_target_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchOutboundResult {
    pub message_ids: Vec<String>,
}

// -- accounts/reload (host → plugin) ---------------------------------------
//
// Host-pushed account refresh. The source of truth for account state
// is `ChannelsConfig` on the host; plugins only *reflect* the account
// set the host hands them. `initialize` (§6.1) seeds the starting
// list; once the child is up, config edits on the host push a fresh
// list via this single RPC instead of triggering a respawn for every
// account change. See §6.5.
//
// Semantics: the parameter list REPLACES the plugin's live set —
// accounts present in the new list are upserted (same `id` ⇒ config
// and enabled bits are refreshed), accounts absent from the list are
// torn down. Plugins that can't cheaply reflect in place MAY return
// `ConfigRejected` (§5.3); the host falls back to a full respawn.
// Response is an empty `{}`.

/// host → plugin: replace the plugin's live account set with
/// `accounts`. Mirrors the shape of `initialize.params.accounts` so
/// handlers that already parse `AccountDescriptor` can share code
/// between the two entry points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReloadAccountsParams {
    #[serde(default)]
    pub accounts: Vec<AccountDescriptor>,
}

/// host → plugin: validate one account config before the host persists
/// it. Plugins that cannot safely probe connectivity should return
/// `validated=false` with a short explanatory message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateAccountParams {
    pub account: AccountDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateAccountResult {
    #[serde(default)]
    pub validated: bool,
    #[serde(default)]
    pub message: String,
}

// -- record_outbound (plugin → host) ---------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordOutbound {
    pub thread_id: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub message_id: String,
}

// -- register_ingress (plugin → host, notification) ------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterIngressParams {
    pub account_id: String,
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default)]
    pub local_url: Option<String>,
}

// -- auth_flow/* (host → plugin) -------------------------------------------
//
// These DTOs mirror the in-process `auth_flow::AuthFlowExecutor` trait
// types 1:1 on the wire. Separate names (`AuthFlowStartRequest` vs
// `AuthSession`, `AuthFlowDisplayItem` vs `AuthDisplayItem`) keep the
// cross-process RPC layer distinguishable from the in-process trait,
// but the JSON shapes must stay identical so a BuiltinPluginAdapter
// can forward without reshaping.

/// RPC request: start a new auto-login session.
/// Mirrors `AuthFlowExecutor::start(form_state)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthFlowStartRequest {
    /// Whatever the user has typed into the schema form so far;
    /// may be an empty object. The plugin picks out the fields it
    /// needs and applies its own defaults for the rest.
    #[serde(default)]
    pub form_state: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthFlowPollRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthFlowCancelRequest {
    pub session_id: String,
}

/// RPC response for `auth_flow/start`. Mirrors `auth_flow::AuthSession`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthFlowStartResponse {
    pub session_id: String,
    pub display: Vec<AuthFlowDisplayItem>,
    pub expires_in_secs: u64,
    pub poll_interval_secs: u64,
}

/// RPC response for `auth_flow/poll`. Mirrors `auth_flow::AuthPollResult`.
/// Three outcomes: still working, done, failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AuthFlowPollResponse {
    /// Keep polling. If `display` is `Some`, the UI MUST replace
    /// the current render; if `next_interval_secs` is set,
    /// subsequent polls MUST use it instead of the original interval.
    Pending {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<Vec<AuthFlowDisplayItem>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_interval_secs: Option<u64>,
    },
    /// Terminal success. `values` is a partial account-config patch
    /// the UI merges into its form.
    Confirmed { values: BTreeMap<String, Value> },
    /// Terminal failure. The executor has given up.
    Failed { reason: String },
}

/// One renderable display item. Mirrors `auth_flow::AuthDisplayItem`.
/// The UI walks the vec in order and renders each item per its kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthFlowDisplayItem {
    /// Verbatim text. URLs go here too — the UI may auto-linkify.
    Text { value: String },
    /// Content to encode as a QR code. Pure text payload; the UI
    /// renders the QR itself. Graphical UIs should show this
    /// payload below the QR and make it copyable; http(s) payloads
    /// should also be clickable/openable.
    Qr { value: String },
    /// Forward-compatibility fallback for UIs that see a newer kind.
    #[serde(other)]
    Unknown,
}

// -- tests -----------------------------------------------------------------

#[cfg(test)]
mod tests;
