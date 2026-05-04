//! Feishu / Lark OAuth 2.0 Device Authorization Grant (RFC 8628).
//!
//! This is the programmatic on-ramp for "one-click create a PersonalAgent
//! bot" — the same flow the official `lark-cli config init` uses. The user
//! does not need to log into the open platform web console, create an app,
//! enable scopes, and paste App ID / App Secret back into our config. They
//! scan a QR (or click a link), approve on the phone, and we receive the
//! `client_id` + `client_secret` directly.
//!
//! ## Flow
//!
//! 1. `begin_app_registration` POSTs `action=begin` to the accounts
//!    server. The server returns a `device_code` (secret, used for polling),
//!    a `user_code` (short human-readable), and an `expires_in` / `interval`
//!    pair. We then construct a `verification_url` under the `open.*` host
//!    that the user can scan/click — the open platform wraps the PersonalAgent
//!    creation flow around this `user_code`.
//!
//! 2. The caller renders the QR + URL to the human somehow.
//!
//! 3. `poll_once` POSTs `action=poll&device_code=...` repeatedly until one of
//!    the terminal states fires:
//!       - `Pending` — keep waiting, caller sleeps `interval` seconds.
//!       - `SlowDown` — rate-limited, caller bumps interval by 5s (cap 60s).
//!       - `Denied` — user declined.
//!       - `Expired` — `device_code` TTL elapsed before confirmation.
//!       - `Success` — we have `app_id`, `app_secret`, `tenant_brand`.
//!
//! 4. If the success response says `tenant_brand="lark"` but the
//!    `client_secret` is empty (can happen when the user picked feishu at
//!    begin time but their tenant is actually hosted on larksuite), the
//!    caller should retry `poll_once` against the Lark endpoint using the
//!    same `device_code`.
//!
//! `run_device_flow` bundles all of the above — callers usually want this.

use std::time::Duration;

use garyx_models::config::FeishuDomain;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use thiserror::Error;
use tokio::time::Instant;
use tracing::warn;

/// The `archetype` value we request — always "PersonalAgent" (matches the
/// lark-cli contract). This triggers the "create a PersonalAgent bot" flow
/// on the open platform rather than any of the enterprise-admin flows.
const ARCHETYPE: &str = "PersonalAgent";

/// How slow can the server ask us to go before we give up pretending
/// and cap the backoff. Matches lark-cli's constant.
const MAX_POLL_INTERVAL_SECS: u64 = 60;

/// Safety cap on total poll attempts regardless of the server's declared
/// TTL — protects against a server bug that keeps returning
/// `authorization_pending` forever.
const MAX_POLL_ATTEMPTS: u32 = 200;

/// Timeout for a single HTTP round-trip to the accounts server.
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Returned by `begin_app_registration`. Render the `verification_url`
/// to the user and keep `device_code` around for polling.
#[derive(Debug, Clone)]
pub struct DeviceFlowBegin {
    /// Opaque secret used in poll requests. Never shown to the user.
    pub device_code: String,
    /// Short human-readable code embedded in `verification_url`. Printing
    /// it next to the QR helps confused users verify they're on the right
    /// pairing session.
    pub user_code: String,
    /// Full URL to open in a browser / scan as QR. Already includes
    /// `user_code`, `from=cli`, and version-tracking params.
    pub verification_url: String,
    /// How long (seconds) the `device_code` stays valid. Default 300.
    pub expires_in: u64,
    /// Minimum polling interval (seconds) requested by the server. Default 5.
    pub interval: u64,
}

/// Terminal + non-terminal states from a single poll.
#[derive(Debug, Clone)]
pub enum PollStatus {
    /// User hasn't finished. Caller should sleep `interval` seconds and poll again.
    Pending,
    /// Server wants us to slow down. Caller should bump interval by 5s
    /// (capped at [`MAX_POLL_INTERVAL_SECS`]) and poll again.
    SlowDown,
    /// User tapped "deny" on the confirmation screen. Flow is dead.
    Denied,
    /// `device_code` TTL elapsed before the user confirmed. Flow is dead.
    Expired,
    /// User confirmed; we have credentials.
    Success(DeviceFlowResult),
}

/// Successful outcome of the flow.
#[derive(Debug, Clone)]
pub struct DeviceFlowResult {
    /// Same as `app_id` in our `FeishuAccount` config.
    pub app_id: String,
    /// Same as `app_secret`. Empty when `tenant_brand=lark` and the
    /// caller polled the wrong endpoint — see `run_device_flow` for the
    /// cross-endpoint retry.
    pub app_secret: String,
    /// Which brand the tenant actually belongs to. May not match the
    /// brand the caller passed to `begin_app_registration` (e.g. user
    /// picked "Feishu" but their tenant is a Lark one).
    pub tenant_brand: FeishuDomain,
}

/// Errors that terminate the flow. Protocol-level outcomes (pending, slow
/// down, denied, expired) ride on [`PollStatus`] instead so callers can
/// differentiate "retry with backoff" from "stop and surface to user".
#[derive(Debug, Error)]
pub enum DeviceFlowError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server returned non-JSON: HTTP {status} body={body}")]
    NonJson { status: u16, body: String },
    #[error("server rejected request: {0}")]
    Rejected(String),
    #[error("device flow timed out waiting for user confirmation")]
    TimedOut,
    #[error("device flow exceeded maximum poll attempts ({0})")]
    TooManyPolls(u32),
    #[error("device flow cancelled by caller")]
    Cancelled,
    #[error("success payload missing client_id")]
    MissingClientId,
}

/// Build the full URL we show/QR-encode for the user.
///
/// Note: the `accounts_url` from the server response is *not* the URL the
/// user should visit — it's the account server endpoint, which doesn't
/// know how to render the PersonalAgent creation UI. The user-visible URL
/// lives under the `open.*` host.
pub fn build_verification_url(domain: &FeishuDomain, user_code: &str, cli_version: &str) -> String {
    let open_host = match domain {
        FeishuDomain::Feishu => "https://open.feishu.cn",
        FeishuDomain::Lark => "https://open.larksuite.com",
    };
    format!(
        "{open_host}/page/cli?user_code={user}&lpv={ver}&ocv={ver}&from=cli",
        open_host = open_host,
        user = urlencoding::encode(user_code),
        ver = urlencoding::encode(cli_version),
    )
}

fn accounts_endpoint(domain: &FeishuDomain) -> &'static str {
    match domain {
        FeishuDomain::Feishu => "https://accounts.feishu.cn/oauth/v1/app/registration",
        FeishuDomain::Lark => "https://accounts.larksuite.com/oauth/v1/app/registration",
    }
}

/// Begin the device flow. Always targets the Feishu accounts server —
/// the brand only matters when constructing the user-facing URL (and
/// later when polling after we detect the tenant brand from the response).
///
/// This mirrors `larkauth.RequestAppRegistration` in `lark-cli`:
/// `action=begin` is always issued against `accounts.feishu.cn`, even
/// for users whose tenant is on Lark.
pub async fn begin_app_registration(
    client: &HttpClient,
    user_facing_domain: &FeishuDomain,
    cli_version: &str,
) -> Result<DeviceFlowBegin, DeviceFlowError> {
    begin_app_registration_at(
        client,
        accounts_endpoint(&FeishuDomain::Feishu),
        user_facing_domain,
        cli_version,
    )
    .await
}

/// Same as [`begin_app_registration`] but targets `endpoint`
/// instead of the hardcoded `accounts.feishu.cn` URL. Purely a
/// test seam — production code uses the zero-arg wrapper above.
/// `#[doc(hidden)]` so it doesn't appear in rustdoc; `pub` so
/// integration tests under `tests/` can reach it.
#[doc(hidden)]
pub async fn begin_app_registration_at(
    client: &HttpClient,
    endpoint: &str,
    user_facing_domain: &FeishuDomain,
    cli_version: &str,
) -> Result<DeviceFlowBegin, DeviceFlowError> {
    let resp = client
        .post(endpoint)
        .timeout(HTTP_TIMEOUT)
        .form(&[
            ("action", "begin"),
            ("archetype", ARCHETYPE),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id tenant_brand"),
        ])
        .send()
        .await?;

    let status = resp.status();
    let body = resp.text().await?;
    let data: BeginOrPollResponse = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return Err(DeviceFlowError::NonJson {
                status: status.as_u16(),
                body,
            });
        }
    };

    if !status.is_success() || data.error.is_some() {
        let msg = data
            .error_description
            .or(data.error)
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        return Err(DeviceFlowError::Rejected(msg));
    }

    let device_code = data.device_code.unwrap_or_default();
    let user_code = data.user_code.unwrap_or_default();
    if device_code.is_empty() || user_code.is_empty() {
        return Err(DeviceFlowError::Rejected(
            "begin response missing device_code or user_code".to_owned(),
        ));
    }

    Ok(DeviceFlowBegin {
        verification_url: build_verification_url(user_facing_domain, &user_code, cli_version),
        device_code,
        user_code,
        expires_in: data.expires_in.unwrap_or(300),
        interval: data.interval.unwrap_or(5),
    })
}

/// Execute one poll request. Pure function of (endpoint, device_code);
/// the caller owns the sleep/backoff loop.
pub async fn poll_once(
    client: &HttpClient,
    poll_domain: &FeishuDomain,
    device_code: &str,
) -> Result<PollStatus, DeviceFlowError> {
    poll_once_at(
        client,
        accounts_endpoint(poll_domain),
        poll_domain,
        device_code,
    )
    .await
}

/// Same as [`poll_once`] but posts to `endpoint` instead of the
/// hardcoded `accounts.feishu.cn` / `accounts.larksuite.com`
/// addresses. Test seam — marked `#[doc(hidden)]` so it doesn't
/// clutter rustdoc.
#[doc(hidden)]
pub async fn poll_once_at(
    client: &HttpClient,
    endpoint: &str,
    poll_domain: &FeishuDomain,
    device_code: &str,
) -> Result<PollStatus, DeviceFlowError> {
    let resp = client
        .post(endpoint)
        .timeout(HTTP_TIMEOUT)
        .form(&[("action", "poll"), ("device_code", device_code)])
        .send()
        .await?;

    let status = resp.status();
    let body = resp.text().await?;
    let data: BeginOrPollResponse = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return Err(DeviceFlowError::NonJson {
                status: status.as_u16(),
                body,
            });
        }
    };

    // Success path: client_id is populated.
    if data.error.is_none()
        && let Some(client_id) = data.client_id.filter(|s| !s.is_empty())
    {
        let tenant_brand = data
            .user_info
            .as_ref()
            .and_then(|u| u.tenant_brand.as_deref())
            .map(|b| {
                if b == "lark" {
                    FeishuDomain::Lark
                } else {
                    FeishuDomain::Feishu
                }
            })
            .unwrap_or_else(|| poll_domain.clone());
        return Ok(PollStatus::Success(DeviceFlowResult {
            app_id: client_id,
            app_secret: data.client_secret.unwrap_or_default(),
            tenant_brand,
        }));
    }

    match data.error.as_deref() {
        Some("authorization_pending") => Ok(PollStatus::Pending),
        Some("slow_down") => Ok(PollStatus::SlowDown),
        Some("access_denied") => Ok(PollStatus::Denied),
        Some("expired_token") | Some("invalid_grant") => Ok(PollStatus::Expired),
        Some(other) => {
            let msg = data.error_description.unwrap_or_else(|| other.to_owned());
            Err(DeviceFlowError::Rejected(msg))
        }
        None => {
            // No error + no client_id. Treat as pending (defensive — some
            // servers have been known to return empty bodies during warm-up).
            Ok(PollStatus::Pending)
        }
    }
}

/// High-level driver: begin + poll until terminal, returning credentials.
///
/// `on_begin` fires exactly once, as soon as we have the URL to show the
/// user. This is where the caller renders the QR / opens a browser / updates
/// UI state. Returning from `on_begin` does not delay polling.
///
/// Cancellation: `cancel` is consulted before each sleep; callers wire it
/// to Ctrl-C, dialog-close, etc. Returning `true` from `cancel` short-circuits
/// the flow with [`DeviceFlowError::Cancelled`].
pub async fn run_device_flow<F, C>(
    client: &HttpClient,
    user_facing_domain: &FeishuDomain,
    cli_version: &str,
    on_begin: F,
    mut cancel: C,
) -> Result<DeviceFlowResult, DeviceFlowError>
where
    F: FnOnce(&DeviceFlowBegin),
    C: FnMut() -> bool,
{
    let begin = begin_app_registration(client, user_facing_domain, cli_version).await?;
    on_begin(&begin);

    let mut interval = begin.interval.max(1);
    let deadline = Instant::now() + Duration::from_secs(begin.expires_in);
    let mut attempts: u32 = 0;
    let mut poll_domain: FeishuDomain = user_facing_domain.clone();

    loop {
        if cancel() {
            return Err(DeviceFlowError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(DeviceFlowError::TimedOut);
        }
        if attempts >= MAX_POLL_ATTEMPTS {
            return Err(DeviceFlowError::TooManyPolls(attempts));
        }
        attempts += 1;

        tokio::time::sleep(Duration::from_secs(interval)).await;

        if cancel() {
            return Err(DeviceFlowError::Cancelled);
        }

        match poll_once(client, &poll_domain, &begin.device_code).await {
            Ok(PollStatus::Pending) => continue,
            Ok(PollStatus::SlowDown) => {
                interval = (interval + 5).min(MAX_POLL_INTERVAL_SECS);
                continue;
            }
            Ok(PollStatus::Denied) => {
                return Err(DeviceFlowError::Rejected(
                    "user denied the authorization".to_owned(),
                ));
            }
            Ok(PollStatus::Expired) => return Err(DeviceFlowError::TimedOut),
            Ok(PollStatus::Success(mut result)) => {
                // Lark-tenant-on-feishu-endpoint quirk: the open platform
                // routes PersonalAgent creation through accounts.feishu.cn
                // for every brand, but if the tenant is a Lark one the
                // client_secret only shows up on accounts.larksuite.com.
                // Detect, swap endpoint, re-poll once. After the swap the
                // server may legitimately return pending for a few seconds
                // so bounded-loop until TTL.
                if result.app_secret.is_empty()
                    && matches!(result.tenant_brand, FeishuDomain::Lark)
                    && !matches!(poll_domain, FeishuDomain::Lark)
                {
                    warn!("detected lark tenant, retrying poll against larksuite endpoint");
                    poll_domain = FeishuDomain::Lark;
                    continue;
                }
                if result.app_id.is_empty() {
                    return Err(DeviceFlowError::MissingClientId);
                }
                // Propagate whatever brand the tenant actually is, even
                // if app_secret is still empty (caller will surface a
                // helpful error in that case).
                result.tenant_brand = match result.tenant_brand {
                    FeishuDomain::Lark => FeishuDomain::Lark,
                    FeishuDomain::Feishu => FeishuDomain::Feishu,
                };
                return Ok(result);
            }
            Err(e) => return Err(e),
        }
    }
}

/// Unified shape for begin + poll responses. The server reuses the same
/// endpoint for both actions; fields not relevant to one action show up
/// as `None` after deserialization.
#[derive(Debug, Deserialize)]
struct BeginOrPollResponse {
    // Begin-only
    #[serde(default)]
    device_code: Option<String>,
    #[serde(default)]
    user_code: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
    // Poll-only
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    user_info: Option<UserInfo>,
    // Error path
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    #[serde(default)]
    tenant_brand: Option<String>,
}

#[cfg(test)]
mod tests;
