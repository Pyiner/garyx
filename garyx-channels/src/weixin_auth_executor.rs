//! [`AuthFlowExecutor`] impl for the Weixin QR login flow.
//!
//! Wraps the pure [`crate::weixin_auth::begin_qr_login`] +
//! [`crate::weixin_auth::poll_qr_login_status`] primitives.
//! The protocol-level display is a text hint plus a QR carrying the
//! opaque session token the server returns; the `scanned`
//! intermediate state collapses into a `Pending` carrying a new
//! display that replaces the original.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use reqwest::Client as HttpClient;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth_flow::{
    AuthDisplayItem, AuthFlowError, AuthFlowExecutor, AuthPollResult, AuthSession,
};
use crate::weixin_auth::{
    WeixinAuthError, WeixinPollStatus, begin_qr_login_at, poll_qr_login_status_at,
};

/// Weixin's own default bot-gateway. Matches the plugin schema's
/// default (see `builtin_catalog::weixin_account_schema`) and the
/// CLI's `normalize_weixin_base_url` fallback, so picking auto-login
/// on a pristine form produces the same endpoint as hitting submit
/// with defaults. `api.weixin.qq.com` (the previous value) doesn't
/// host the `/ilink/bot/...` paths and replies errcode 40066
/// "invalid url hints", which surfaces to the desktop as
/// `start_failed` with "error decoding response body".
const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";

/// Fallback session TTL when the server doesn't tell us one.
/// Mirrors the CLI's historical `480` seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 480;

struct Session {
    qrcode: String,
    base_url: String,
    poll_endpoint: String,
}

#[derive(Clone)]
pub struct WeixinAuthExecutor {
    http: HttpClient,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    endpoint_override: Option<EndpointOverride>,
}

#[derive(Clone)]
struct EndpointOverride {
    begin: String,
    poll: String,
}

impl WeixinAuthExecutor {
    pub fn new(http: HttpClient) -> Self {
        Self {
            http,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            endpoint_override: None,
        }
    }

    #[doc(hidden)]
    pub fn with_endpoint_override(
        http: HttpClient,
        begin_url: impl Into<String>,
        poll_url: impl Into<String>,
    ) -> Self {
        Self {
            http,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            endpoint_override: Some(EndpointOverride {
                begin: begin_url.into(),
                poll: poll_url.into(),
            }),
        }
    }
}

impl Default for WeixinAuthExecutor {
    fn default() -> Self {
        Self::new(HttpClient::new())
    }
}

#[async_trait]
impl AuthFlowExecutor for WeixinAuthExecutor {
    /// `form_state` is read with plugin-owned defaults:
    ///   - `base_url`     : defaults to [`DEFAULT_BASE_URL`].
    ///   - `timeout_secs` : defaults to [`DEFAULT_TIMEOUT_SECS`].
    async fn start(&self, form_state: Value) -> Result<AuthSession, AuthFlowError> {
        let base = form_state
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_BASE_URL)
            .trim_end_matches('/')
            .to_owned();
        let timeout_secs = form_state
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let (begin_endpoint, poll_endpoint) = match &self.endpoint_override {
            Some(o) => (o.begin.clone(), o.poll.clone()),
            None => (
                format!("{base}/ilink/bot/get_bot_qrcode"),
                format!("{base}/ilink/bot/get_qrcode_status"),
            ),
        };

        let start = begin_qr_login_at(&self.http, &begin_endpoint)
            .await
            .map_err(map_err)?;

        let session_id = Uuid::new_v4().to_string();
        self.sessions
            .lock()
            .expect("weixin auth sessions mutex poisoned")
            .insert(
                session_id.clone(),
                Session {
                    qrcode: start.qrcode.clone(),
                    base_url: base,
                    poll_endpoint,
                },
            );

        // Weixin's "display" is a hint + a QR encoding the
        // server-provided `qrcode_img_content` URL (e.g.
        // `https://liteapp.weixin.qq.com/q/...?qrcode=<token>&bot_type=3`).
        // Scanning it on a phone navigates to weixin's confirmation
        // page; we keep the opaque `qrcode` token in `Session` for
        // the `poll()` round trip, since that's what the upstream
        // `/get_qrcode_status?qrcode=` parameter expects.
        let display = vec![
            AuthDisplayItem::text("请用微信扫码完成授权"),
            AuthDisplayItem::qr(start.qrcode_img_content.clone()),
        ];

        Ok(AuthSession {
            session_id,
            display,
            expires_in_secs: timeout_secs,
            // Weixin doesn't tell us an interval — 1s matches the
            // old CLI driver's fixed cadence.
            poll_interval_secs: 1,
        })
    }

    async fn poll(&self, session_id: &str) -> Result<AuthPollResult, AuthFlowError> {
        let (qrcode, poll_endpoint, base_url) = {
            let guard = self
                .sessions
                .lock()
                .expect("weixin auth sessions mutex poisoned");
            let session = guard
                .get(session_id)
                .ok_or_else(|| AuthFlowError::UnknownSession(session_id.to_owned()))?;
            (
                session.qrcode.clone(),
                session.poll_endpoint.clone(),
                session.base_url.clone(),
            )
        };

        match poll_qr_login_status_at(&self.http, &poll_endpoint, &qrcode, &base_url)
            .await
            .map_err(map_err)?
        {
            WeixinPollStatus::Pending => Ok(AuthPollResult::Pending {
                display: None,
                next_interval_secs: None,
            }),
            WeixinPollStatus::Scanned => Ok(AuthPollResult::Pending {
                // Replace the QR with a status-update text so the
                // user stops scanning and knows to confirm on
                // their phone. The UI treats this as "re-render
                // from scratch with the new vec".
                display: Some(vec![AuthDisplayItem::text("已扫码，请在微信内确认登录")]),
                next_interval_secs: None,
            }),
            WeixinPollStatus::Confirmed(confirmed) => {
                self.sessions
                    .lock()
                    .expect("weixin auth sessions mutex poisoned")
                    .remove(session_id);
                let mut values: BTreeMap<String, Value> = BTreeMap::new();
                values.insert("token".into(), Value::String(confirmed.bot_token));
                values.insert("base_url".into(), Value::String(confirmed.base_url));
                // `ilink_bot_id` is the CLI's HashMap key (not a
                // field in the account struct). Expose it under
                // `account_id` so the caller can pick it up
                // without reinventing the key-from-scan convention.
                values.insert("account_id".into(), Value::String(confirmed.ilink_bot_id));
                Ok(AuthPollResult::Confirmed { values })
            }
        }
    }
}

fn map_err(err: WeixinAuthError) -> AuthFlowError {
    match err {
        WeixinAuthError::Http(e) => AuthFlowError::Transport(e.to_string()),
        WeixinAuthError::Protocol(msg) => AuthFlowError::Protocol(msg),
        WeixinAuthError::UnknownStatus(s) => {
            AuthFlowError::Protocol(format!("weixin returned unknown status `{s}`"))
        }
    }
}

#[allow(dead_code)]
fn _unused_serde_marker() -> Value {
    json!(null)
}
