//! Local coding-assistant weekly quota API.
//!
//! Reads the locally stored OAuth credentials for Claude Code, Codex, and
//! Antigravity on the current machine and reports each tool's weekly/session
//! usage windows or per-model quota buckets so mobile/desktop surfaces can show
//! how much of the allowance remains.
//!
//! Source of truth for each tool:
//! - Claude Code: macOS keychain item `Claude Code-credentials`, then
//!   `~/.claude/.credentials.json`; usage from
//!   `GET https://api.anthropic.com/api/oauth/usage`.
//! - Codex: `~/.codex/auth.json` (`tokens.access_token` + `account_id`); usage
//!   from `GET https://chatgpt.com/backend-api/wham/usage`.
//! - Antigravity: macOS keychain item `gemini` / account `antigravity`; project
//!   discovery from `loadCodeAssist`, usage from
//!   `POST https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary`.
//!
//! Only numeric utilization fields and reset timestamps are extracted. Tokens,
//! account identity, and project ids are never logged or persisted.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use garyx_models::{
    ProviderType,
    config::{AgentProviderConfig, GaryxConfig},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::timeout;

use crate::claude_oauth;
use crate::server::AppState;

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_USER_AGENT: &str = "codex_cli_rs";

const ANTIGRAVITY_LOAD_CODE_ASSIST_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const ANTIGRAVITY_USAGE_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";
const ANTIGRAVITY_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const ANTIGRAVITY_USER_AGENT: &str = "antigravity/cli/1.0.10 darwin/arm64";
const ANTIGRAVITY_OAUTH_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const ANTIGRAVITY_KEYCHAIN_PREFIX: &str = "go-keyring-base64:";

const PROVIDER_CLAUDE: &str = "claude_code";
const PROVIDER_CODEX: &str = "codex";
const PROVIDER_ANTIGRAVITY: &str = "antigravity";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const ANTIGRAVITY_PROVIDER_TIMEOUT: Duration = Duration::from_secs(12);
const ANTIGRAVITY_CLI_REFRESH_TIMEOUT: Duration = Duration::from_secs(8);
const ANTIGRAVITY_CLI_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(5 * 60);
const ANTIGRAVITY_TOKEN_EXPIRY_SKEW_SECONDS: i64 = 60;
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

/// One Antigravity model quota bucket.
#[derive(Debug, Clone, Serialize)]
pub struct ModelUsage {
    /// Stable upstream bucket id.
    pub id: String,
    /// Human-readable model/bucket name.
    pub name: String,
    /// Remaining allowance fraction (0-1).
    pub remaining_fraction: f64,
    /// Percentage of the allowance still available (0-100).
    pub remaining_percent: f64,
    /// Percentage of the allowance already consumed (0-100).
    pub used_percent: f64,
    /// ISO 8601 timestamp when the bucket resets, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    /// Seconds until the bucket resets, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_after_seconds: Option<i64>,
    /// Upstream reset/limit description, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ModelUsage {
    fn from_remaining_fraction(
        id: String,
        name: String,
        remaining_fraction: f64,
        resets_at: Option<String>,
        description: Option<String>,
    ) -> Self {
        let remaining_fraction = remaining_fraction.clamp(0.0, 1.0);
        let remaining_percent = (remaining_fraction * 100.0).clamp(0.0, 100.0);
        let used_percent = (100.0 - remaining_percent).clamp(0.0, 100.0);
        let reset_after_seconds = resets_at.as_deref().and_then(seconds_until);
        Self {
            id,
            name,
            remaining_fraction,
            remaining_percent,
            used_percent,
            resets_at,
            reset_after_seconds,
            description,
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
    /// Per-model quota buckets. Present for Antigravity only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelUsage>,
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
            models: Vec::new(),
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
pub async fn get_coding_usage(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let antigravity_config = antigravity_usage_config(state.config_snapshot().as_ref());
    let (claude, codex, antigravity) = tokio::join!(
        resolve_provider(PROVIDER_CLAUDE, "Claude Code", fetch_claude_usage()),
        resolve_provider(PROVIDER_CODEX, "Codex", fetch_codex_usage()),
        resolve_provider(
            PROVIDER_ANTIGRAVITY,
            "Antigravity",
            fetch_antigravity_usage_with_timeout(antigravity_config),
        ),
    );

    let body = CodingUsageResponse {
        providers: vec![claude, codex, antigravity],
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
    let (token, plan) = claude_oauth::read_stored_oauth_token_and_subscription().await?;

    let client = http_client()?;
    let response = client
        .get(CLAUDE_USAGE_URL)
        .bearer_auth(&token)
        .header("anthropic-beta", claude_oauth::CLAUDE_OAUTH_BETA)
        .header(reqwest::header::USER_AGENT, claude_oauth::CLAUDE_USER_AGENT)
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
        models: Vec::new(),
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
    Some(UsageWindow::from_used_percent(
        used,
        resets_at,
        reset_after_seconds,
    ))
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
    // Window position is not semantic: Codex historically returned the
    // 5-hour allowance as primary and the weekly allowance as secondary, but
    // now also returns the weekly allowance alone as primary. Identify the
    // weekly window by its declared duration in either slot.
    let weekly = ["primary_window", "secondary_window"]
        .into_iter()
        .filter_map(|key| rate_limit.get(key))
        .filter(|window| codex_window_is_weekly(window))
        .find_map(parse_codex_window);
    // Keep the legacy primary-window fallback for the session allowance, but
    // never expose a confirmed weekly primary window as a 5-hour limit.
    let session = rate_limit
        .get("primary_window")
        .filter(|window| !codex_window_is_weekly(window))
        .and_then(parse_codex_window);
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
        models: Vec::new(),
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
    Some(UsageWindow::from_used_percent(
        used,
        resets_at,
        reset_after_seconds,
    ))
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
    let value: Value = serde_json::from_str(&contents).map_err(|error| {
        format!("Codex auth file (~/.codex/auth.json) was not valid JSON: {error}")
    })?;
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
// Antigravity
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AntigravityUsageConfig {
    antigravity_bin: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AntigravityTokenPayload {
    token: AntigravityToken,
}

#[derive(Debug, Clone, Deserialize)]
struct AntigravityToken {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expiry: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AntigravityRefreshResponse {
    access_token: Option<String>,
}

struct AntigravityRequestError {
    status: Option<u16>,
    message: String,
}

async fn fetch_antigravity_usage_with_timeout(
    config: AntigravityUsageConfig,
) -> Result<ProviderUsage, String> {
    match timeout(
        ANTIGRAVITY_PROVIDER_TIMEOUT,
        fetch_antigravity_usage(config),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err("Antigravity usage request timed out".to_string()),
    }
}

async fn fetch_antigravity_usage(config: AntigravityUsageConfig) -> Result<ProviderUsage, String> {
    let client = http_client()?;
    let token = resolve_antigravity_access_token(&config, false).await?;
    let project = fetch_antigravity_project(&client, &token).await?;

    let usage = match fetch_antigravity_quota(&client, &token, &project).await {
        Ok(value) => value,
        Err(error) if error.status == Some(401) => {
            let token = resolve_antigravity_access_token(&config, true).await?;
            let project = fetch_antigravity_project(&client, &token).await?;
            fetch_antigravity_quota(&client, &token, &project)
                .await
                .map_err(|retry_error| retry_error.message)?
        }
        Err(error) => return Err(error.message),
    };

    parse_antigravity_usage(&usage)
}

fn antigravity_usage_config(config: &GaryxConfig) -> AntigravityUsageConfig {
    AntigravityUsageConfig {
        antigravity_bin: configured_antigravity_bin(config),
    }
}

fn configured_antigravity_bin(config: &GaryxConfig) -> String {
    for key in ["antigravity", "agy", "antigravity_cli"] {
        if let Some(value) = config.agents.get(key)
            && let Ok(agent_config) = serde_json::from_value::<AgentProviderConfig>(value.clone())
            && ProviderType::from_slug(&agent_config.provider_type)
                == Some(ProviderType::AntigravityCli)
        {
            let trimmed = agent_config.antigravity_bin.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }
    "agy".to_string()
}

async fn resolve_antigravity_access_token(
    config: &AntigravityUsageConfig,
    force_refresh: bool,
) -> Result<String, String> {
    let token = read_antigravity_keychain_token().await?;
    if !force_refresh && antigravity_token_is_fresh(&token) {
        return Ok(token.access_token);
    }

    if let Some(refresh_token) = token.refresh_token.as_deref()
        && !refresh_token.trim().is_empty()
        && let Ok(access_token) = refresh_antigravity_oauth_token(refresh_token).await
    {
        return Ok(access_token);
    }

    refresh_antigravity_keychain_with_cli(config).await?;
    let token = read_antigravity_keychain_token().await?;
    if antigravity_token_is_fresh(&token) {
        return Ok(token.access_token);
    }
    Err("Antigravity credentials are expired; run Antigravity once to refresh login".to_string())
}

fn antigravity_token_is_fresh(token: &AntigravityToken) -> bool {
    let Some(expiry) = token.expiry.as_deref() else {
        return false;
    };
    let Ok(expiry) = DateTime::parse_from_rfc3339(expiry) else {
        return false;
    };
    expiry.with_timezone(&Utc) - chrono::Duration::seconds(ANTIGRAVITY_TOKEN_EXPIRY_SKEW_SECONDS)
        > Utc::now()
}

#[cfg(target_os = "macos")]
async fn read_antigravity_keychain_token() -> Result<AntigravityToken, String> {
    let output = tokio::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
            "-w",
        ])
        .output()
        .await
        .map_err(|error| format!("Antigravity keychain lookup failed to launch: {error}"))?;
    if !output.status.success() {
        return Err("Antigravity credentials not found in keychain".to_string());
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    parse_antigravity_keychain_payload(raw.trim())
}

#[cfg(not(target_os = "macos"))]
async fn read_antigravity_keychain_token() -> Result<AntigravityToken, String> {
    Err("Antigravity usage is only available on macOS keychain hosts".to_string())
}

fn parse_antigravity_keychain_payload(raw: &str) -> Result<AntigravityToken, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Antigravity keychain entry was empty".to_string());
    }
    let json = if let Some(encoded) = trimmed.strip_prefix(ANTIGRAVITY_KEYCHAIN_PREFIX) {
        let decoded = BASE64
            .decode(encoded)
            .map_err(|_| "Antigravity keychain entry was not valid base64".to_string())?;
        String::from_utf8(decoded)
            .map_err(|_| "Antigravity keychain entry was not UTF-8".to_string())?
    } else {
        trimmed.to_owned()
    };
    let payload: AntigravityTokenPayload = serde_json::from_str(&json)
        .map_err(|error| format!("Antigravity keychain entry was not JSON: {error}"))?;
    if payload.token.access_token.trim().is_empty() {
        return Err("Antigravity keychain entry missing token.access_token".to_string());
    }
    Ok(payload.token)
}

async fn refresh_antigravity_oauth_token(refresh_token: &str) -> Result<String, String> {
    let client = http_client()?;
    let response = client
        .post(ANTIGRAVITY_OAUTH_TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", ANTIGRAVITY_OAUTH_CLIENT_ID),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(|error| format!("Antigravity token refresh request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Antigravity token refresh request returned HTTP {status}"
        ));
    }
    let text = response
        .text()
        .await
        .map_err(|error| format!("Antigravity token refresh response unreadable: {error}"))?;
    let value: AntigravityRefreshResponse = serde_json::from_str(&text)
        .map_err(|error| format!("Antigravity token refresh response was not JSON: {error}"))?;
    value
        .access_token
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "Antigravity token refresh response missing access_token".to_string())
}

async fn refresh_antigravity_keychain_with_cli(
    config: &AntigravityUsageConfig,
) -> Result<(), String> {
    if !claim_antigravity_cli_refresh_slot() {
        return Err(
            "Antigravity CLI refresh was attempted recently; run Antigravity once to refresh login"
                .to_string(),
        );
    }
    let output = timeout(
        ANTIGRAVITY_CLI_REFRESH_TIMEOUT,
        tokio::process::Command::new(&config.antigravity_bin)
            .arg("models")
            .output(),
    )
    .await
    .map_err(|_| "Antigravity CLI refresh timed out".to_string())?
    .map_err(|error| format!("Antigravity CLI refresh failed to launch: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err("Antigravity CLI refresh failed; run Antigravity once to refresh login".to_string())
}

fn claim_antigravity_cli_refresh_slot() -> bool {
    static LAST_ATTEMPT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let lock = LAST_ATTEMPT.get_or_init(|| Mutex::new(None));
    let Ok(mut guard) = lock.lock() else {
        return false;
    };
    if let Some(last_attempt) = *guard
        && last_attempt.elapsed() < ANTIGRAVITY_CLI_REFRESH_MIN_INTERVAL
    {
        return false;
    }
    *guard = Some(Instant::now());
    true
}

async fn fetch_antigravity_project(
    client: &reqwest::Client,
    token: &str,
) -> Result<String, String> {
    let response = client
        .post(ANTIGRAVITY_LOAD_CODE_ASSIST_URL)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, ANTIGRAVITY_USER_AGENT)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|error| format!("Antigravity project request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Antigravity project request returned HTTP {status}"
        ));
    }
    let text = response
        .text()
        .await
        .map_err(|error| format!("Antigravity project response unreadable: {error}"))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("Antigravity project response was not JSON: {error}"))?;
    parse_antigravity_project(&value)
        .ok_or_else(|| "Antigravity project response missing cloudaicompanionProject".to_string())
}

fn parse_antigravity_project(value: &Value) -> Option<String> {
    value
        .get("cloudaicompanionProject")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("onboardUser")
                .and_then(|onboard| onboard.get("cloudaicompanionProject"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|project| !project.is_empty())
        .map(ToOwned::to_owned)
}

async fn fetch_antigravity_quota(
    client: &reqwest::Client,
    token: &str,
    project: &str,
) -> Result<Value, AntigravityRequestError> {
    let response = client
        .post(ANTIGRAVITY_USAGE_URL)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, ANTIGRAVITY_USER_AGENT)
        .json(&serde_json::json!({ "project": project }))
        .send()
        .await
        .map_err(|error| AntigravityRequestError {
            status: None,
            message: format!("Antigravity usage request failed: {error}"),
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(AntigravityRequestError {
            status: Some(status.as_u16()),
            message: format!("Antigravity usage request returned HTTP {status}"),
        });
    }
    let text = response
        .text()
        .await
        .map_err(|error| AntigravityRequestError {
            status: Some(status.as_u16()),
            message: format!("Antigravity usage response unreadable: {error}"),
        })?;
    serde_json::from_str(&text).map_err(|error| AntigravityRequestError {
        status: Some(status.as_u16()),
        message: format!("Antigravity usage response was not JSON: {error}"),
    })
}

fn parse_antigravity_usage(value: &Value) -> Result<ProviderUsage, String> {
    let models: Vec<ModelUsage> = value
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            group
                .get("buckets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(parse_antigravity_model_usage)
        .collect();

    if models.is_empty() {
        return Err("Antigravity usage response had no usable model buckets".to_string());
    }

    Ok(ProviderUsage {
        id: PROVIDER_ANTIGRAVITY,
        name: "Antigravity",
        available: true,
        stale: false,
        plan: None,
        weekly: None,
        session: None,
        models,
        error: None,
    })
}

fn parse_antigravity_model_usage(bucket: &Value) -> Option<ModelUsage> {
    let id = bucket
        .get("bucketId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let name = bucket
        .get("displayName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&id)
        .to_owned();
    let remaining_fraction = bucket.get("remainingFraction").and_then(Value::as_f64)?;
    let resets_at = bucket
        .get("resetTime")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let description = bucket
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Some(ModelUsage::from_remaining_fraction(
        id,
        name,
        remaining_fraction,
        resets_at,
        description,
    ))
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
        assert_eq!(
            weekly.resets_at.as_deref(),
            Some("2030-01-07T11:00:00+00:00")
        );
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
    fn parses_codex_weekly_primary_without_session() {
        let payload = json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 36,
                    "limit_window_seconds": 604800,
                    "reset_after_seconds": 479565,
                    "reset_at": 1784361431
                }
            }
        });

        let usage = parse_codex_usage(&payload).unwrap();
        let weekly = usage.weekly.expect("weekly window");
        assert_eq!(weekly.remaining_percent, 64.0);
        assert_eq!(weekly.reset_after_seconds, Some(479565));
        assert!(
            usage.session.is_none(),
            "weekly window is not a session limit"
        );
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
        assert!(
            usage.weekly.is_none(),
            "weekly without used_percent must be unavailable"
        );
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
        assert!(
            usage.weekly.is_none(),
            "daily secondary window is not weekly"
        );
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

    #[test]
    fn parses_antigravity_keychain_base64_payload() {
        let payload = json!({
            "token": {
                "access_token": "test-access-token",
                "refresh_token": "test-refresh-token",
                "token_type": "Bearer",
                "expiry": "2030-01-01T00:00:00+08:00"
            },
            "auth_method": "consumer"
        })
        .to_string();
        let raw = format!(
            "{}{}",
            ANTIGRAVITY_KEYCHAIN_PREFIX,
            BASE64.encode(payload.as_bytes())
        );

        let token = parse_antigravity_keychain_payload(&raw).unwrap();
        assert_eq!(token.access_token, "test-access-token");
        assert_eq!(token.refresh_token.as_deref(), Some("test-refresh-token"));
        assert_eq!(token.expiry.as_deref(), Some("2030-01-01T00:00:00+08:00"));
        assert!(antigravity_token_is_fresh(&token));
    }

    #[test]
    fn antigravity_keychain_payload_rejects_missing_access_token() {
        let payload = json!({
            "token": {
                "refresh_token": "test-refresh-token",
                "expiry": "2030-01-01T00:00:00+08:00"
            }
        })
        .to_string();

        let error = parse_antigravity_keychain_payload(&payload).unwrap_err();
        assert!(
            error.contains("token.access_token") || error.contains("JSON"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parses_antigravity_project_from_load_code_assist() {
        let top_level = json!({
            "cloudaicompanionProject": "test-project",
            "gcpManaged": true
        });
        assert_eq!(
            parse_antigravity_project(&top_level).as_deref(),
            Some("test-project")
        );

        let onboard = json!({
            "onboardUser": {
                "cloudaicompanionProject": "onboard-project"
            }
        });
        assert_eq!(
            parse_antigravity_project(&onboard).as_deref(),
            Some("onboard-project")
        );
    }

    #[test]
    fn parses_antigravity_model_buckets() {
        let payload = json!({
            "groups": [
                {
                    "buckets": [
                        {
                            "bucketId": "claude-opus-4-6-thinking",
                            "displayName": "Claude Opus 4.6 (Thinking)",
                            "resetTime": "2030-01-01T00:00:00Z",
                            "description": "Quota resets in 1 hour.",
                            "remainingFraction": 0.98571426
                        },
                        {
                            "bucketId": "gemini-3-5-flash-high",
                            "displayName": "Gemini 3.5 Flash (High)",
                            "remainingFraction": 1.2
                        },
                        {
                            "bucketId": "missing-fraction",
                            "displayName": "Missing Fraction"
                        }
                    ]
                }
            ]
        });

        let usage = parse_antigravity_usage(&payload).unwrap();
        assert!(usage.available);
        assert_eq!(usage.id, PROVIDER_ANTIGRAVITY);
        assert_eq!(usage.models.len(), 2);
        let opus = &usage.models[0];
        assert_eq!(opus.id, "claude-opus-4-6-thinking");
        assert_eq!(opus.name, "Claude Opus 4.6 (Thinking)");
        assert!((opus.remaining_fraction - 0.98571426).abs() < f64::EPSILON);
        assert!((opus.remaining_percent - 98.571426).abs() < 0.0001);
        assert!((opus.used_percent - 1.428574).abs() < 0.0001);
        assert_eq!(opus.resets_at.as_deref(), Some("2030-01-01T00:00:00Z"));
        assert!(opus.reset_after_seconds.is_some());
        assert_eq!(opus.description.as_deref(), Some("Quota resets in 1 hour."));

        let gemini = &usage.models[1];
        assert_eq!(gemini.remaining_fraction, 1.0);
        assert_eq!(gemini.remaining_percent, 100.0);
        assert_eq!(gemini.used_percent, 0.0);
    }

    #[test]
    fn antigravity_usage_without_usable_buckets_is_error() {
        let payload = json!({
            "groups": [
                {
                    "buckets": [
                        {"bucketId": "missing-fraction", "displayName": "Missing Fraction"}
                    ]
                }
            ]
        });
        assert!(parse_antigravity_usage(&payload).is_err());
    }

    #[test]
    fn configured_antigravity_bin_uses_default_agent_config() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "antigravity".to_string(),
            json!({
                "provider_type": "antigravity",
                "antigravity_bin": "test-agy"
            }),
        );
        assert_eq!(configured_antigravity_bin(&config), "test-agy");

        config.agents.clear();
        assert_eq!(configured_antigravity_bin(&config), "agy");
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
        match fetch_antigravity_usage_with_timeout(AntigravityUsageConfig {
            antigravity_bin: configured_antigravity_bin(&GaryxConfig::default()),
        })
        .await
        {
            Ok(usage) => {
                assert!(usage.available);
                println!(
                    "antigravity: bucket_count={} buckets={:?}",
                    usage.models.len(),
                    usage
                        .models
                        .iter()
                        .map(|model| model.name.as_str())
                        .collect::<Vec<_>>()
                );
            }
            Err(error) => println!("antigravity unavailable: {error}"),
        }
    }
}
