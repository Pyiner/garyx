//! [`AuthFlowExecutor`] impl for the Feishu / Lark channel.
//!
//! Wraps `device_auth::begin_app_registration` + `poll_once` so the
//! CLI / Mac App can drive the flow through the channel-blind trait
//! instead of calling `run_device_flow` directly.
//!
//! The tricky state the old driver managed (lark-tenant-on-feishu-
//! endpoint retry, SlowDown interval bumping, tenant-brand
//! detection from the poll response) is preserved verbatim — this
//! file MUST stay behaviourally identical to `run_device_flow` for
//! the differential test to pass.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use reqwest::Client as HttpClient;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth_flow::{
    AuthDisplayItem, AuthFlowError, AuthFlowExecutor, AuthPollResult, AuthSession,
};
use garyx_models::config::FeishuDomain;

use super::device_auth::{
    DeviceFlowBegin, DeviceFlowError, PollStatus, begin_app_registration_at, poll_once_at,
};

/// Max interval cap, mirrors `device_auth::MAX_POLL_INTERVAL_SECS`.
const MAX_POLL_INTERVAL_SECS: u64 = 30;

/// Default tenant domain when the user-supplied `form_state` doesn't
/// pin one. Matches the schema default so picking "auto login"
/// before touching the form gives the same result as hitting
/// "submit form" with the defaults in place.
const DEFAULT_DOMAIN: &str = "feishu";

/// Internal per-session state the executor owns.
struct Session {
    device_code: String,
    /// Tenant-brand-driven endpoint override. Starts at the
    /// user-facing domain, flips to `Lark` when the poll response
    /// indicates a lark tenant but the secret came back empty
    /// (same retry the old driver did at `device_auth.rs:353`).
    poll_domain: FeishuDomain,
    interval: u64,
    /// Whether we've already flipped to Lark once. Prevents an
    /// infinite loop if the server keeps returning empty secrets.
    lark_retry_used: bool,
}

/// Feishu / Lark auth flow executor. Clone-cheap — the HTTP client
/// and session map live behind `Arc`s.
#[derive(Clone)]
pub struct FeishuAuthExecutor {
    http: HttpClient,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    /// Test seam: when `Some(url)`, all begin / poll requests target
    /// that URL instead of the hardcoded production endpoints.
    endpoint_override: Option<String>,
}

impl FeishuAuthExecutor {
    pub fn new(http: HttpClient) -> Self {
        Self {
            http,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            endpoint_override: None,
        }
    }

    /// Test-only constructor pointing begin + poll at `endpoint`.
    /// Used by `tests/feishu_auth_flow_diff.rs`.
    #[doc(hidden)]
    pub fn with_endpoint_override(http: HttpClient, endpoint: impl Into<String>) -> Self {
        Self {
            http,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            endpoint_override: Some(endpoint.into()),
        }
    }
}

impl Default for FeishuAuthExecutor {
    fn default() -> Self {
        Self::new(HttpClient::new())
    }
}

fn endpoint_for(domain: &FeishuDomain, override_url: &Option<String>) -> String {
    if let Some(url) = override_url.as_ref() {
        return url.clone();
    }
    match domain {
        FeishuDomain::Feishu => "https://accounts.feishu.cn/oauth/v1/app/registration".to_owned(),
        FeishuDomain::Lark => "https://accounts.larksuite.com/oauth/v1/app/registration".to_owned(),
    }
}

fn parse_domain(raw: &str) -> Result<FeishuDomain, AuthFlowError> {
    match raw.to_ascii_lowercase().as_str() {
        "feishu" => Ok(FeishuDomain::Feishu),
        "lark" => Ok(FeishuDomain::Lark),
        other => Err(AuthFlowError::InvalidArgs(format!(
            "unknown feishu domain `{other}` (expected feishu|lark)"
        ))),
    }
}

#[async_trait]
impl AuthFlowExecutor for FeishuAuthExecutor {
    /// `form_state` is read with plugin-owned defaults — the UI
    /// does not need to pass anything. Recognised keys:
    ///   - `domain`       : "feishu" (default) | "lark"
    ///   - `cli_version`  : analytics tag, defaults to the crate ver
    async fn start(&self, form_state: Value) -> Result<AuthSession, AuthFlowError> {
        let domain_raw = form_state
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_DOMAIN);
        let domain = parse_domain(domain_raw)?;
        let cli_version = form_state
            .get("cli_version")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());

        // Begin always hits the Feishu endpoint (even for Lark
        // tenants — mirrors `larkauth.RequestAppRegistration`).
        // Tests flip it via `endpoint_override`.
        let begin_endpoint = endpoint_for(&FeishuDomain::Feishu, &self.endpoint_override);
        let begin: DeviceFlowBegin =
            begin_app_registration_at(&self.http, &begin_endpoint, &domain, &cli_version)
                .await
                .map_err(map_device_error)?;

        let session_id = Uuid::new_v4().to_string();
        let session = Session {
            device_code: begin.device_code.clone(),
            poll_domain: domain.clone(),
            interval: begin.interval.max(1),
            lark_retry_used: false,
        };
        self.sessions
            .lock()
            .expect("feishu auth sessions mutex poisoned")
            .insert(session_id.clone(), session);

        // Plugin decides the layout: two text lines telling the
        // user what to do, a user_code to verify, and a QR encoding
        // the same URL for phone pickup. The UI walks this vec
        // in order.
        let display = vec![
            AuthDisplayItem::text("请在浏览器中打开以下链接完成授权："),
            AuthDisplayItem::text(begin.verification_url.clone()),
            AuthDisplayItem::text(format!("授权码：{}", begin.user_code)),
            AuthDisplayItem::qr(begin.verification_url.clone()),
        ];

        Ok(AuthSession {
            session_id,
            display,
            expires_in_secs: begin.expires_in,
            poll_interval_secs: begin.interval.max(1),
        })
    }

    async fn poll(&self, session_id: &str) -> Result<AuthPollResult, AuthFlowError> {
        // Snapshot the bits we need for one round trip without
        // holding the mutex across `.await`.
        let (device_code, poll_domain, interval, lark_retry_used) = {
            let guard = self
                .sessions
                .lock()
                .expect("feishu auth sessions mutex poisoned");
            let session = guard
                .get(session_id)
                .ok_or_else(|| AuthFlowError::UnknownSession(session_id.to_owned()))?;
            (
                session.device_code.clone(),
                session.poll_domain.clone(),
                session.interval,
                session.lark_retry_used,
            )
        };

        let poll_endpoint = endpoint_for(&poll_domain, &self.endpoint_override);
        let status = poll_once_at(&self.http, &poll_endpoint, &poll_domain, &device_code)
            .await
            .map_err(map_device_error)?;

        match status {
            PollStatus::Pending => Ok(AuthPollResult::Pending {
                display: None,
                next_interval_secs: None,
            }),
            PollStatus::SlowDown => {
                // Same ramp as `run_device_flow`: +5s, capped at
                // 30. Collapses to the protocol's bare Pending
                // variant with a caller-honoured backoff hint.
                let next = (interval + 5).min(MAX_POLL_INTERVAL_SECS);
                self.sessions
                    .lock()
                    .expect("feishu auth sessions mutex poisoned")
                    .get_mut(session_id)
                    .ok_or_else(|| AuthFlowError::UnknownSession(session_id.to_owned()))?
                    .interval = next;
                Ok(AuthPollResult::Pending {
                    display: None,
                    next_interval_secs: Some(next),
                })
            }
            PollStatus::Denied => {
                self.sessions
                    .lock()
                    .expect("feishu auth sessions mutex poisoned")
                    .remove(session_id);
                Ok(AuthPollResult::Failed {
                    reason: "user denied the authorization".into(),
                })
            }
            PollStatus::Expired => {
                self.sessions
                    .lock()
                    .expect("feishu auth sessions mutex poisoned")
                    .remove(session_id);
                Ok(AuthPollResult::Failed {
                    reason: "feishu device code expired before confirmation".into(),
                })
            }
            PollStatus::Success(result) => {
                // Lark-tenant-on-feishu-endpoint quirk: empty
                // secret + lark tenant + not-yet-on-lark-endpoint
                // → flip domain once, return Pending so the caller
                // polls again. `lark_retry_used` guards against an
                // infinite loop.
                if result.app_secret.is_empty()
                    && matches!(result.tenant_brand, FeishuDomain::Lark)
                    && !matches!(poll_domain, FeishuDomain::Lark)
                    && !lark_retry_used
                {
                    let mut guard = self
                        .sessions
                        .lock()
                        .expect("feishu auth sessions mutex poisoned");
                    let session = guard
                        .get_mut(session_id)
                        .ok_or_else(|| AuthFlowError::UnknownSession(session_id.to_owned()))?;
                    session.poll_domain = FeishuDomain::Lark;
                    session.lark_retry_used = true;
                    return Ok(AuthPollResult::Pending {
                        display: None,
                        next_interval_secs: None,
                    });
                }
                if result.app_id.is_empty() {
                    return Err(AuthFlowError::Protocol(
                        "feishu server returned empty app_id".into(),
                    ));
                }
                let domain_str = match result.tenant_brand {
                    FeishuDomain::Feishu => "feishu",
                    FeishuDomain::Lark => "lark",
                };
                let mut values: BTreeMap<String, Value> = BTreeMap::new();
                values.insert("app_id".into(), Value::String(result.app_id));
                values.insert("app_secret".into(), Value::String(result.app_secret));
                values.insert("domain".into(), Value::String(domain_str.into()));
                self.sessions
                    .lock()
                    .expect("feishu auth sessions mutex poisoned")
                    .remove(session_id);
                Ok(AuthPollResult::Confirmed { values })
            }
        }
    }
}

fn map_device_error(err: DeviceFlowError) -> AuthFlowError {
    match err {
        DeviceFlowError::Http(e) => AuthFlowError::Transport(e.to_string()),
        DeviceFlowError::NonJson { status, body } => AuthFlowError::Protocol(format!(
            "feishu accounts server returned HTTP {status} non-JSON body: {body}"
        )),
        DeviceFlowError::Rejected(m) => AuthFlowError::Protocol(m),
        DeviceFlowError::TimedOut => AuthFlowError::Protocol("device flow timed out".into()),
        DeviceFlowError::TooManyPolls(n) => {
            AuthFlowError::Protocol(format!("too many poll attempts ({n})"))
        }
        DeviceFlowError::Cancelled => AuthFlowError::Protocol("cancelled".into()),
        DeviceFlowError::MissingClientId => {
            AuthFlowError::Protocol("feishu server success response had no client_id".into())
        }
    }
}

#[allow(dead_code)]
fn _unused_serde_marker() -> Value {
    json!(null)
}
