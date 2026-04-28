//! Schema-driven channel configuration.
//!
//! There are a fixed number of ways to hand a channel its
//! credentials. Each channel's [`PluginMetadata`] carries a
//! `config_methods: Vec<ConfigMethod>` list saying which ones it
//! supports — the UI reads that list, renders each method in order,
//! and stays channel-blind.
//!
//! Today two methods exist:
//!
//! 1. [`ConfigMethod::Form`] — render the plugin's JSON Schema as a
//!    form. Required fields / defaults / conditional requires all
//!    come from the schema; the protocol does not duplicate that
//!    logic. Telegram is the purest example: `[Form]` and nothing
//!    else.
//! 2. [`ConfigMethod::AutoLogin`] — show a button that calls
//!    [`AuthFlowExecutor::start`]. The plugin drives its own state
//!    machine, pushes display items (text / QR) to the UI, and on
//!    success returns a `values` patch the UI merges back into the
//!    form. Feishu and weixin list `[Form, AutoLogin]`
//!    so the form remains the fallback if the auto flow fails.
//!
//! Adding a third config method is a matter of landing a new enum
//! variant + a new trait (or a new method here) — the shape
//! generalises.
//!
//! **What the protocol deliberately does NOT carry:**
//!
//! - No `flow_id`. The UI doesn't pick between "device code" /
//!   "qr code" / etc. — those are internal to the plugin. One
//!   `AutoLogin` entry, one `start()` call.
//! - No channel-specific poll states (scanned / slow_down / denied
//!   / expired). State machines stay inside the channel; the
//!   protocol carries three outcomes: still working, done, failed.
//! - No distinction between "text" and "URL" in the display — both
//!   are `Text(String)`. The UI may auto-linkify if it wants; the
//!   plugin decides layout by ordering items in the `display` vec.
//!
//! Usage (channel-blind caller):
//!
//!   let session = executor.start(form_state).await?;
//!   render(&session.display);
//!   loop {
//!       sleep(session.poll_interval_secs).await;
//!       match executor.poll(&session.session_id).await? {
//!           Pending { display, next_interval_secs } => {
//!               if let Some(d) = display { render(&d); }
//!               if let Some(i) = next_interval_secs { interval = i; }
//!           }
//!           Confirmed { values } => { merge_into_form(values); break; }
//!           Failed { reason }    => { show_error(reason); break; }
//!       }
//!   }

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Configuration methods a channel advertises through its catalog
/// entry. Serialised as an externally-tagged JSON object
/// (`{"kind":"form"}` / `{"kind":"auto_login"}`) so adding a third
/// variant later is forward-compatible: older callers that don't
/// recognise the tag ignore it via `serde(other)` below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigMethod {
    /// Plain schema-driven form. Required fields, defaults, and
    /// conditional requires all come from the plugin's JSON Schema.
    Form,
    /// Automated login — UI renders a button that invokes the
    /// channel's [`AuthFlowExecutor`]. On success, the result's
    /// `values` are merged into the form the user can still review.
    AutoLogin,
    /// Forward-compatibility fallback: a variant the local build
    /// doesn't recognise. UI MUST render nothing for this entry.
    #[serde(other)]
    Unknown,
}

/// One renderable item in an [`AuthSession::display`] list. The UI
/// walks the vec in order and renders each item per its kind. New
/// items (e.g. an embedded image) can be added as enum variants
/// without breaking the protocol — older UIs see `Unknown` and skip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthDisplayItem {
    /// Verbatim text the UI shows as a paragraph. URLs go here too
    /// — the UI is free to auto-linkify; the protocol doesn't
    /// distinguish "plain" from "link" text because plugins already
    /// control layout by ordering the vec.
    Text { value: String },
    /// Content to encode as a QR code. Pure text payload — the UI
    /// renders the QR itself (native widget / canvas / terminal
    /// block characters for the CLI). Graphical UIs should also
    /// show this payload below the QR and make it copyable; if it
    /// is an http(s) URL, it should be clickable/openable as the
    /// non-camera fallback. No pre-rendered PNG: every surface
    /// already has a capable QR renderer, and shipping a PNG forces
    /// a fixed resolution the UI can't scale.
    Qr { value: String },
    /// Forward-compatibility fallback.
    #[serde(other)]
    Unknown,
}

impl AuthDisplayItem {
    /// Convenience: `AuthDisplayItem::text("...")`.
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text {
            value: value.into(),
        }
    }

    /// Convenience: `AuthDisplayItem::qr("...")`.
    pub fn qr(value: impl Into<String>) -> Self {
        Self::Qr {
            value: value.into(),
        }
    }
}

/// Return value of [`AuthFlowExecutor::start`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    /// Opaque id the caller passes to [`AuthFlowExecutor::poll`].
    /// Executors may encode internal state in here (device_code,
    /// tenant brand, derived poll URL, ...) — the caller treats it
    /// as a cookie.
    pub session_id: String,
    /// Ordered list of items the UI should render. UIs are REQUIRED
    /// to render them top-to-bottom; plugins control layout via
    /// ordering.
    pub display: Vec<AuthDisplayItem>,
    /// Total TTL of the session. After this the caller should stop
    /// polling and surface a timeout.
    pub expires_in_secs: u64,
    /// Starting poll cadence. A subsequent
    /// [`AuthPollResult::Pending`] may bump this via
    /// `next_interval_secs` (feishu's `slow_down`, etc.).
    pub poll_interval_secs: u64,
}

/// Outcome of one [`AuthFlowExecutor::poll`] tick. Three states by
/// design — channel-specific intermediate states (weixin's
/// `scanned`, feishu's `slow_down`) collapse into `Pending` with an
/// optional display refresh and/or backoff hint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AuthPollResult {
    /// Keep polling. If `display` is `Some`, the UI MUST replace
    /// the current render with it (this is how weixin goes from
    /// "scan the QR" to "confirm on your phone"). If
    /// `next_interval_secs` is set, subsequent polls MUST use it
    /// instead of the original interval.
    Pending {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<Vec<AuthDisplayItem>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_interval_secs: Option<u64>,
    },
    /// Terminal success. `values` is a partial account-config patch
    /// the UI merges into its form (keys = JSON Schema property
    /// names from the plugin's `schema`). Typical keys:
    ///   feishu  → {app_id, app_secret, domain}
    ///   weixin  → {token, base_url, account_id}
    Confirmed { values: BTreeMap<String, Value> },
    /// Terminal failure. The executor has given up. `reason` is a
    /// channel-authored message the UI MAY show verbatim.
    Failed { reason: String },
}

/// Errors [`AuthFlowExecutor`] implementations surface. Expected
/// protocol outcomes (expired / denied / slow_down) ride on
/// [`AuthPollResult`] instead, so callers can distinguish "retry /
/// back off" from "network or plugin failure".
#[derive(Debug, Error)]
pub enum AuthFlowError {
    #[error("unknown session id: {0}")]
    UnknownSession(String),
    #[error("invalid start args: {0}")]
    InvalidArgs(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// Channel-blind interface for [`ConfigMethod::AutoLogin`]. Stateful
/// by design: `start` hands the caller a `session_id` whose state
/// the executor owns (device_code for OAuth device flows, QR nonce
/// for weixin, ...).
///
/// Implementations must be thread-safe and cheap to clone (wrap
/// internal state in `Arc` where needed).
#[async_trait]
pub trait AuthFlowExecutor: Send + Sync {
    /// Begin the auto-login flow. `form_state` is whatever the user
    /// has typed into the schema form so far (may be empty / {}).
    /// The plugin picks out whichever fields it needs, applies its
    /// own defaults for the rest, and starts its internal state
    /// machine. No `flow_id` — the plugin alone decides whether
    /// that means device flow, QR scan, or something else.
    async fn start(&self, form_state: Value) -> Result<AuthSession, AuthFlowError>;

    /// Advance the flow by one poll tick. Caller must honour any
    /// `next_interval_secs` and stop polling after `expires_in_secs`
    /// elapses since `start`.
    async fn poll(&self, session_id: &str) -> Result<AuthPollResult, AuthFlowError>;
}

#[cfg(test)]
mod tests;
