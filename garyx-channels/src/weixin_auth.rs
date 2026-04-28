//! Pure HTTP primitives for the Weixin QR login flow.
//!
//! Mirrors the shape of `feishu::device_auth`:
//! `begin_qr_login` + `poll_qr_login_status` do one round trip
//! each. Callers own the sleep/deadline/rendering loop.
//!
//! The weixin server gives us `qrcode_img_content` as a fully-
//! qualified URL (e.g. `https://liteapp.weixin.qq.com/q/...`) on
//! the `/get_bot_qrcode` response — that URL is what gets
//! encoded into the QR; scanning it on a phone navigates to
//! weixin's confirmation page.

use std::time::Duration;

use reqwest::Client as HttpClient;
use serde::Deserialize;
use thiserror::Error;

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Deserialize)]
pub struct WeixinQrStart {
    /// Opaque token the caller passes to poll. NOT the bot token
    /// that lands in account config — this is transient session
    /// state the server tracks until confirmation.
    pub qrcode: String,
    /// Fully-qualified URL to embed inside the QR code (e.g.
    /// `https://liteapp.weixin.qq.com/q/<short>?qrcode=<token>&bot_type=3`).
    /// The UI encodes this string verbatim into a QR; scanning it
    /// on a phone opens weixin's confirmation page.
    pub qrcode_img_content: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WeixinQrStatus {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub ilink_bot_id: Option<String>,
    #[serde(default)]
    pub baseurl: Option<String>,
}

#[derive(Debug, Clone)]
pub enum WeixinPollStatus {
    /// Server hasn't seen a scan yet.
    Pending,
    /// User scanned on their phone but hasn't confirmed login in
    /// the WeChat UI yet. Caller keeps polling but UI may update
    /// to "waiting for confirmation on your phone".
    Scanned,
    /// Login confirmed. All fields below are guaranteed non-empty
    /// (a missing `bot_token` or `ilink_bot_id` is mapped to
    /// `Protocol` error so the caller can surface a clear
    /// message instead of silently confirming with empty creds).
    Confirmed(WeixinConfirmed),
}

#[derive(Debug, Clone)]
pub struct WeixinConfirmed {
    /// Bot token — lands in `channels.weixin.accounts.<id>.config.token`.
    pub bot_token: String,
    /// The scanned bot's id. Today this becomes the HashMap KEY
    /// (`account_id`) in `channels.weixin.accounts`, NOT a
    /// field inside the account struct. The CLI's
    /// `perform_weixin_login` surfaces it via `scanned_account_id`;
    /// we preserve the same semantics.
    pub ilink_bot_id: String,
    /// Effective base URL. May equal the one we called `/begin` on
    /// or a cluster override from the server.
    pub base_url: String,
}

#[derive(Debug, Error)]
pub enum WeixinAuthError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server returned malformed response: {0}")]
    Protocol(String),
    #[error("unknown poll status: {0}")]
    UnknownStatus(String),
}

/// Production entrypoint: GET `{base}/ilink/bot/get_bot_qrcode?bot_type=3`.
pub async fn begin_qr_login(
    client: &HttpClient,
    base_url: &str,
) -> Result<WeixinQrStart, WeixinAuthError> {
    let endpoint = format!(
        "{}/ilink/bot/get_bot_qrcode",
        base_url.trim_end_matches('/')
    );
    begin_qr_login_at(client, &endpoint).await
}

#[doc(hidden)]
pub async fn begin_qr_login_at(
    client: &HttpClient,
    endpoint: &str,
) -> Result<WeixinQrStart, WeixinAuthError> {
    let resp = client
        .get(endpoint)
        .query(&[("bot_type", "3")])
        .timeout(HTTP_TIMEOUT)
        .send()
        .await?
        .error_for_status()?;
    let parsed: WeixinQrStart = resp.json().await?;
    if parsed.qrcode.trim().is_empty() || parsed.qrcode_img_content.trim().is_empty() {
        return Err(WeixinAuthError::Protocol(
            "qrcode response missing qrcode or qrcode_img_content".into(),
        ));
    }
    Ok(parsed)
}

/// Production entrypoint: GET
/// `{base}/ilink/bot/get_qrcode_status?qrcode={token}`.
///
/// Weixin's API expects the `iLink-App-ClientVersion: 1` header
/// on every poll; we set it here so the executor doesn't have to.
pub async fn poll_qr_login_status(
    client: &HttpClient,
    base_url: &str,
    qrcode: &str,
) -> Result<WeixinPollStatus, WeixinAuthError> {
    let endpoint = format!(
        "{}/ilink/bot/get_qrcode_status",
        base_url.trim_end_matches('/')
    );
    poll_qr_login_status_at(client, &endpoint, qrcode, base_url).await
}

#[doc(hidden)]
pub async fn poll_qr_login_status_at(
    client: &HttpClient,
    endpoint: &str,
    qrcode: &str,
    fallback_base_url: &str,
) -> Result<WeixinPollStatus, WeixinAuthError> {
    let resp = client
        .get(endpoint)
        .query(&[("qrcode", qrcode)])
        .header("iLink-App-ClientVersion", "1")
        .timeout(HTTP_TIMEOUT)
        .send()
        .await?
        .error_for_status()?;
    let parsed: WeixinQrStatus = resp.json().await.unwrap_or_default();
    // Server returns "wait" for the not-yet-scanned state. Empty
    // status is treated as pending too — the old CLI driver does
    // the same for robustness against warm-up races.
    match parsed.status.as_str() {
        "wait" | "" => Ok(WeixinPollStatus::Pending),
        "scaned" => Ok(WeixinPollStatus::Scanned),
        "confirmed" => {
            let bot_token = parsed
                .bot_token
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    WeixinAuthError::Protocol("confirmed response missing bot_token".into())
                })?
                .to_owned();
            let ilink_bot_id = parsed
                .ilink_bot_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    WeixinAuthError::Protocol("confirmed response missing ilink_bot_id".into())
                })?
                .to_owned();
            let base_url = parsed
                .baseurl
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| fallback_base_url.trim_end_matches('/').to_owned());
            Ok(WeixinPollStatus::Confirmed(WeixinConfirmed {
                bot_token,
                ilink_bot_id,
                base_url,
            }))
        }
        other => Err(WeixinAuthError::UnknownStatus(other.to_owned())),
    }
}

#[cfg(test)]
mod tests;
