//! Local coding-assistant weekly quota API.
//!
//! Reads the locally stored OAuth credentials for Claude Code and Codex on the
//! current machine and reports each tool's weekly (and session) usage windows so
//! mobile/desktop surfaces can show how much of the weekly allowance remains.
//!
//! Source of truth for each tool:
//! - Claude Code: `~/.claude/.credentials.json` if present, otherwise the macOS
//!   keychain item `Claude Code-credentials`; usage from
//!   `GET https://api.anthropic.com/api/oauth/usage`.
//! - Codex: `~/.codex/auth.json` (`tokens.access_token` + `account_id`); usage
//!   from `GET https://chatgpt.com/backend-api/wham/usage`.
//!
//! Only numeric utilization fields and reset timestamps are extracted. Tokens
//! and account identity returned by these endpoints are never logged or
//! persisted.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{Json, http::StatusCode, response::IntoResponse};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_USER_AGENT: &str = "claude-code/2.0.32";
const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";

const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_USER_AGENT: &str = "codex_cli_rs";

const PROVIDER_CLAUDE: &str = "claude_code";
const PROVIDER_CODEX: &str = "codex";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Reuse a cached reading when it is younger than this, to avoid hammering the
/// upstream usage endpoints when several clients poll at once.
const FRESH_TTL: Duration = Duration::from_secs(20);

/// One usage window (e.g. the rolling weekly allowance or the 5-hour session).
#[derive(Debug, Clone, Serialize)]
pub struct UsageWindow {
    /// Percentage of the allowance already consumed (0-100).
    pub used_percent: f64,
    /// Percentage of the allowance still available (0-100).
    pub remaining_percent: f64,
    /// ISO 8601 timestamp when the window resets, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    /// Seconds until the window resets, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_after_seconds: Option<i64>,
}

impl UsageWindow {
    fn from_used_percent(
        used_percent: f64,
        resets_at: Option<String>,
        reset_after_seconds: Option<i64>,
    ) -> Self {
        let used = used_percent.clamp(0.0, 100.0);
        Self {
            used_percent: used,
            remaining_percent: (100.0 - used).clamp(0.0, 100.0),
            resets_at,
            reset_after_seconds,
        }
    }
}

/// Usage for a single coding assistant on this machine.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderUsage {
    /// Stable identifier, e.g. `claude_code` or `codex`.
    pub id: &'static str,
    /// Display name as shown in the Mac app.
    pub name: &'static str,
    /// Whether a fresh reading was available for this provider.
    pub available: bool,
    /// True when the data is a previously cached reading served because the
    /// latest refresh failed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
    /// Plan/subscription label reported by the upstream service, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Rolling weekly allowance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly: Option<UsageWindow>,
    /// Rolling session allowance (5-hour window).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<UsageWindow>,
    /// Human-readable reason a reading is unavailable, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ProviderUsage {
    fn unavailable(id: &'static str, name: &'static str, error: String) -> Self {
        Self {
            id,
            name,
            available: false,
            stale: false,
            plan: None,
            weekly: None,
            session: None,
            error: Some(error),
        }
    }
}

/// Aggregate response for all known coding assistants.
#[derive(Debug, Clone, Serialize)]
pub struct CodingUsageResponse {
    pub providers: Vec<ProviderUsage>,
    /// ISO 8601 timestamp when this response was produced.
    pub refreshed_at: String,
}

/// `GET /api/usage/coding` — weekly quota remaining for local coding assistants.
pub async fn get_coding_usage() -> impl IntoResponse {
    let (claude, codex) = tokio::join!(
        resolve_provider(PROVIDER_CLAUDE, "Claude Code", fetch_claude_usage()),
        resolve_provider(PROVIDER_CODEX, "Codex", fetch_codex_usage()),
    );

    let body = CodingUsageResponse {
        providers: vec![claude, codex],
        refreshed_at: Utc::now().to_rfc3339(),
    };
    (StatusCode::OK, Json(body))
}

/// Wrap a provider fetch with the short-lived cache and stale-on-error policy.
async fn resolve_provider(
    id: &'static str,
    name: &'static str,
    fetch: impl std::future::Future<Output = Result<ProviderUsage, String>>,
) -> ProviderUsage {
    if let Some((age, value)) = cached(id)
        && age < FRESH_TTL
    {
        return value;
    }

    match fetch.await {
        Ok(value) => {
            store(id, value.clone());
            value
        }
        Err(error) => match cached(id) {
            Some((_, mut value)) => {
                value.stale = true;
                value.error = Some(error);
                value
            }
            None => ProviderUsage::unavailable(id, name, error),
        },
    }
}

// ---------------------------------------------------------------------------
// Claude Code
// ---------------------------------------------------------------------------

async fn fetch_claude_usage() -> Result<ProviderUsage, String> {
    let credentials = read_claude_credentials().await?;
    let token = credentials
        .get("claudeAiOauth")
        .and_then(|oauth| oauth.get("accessToken"))
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "Claude credentials missing claudeAiOauth.accessToken".to_string())?;
    let plan = credentials
        .get("claudeAiOauth")
        .and_then(|oauth| oauth.get("subscriptionType"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    let client = http_client()?;
    let response = client
        .get(CLAUDE_USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", CLAUDE_OAUTH_BETA)
        .header(reqwest::header::USER_AGENT, CLAUDE_USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| format!("Claude usage request failed: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("Claude usage response unreadable: {error}"))?;
    if !status.is_success() {
        return Err(format!("Claude usage request returned HTTP {status}"));
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("Claude usage response was not JSON: {error}"))?;
    parse_claude_usage(&value, plan)
}

/// Build a [`ProviderUsage`] from the Anthropic OAuth usage payload.
fn parse_claude_usage(value: &Value, plan: Option<String>) -> Result<ProviderUsage, String> {
    let weekly = value.get("seven_day").and_then(parse_claude_window);
    let session = value.get("five_hour").and_then(parse_claude_window);
    if weekly.is_none() && session.is_none() {
        return Err("Claude usage response had no usable usage windows".to_string());
    }
    Ok(ProviderUsage {
        id: PROVIDER_CLAUDE,
        name: "Claude Code",
        available: true,
        stale: false,
        plan,
        weekly,
        session,
        error: None,
    })
}

/// Parse one Anthropic usage window. Returns `None` when the numeric
/// `utilization` is absent or non-numeric, so a partial/changed payload reports
/// the window as unavailable instead of silently claiming 100% remaining.
fn parse_claude_window(window: &Value) -> Option<UsageWindow> {
    let used = window.get("utilization").and_then(Value::as_f64)?;
    let resets_at = window
        .get("resets_at")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let reset_after_seconds = resets_at.as_deref().and_then(seconds_until);
    Some(UsageWindow::from_used_percent(used, resets_at, reset_after_seconds))
}

/// Read the Claude credential JSON, preferring an on-disk credentials file and
/// falling back to the macOS keychain.
async fn read_claude_credentials() -> Result<Value, String> {
    if let Some(home) = garyx_models::local_paths::home_dir() {
        let path = home.join(".claude").join(".credentials.json");
        if let Ok(contents) = tokio::fs::read_to_string(&path).await {
            return serde_json::from_str(&contents).map_err(|error| {
                format!("Claude credentials file (~/.claude/.credentials.json) was not valid JSON: {error}")
            });
        }
    }

    #[cfg(target_os = "macos")]
    {
        read_claude_keychain().await
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Claude credentials not found (~/.claude/.credentials.json missing)".to_string())
    }
}

#[cfg(target_os = "macos")]
async fn read_claude_keychain() -> Result<Value, String> {
    let output = tokio::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .await
        .map_err(|error| format!("Claude keychain lookup failed to launch: {error}"))?;
    if !output.status.success() {
        return Err("Claude credentials not found in keychain".to_string());
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(raw.trim())
        .map_err(|error| format!("Claude keychain entry was not JSON: {error}"))
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

async fn fetch_codex_usage() -> Result<ProviderUsage, String> {
    let auth = read_codex_chatgpt_auth()?;

    let client = http_client()?;
    let mut request = client
        .get(CODEX_USAGE_URL)
        .bearer_auth(&auth.access_token)
        .header(reqwest::header::USER_AGENT, CODEX_USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(account_id) = auth.account_id.as_deref()
        && !account_id.trim().is_empty()
    {
        request = request.header("ChatGPT-Account-ID", account_id);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("Codex usage request failed: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("Codex usage response unreadable: {error}"))?;
    if !status.is_success() {
        return Err(format!("Codex usage request returned HTTP {status}"));
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("Codex usage response was not JSON: {error}"))?;
    parse_codex_usage(&value)
}

/// Build a [`ProviderUsage`] from the ChatGPT/Codex `wham/usage` payload.
fn parse_codex_usage(value: &Value) -> Result<ProviderUsage, String> {
    let rate_limit = value
        .get("rate_limit")
        .ok_or_else(|| "Codex usage response missing rate_limit".to_string())?;
    // Only treat `secondary_window` as the weekly allowance when its declared
    // window length is actually ~7 days; otherwise we would mislabel a daily or
    // other rolling limit as weekly quota.
    let weekly = rate_limit
        .get("secondary_window")
        .filter(|window| codex_window_is_weekly(window))
        .and_then(parse_codex_window);
    let session = rate_limit.get("primary_window").and_then(parse_codex_window);
    if weekly.is_none() && session.is_none() {
        return Err("Codex usage response had no usable rate-limit windows".to_string());
    }
    let plan = value
        .get("plan_type")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Ok(ProviderUsage {
        id: PROVIDER_CODEX,
        name: "Codex",
        available: true,
        stale: false,
        plan,
        weekly,
        session,
        error: None,
    })
}

const WEEK_SECONDS: i64 = 7 * 24 * 60 * 60;
const WINDOW_TOLERANCE_SECONDS: i64 = 24 * 60 * 60;

/// True when a Codex rate-limit window declares a ~7-day length. Absent or
/// mismatched `limit_window_seconds` is treated as "not weekly" so the API
/// never claims weekly data it cannot confirm.
fn codex_window_is_weekly(window: &Value) -> bool {
    match window.get("limit_window_seconds").and_then(Value::as_i64) {
        Some(seconds) => (seconds - WEEK_SECONDS).abs() <= WINDOW_TOLERANCE_SECONDS,
        None => false,
    }
}

/// Parse one Codex rate-limit window. Returns `None` when the numeric
/// `used_percent` is absent or non-numeric, so a partial/changed payload reports
/// the window as unavailable instead of silently claiming 100% remaining.
fn parse_codex_window(window: &Value) -> Option<UsageWindow> {
    let used = window.get("used_percent").and_then(Value::as_f64)?;
    let reset_after_seconds = window.get("reset_after_seconds").and_then(Value::as_i64);
    let resets_at = window
        .get("reset_at")
        .and_then(Value::as_i64)
        .and_then(|epoch| DateTime::<Utc>::from_timestamp(epoch, 0))
        .map(|dt| dt.to_rfc3339());
    Some(UsageWindow::from_used_percent(used, resets_at, reset_after_seconds))
}

struct CodexChatgptAuth {
    access_token: String,
    account_id: Option<String>,
}

/// Read the ChatGPT (subscription) Codex credentials. Unlike provider auth
/// resolution this intentionally ignores any `OPENAI_API_KEY`, because weekly
/// quota windows only exist for the ChatGPT-plan login.
fn read_codex_chatgpt_auth() -> Result<CodexChatgptAuth, String> {
    let home = codex_home().ok_or_else(|| "Codex home directory is unset".to_string())?;
    let auth_path = home.join("auth.json");
    let contents = std::fs::read_to_string(&auth_path)
        .map_err(|error| format!("Codex auth (~/.codex/auth.json) not readable: {error}"))?;
    let value: Value = serde_json::from_str(&contents)
        .map_err(|error| format!("Codex auth file (~/.codex/auth.json) was not valid JSON: {error}"))?;
    let tokens = value
        .get("tokens")
        .ok_or_else(|| "Codex auth has no ChatGPT login (sign in with ChatGPT)".to_string())?;
    let access_token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "Codex auth missing tokens.access_token".to_string())?
        .to_string();
    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .or_else(|| {
            tokens
                .get("id_token")
                .and_then(|id| id.get("chatgpt_account_id"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned);
    Ok(CodexChatgptAuth {
        access_token,
        account_id,
    })
}

fn codex_home() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var("CODEX_HOME")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        return Some(std::path::PathBuf::from(dir));
    }
    garyx_models::local_paths::home_dir().map(|home| home.join(".codex"))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("failed to build HTTP client: {error}"))
}

/// Seconds from now until an RFC 3339 timestamp, clamped at zero.
fn seconds_until(timestamp: &str) -> Option<i64> {
    let target = DateTime::parse_from_rfc3339(timestamp).ok()?;
    let delta = target.with_timezone(&Utc) - Utc::now();
    Some(delta.num_seconds().max(0))
}

struct CacheEntry {
    fetched_at: Instant,
    value: ProviderUsage,
}

fn cache() -> &'static Mutex<HashMap<&'static str, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached(id: &'static str) -> Option<(Duration, ProviderUsage)> {
    let guard = cache().lock().ok()?;
    let entry = guard.get(id)?;
    Some((entry.fetched_at.elapsed(), entry.value.clone()))
}

fn store(id: &'static str, value: ProviderUsage) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(
            id,
            CacheEntry {
                fetched_at: Instant::now(),
                value,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_claude_weekly_and_session() {
        let payload = json!({
            "five_hour": {"utilization": 11.0, "resets_at": "2030-01-01T05:00:00+00:00"},
            "seven_day": {"utilization": 27.0, "resets_at": "2030-01-07T11:00:00+00:00"},
        });
        let usage = parse_claude_usage(&payload, Some("max".to_string())).unwrap();
        assert!(usage.available);
        assert_eq!(usage.id, PROVIDER_CLAUDE);
        assert_eq!(usage.plan.as_deref(), Some("max"));
        let weekly = usage.weekly.expect("weekly window");
        assert_eq!(weekly.used_percent, 27.0);
        assert_eq!(weekly.remaining_percent, 73.0);
        assert_eq!(weekly.resets_at.as_deref(), Some("2030-01-07T11:00:00+00:00"));
        let session = usage.session.expect("session window");
        assert_eq!(session.remaining_percent, 89.0);
    }

    #[test]
    fn claude_missing_windows_is_error() {
        let payload = json!({ "extra_usage": { "is_enabled": true } });
        assert!(parse_claude_usage(&payload, None).is_err());
    }

    #[test]
    fn parses_codex_weekly_and_session() {
        let payload = json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 2,
                    "limit_window_seconds": 18000,
                    "reset_after_seconds": 16384,
                    "reset_at": 1893477600
                },
                "secondary_window": {
                    "used_percent": 89,
                    "limit_window_seconds": 604800,
                    "reset_after_seconds": 140803,
                    "reset_at": 1893477600
                }
            }
        });
        let usage = parse_codex_usage(&payload).unwrap();
        assert!(usage.available);
        assert_eq!(usage.id, PROVIDER_CODEX);
        assert_eq!(usage.plan.as_deref(), Some("pro"));
        let weekly = usage.weekly.expect("weekly window");
        assert_eq!(weekly.used_percent, 89.0);
        assert_eq!(weekly.remaining_percent, 11.0);
        assert_eq!(weekly.reset_after_seconds, Some(140803));
        assert!(weekly.resets_at.is_some());
        let session = usage.session.expect("session window");
        assert_eq!(session.remaining_percent, 98.0);
    }

    #[test]
    fn codex_missing_rate_limit_is_error() {
        let payload = json!({ "plan_type": "pro" });
        assert!(parse_codex_usage(&payload).is_err());
    }

    #[test]
    fn claude_window_without_utilization_is_unavailable() {
        // seven_day present but no numeric utilization -> weekly unavailable,
        // never silently reported as 100% remaining.
        let payload = json!({
            "seven_day": {"resets_at": "2030-01-07T11:00:00+00:00"},
            "five_hour": {"utilization": 11.0},
        });
        let usage = parse_claude_usage(&payload, None).unwrap();
        assert!(usage.weekly.is_none());
        assert!(usage.session.is_some());

        // No usable windows at all -> error (provider unavailable).
        let empty = json!({ "seven_day": {}, "five_hour": {"utilization": "oops"} });
        assert!(parse_claude_usage(&empty, None).is_err());
    }

    #[test]
    fn codex_window_without_used_percent_is_unavailable() {
        let payload = json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {"used_percent": 2, "limit_window_seconds": 18000},
                "secondary_window": {"limit_window_seconds": 604800, "reset_after_seconds": 100},
            }
        });
        let usage = parse_codex_usage(&payload).unwrap();
        assert!(usage.weekly.is_none(), "weekly without used_percent must be unavailable");
        assert!(usage.session.is_some());
    }

    #[test]
    fn codex_secondary_window_only_weekly_when_length_matches() {
        // A daily-length secondary window must NOT be reported as weekly quota.
        let daily = json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {"used_percent": 2, "limit_window_seconds": 18000},
                "secondary_window": {"used_percent": 50, "limit_window_seconds": 86400},
            }
        });
        let usage = parse_codex_usage(&daily).unwrap();
        assert!(usage.weekly.is_none(), "daily secondary window is not weekly");
        assert!(usage.session.is_some());

        // Missing window length is also treated as not-weekly.
        let no_length = json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {"used_percent": 2, "limit_window_seconds": 18000},
                "secondary_window": {"used_percent": 50},
            }
        });
        let usage = parse_codex_usage(&no_length).unwrap();
        assert!(usage.weekly.is_none());
        assert!(usage.session.is_some());

        // A near-weekly length (within tolerance) still counts as weekly.
        let near_weekly = json!({
            "rate_limit": {
                "secondary_window": {"used_percent": 50, "limit_window_seconds": 600000},
            }
        });
        let usage = parse_codex_usage(&near_weekly).unwrap();
        assert!(usage.weekly.is_some());
    }

    #[test]
    fn usage_window_clamps_out_of_range() {
        let window = UsageWindow::from_used_percent(140.0, None, None);
        assert_eq!(window.used_percent, 100.0);
        assert_eq!(window.remaining_percent, 0.0);
    }

    /// Full-path smoke test against the live endpoints using this machine's
    /// local credentials. Ignored by default; run with
    /// `cargo test -p garyx-gateway --lib coding_usage::tests::live_fetch_smoke -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "hits live Anthropic/ChatGPT endpoints with local credentials"]
    async fn live_fetch_smoke() {
        match fetch_claude_usage().await {
            Ok(usage) => {
                assert!(usage.available);
                println!(
                    "claude: plan={:?} weekly_remaining={:?} session_remaining={:?}",
                    usage.plan,
                    usage.weekly.as_ref().map(|w| w.remaining_percent),
                    usage.session.as_ref().map(|w| w.remaining_percent),
                );
            }
            Err(error) => println!("claude unavailable: {error}"),
        }
        match fetch_codex_usage().await {
            Ok(usage) => {
                assert!(usage.available);
                println!(
                    "codex: plan={:?} weekly_remaining={:?} session_remaining={:?}",
                    usage.plan,
                    usage.weekly.as_ref().map(|w| w.remaining_percent),
                    usage.session.as_ref().map(|w| w.remaining_percent),
                );
            }
            Err(error) => println!("codex unavailable: {error}"),
        }
    }
}
