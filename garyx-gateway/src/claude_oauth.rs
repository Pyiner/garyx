use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub(crate) const CLAUDE_USER_AGENT: &str = "claude-code/2.0.32";
pub(crate) const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";
pub(crate) const CLAUDE_ANTHROPIC_VERSION: &str = "2023-06-01";
pub(crate) const CLAUDE_OAUTH_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";

const CLAUDE_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const OAUTH_REFRESH_TIMEOUT: Duration = Duration::from_secs(10);

const CLAUDE_OAUTH_ENV_KEYS: &[&str] = &[
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_OAUTH_TOKEN",
];
#[cfg(target_os = "macos")]
const KEYCHAIN_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OAuthRefreshErrorKind {
    MissingRefreshToken,
    RefreshTokenExpired,
    RateLimited,
    ReauthRequired,
    Network,
    InvalidResponse,
    Persistence,
    ConcurrentUpdate,
    Upstream,
}

#[derive(Debug, Clone)]
pub(crate) struct OAuthRefreshError {
    pub kind: OAuthRefreshErrorKind,
    pub message: String,
    pub retry_after_seconds: Option<u64>,
}

impl OAuthRefreshError {
    fn new(kind: OAuthRefreshErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    fn with_retry_after(mut self, retry_after_seconds: Option<u64>) -> Self {
        self.retry_after_seconds = retry_after_seconds;
        self
    }
}

impl std::fmt::Display for OAuthRefreshError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaudeOAuthCredentialSnapshot {
    pub access_token: String,
    pub subscription: Option<String>,
    expires_at_ms: Option<i64>,
    refresh_token_expires_at_ms: Option<i64>,
    has_refresh_token: bool,
}

impl ClaudeOAuthCredentialSnapshot {
    pub(crate) fn needs_refresh(&self, now_ms: i64) -> bool {
        self.access_token.is_empty()
            || self
                .expires_at_ms
                .is_some_and(|expires_at| now_ms >= expires_at)
    }

    pub(crate) fn can_refresh(&self, now_ms: i64) -> bool {
        self.has_refresh_token
            && self
                .refresh_token_expires_at_ms
                .is_none_or(|expires_at| now_ms < expires_at)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum OAuthCredentialsLocation {
    #[cfg(target_os = "macos")]
    Keychain {
        service: String,
        account: String,
    },
    File(PathBuf),
}

#[derive(Debug, Clone)]
struct StoredOAuthCredentials {
    value: Value,
    location: OAuthCredentialsLocation,
}

pub(crate) async fn read_oauth_token() -> Result<String, String> {
    read_oauth_token_and_subscription()
        .await
        .map(|(token, _)| token)
}

pub(crate) async fn read_oauth_token_and_subscription() -> Result<(String, Option<String>), String>
{
    match read_stored_oauth_token_and_subscription().await {
        Ok(credentials) => Ok(credentials),
        Err(credentials_error) => read_env_oauth_token()
            .map(|token| (token, None))
            .ok_or(credentials_error),
    }
}

pub(crate) async fn read_stored_oauth_token_and_subscription()
-> Result<(String, Option<String>), String> {
    read_stored_oauth_token_and_subscription_for_config_dir(None).await
}

pub(crate) async fn read_stored_oauth_token_and_subscription_for_config_dir(
    config_dir: Option<&Path>,
) -> Result<(String, Option<String>), String> {
    let credentials = read_stored_oauth_credential_snapshot(config_dir).await?;
    if credentials.access_token.is_empty() {
        return Err("Claude credentials missing claudeAiOauth.accessToken".to_owned());
    }
    Ok((credentials.access_token, credentials.subscription))
}

pub(crate) async fn read_stored_oauth_credential_snapshot(
    config_dir: Option<&Path>,
) -> Result<ClaudeOAuthCredentialSnapshot, String> {
    let credentials = read_oauth_credentials(config_dir).await?;
    credential_snapshot(&credentials)
}

fn read_env_oauth_token() -> Option<String> {
    CLAUDE_OAUTH_ENV_KEYS.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

async fn read_oauth_credentials(config_dir: Option<&Path>) -> Result<Value, String> {
    read_stored_oauth_credentials(config_dir)
        .await
        .map(|credentials| credentials.value)
}

async fn read_stored_oauth_credentials(
    config_dir: Option<&Path>,
) -> Result<StoredOAuthCredentials, String> {
    #[cfg(target_os = "macos")]
    {
        let service = claude_keychain_service(config_dir);
        let account = keychain_account();
        let location = OAuthCredentialsLocation::Keychain { service, account };
        if let Ok(value) = read_oauth_credentials_at_location(&location).await {
            return Ok(StoredOAuthCredentials { value, location });
        }
        let location = OAuthCredentialsLocation::File(credentials_file_path(config_dir)?);
        let value = read_oauth_credentials_at_location(&location).await?;
        Ok(StoredOAuthCredentials { value, location })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let location = OAuthCredentialsLocation::File(credentials_file_path(config_dir)?);
        let value = read_oauth_credentials_at_location(&location).await?;
        Ok(StoredOAuthCredentials { value, location })
    }
}

#[cfg(test)]
async fn read_oauth_credentials_file(config_dir: Option<&Path>) -> Result<Value, String> {
    let path = credentials_file_path(config_dir)?;
    read_oauth_credentials_file_at_path(&path).await
}

fn credentials_file_path(config_dir: Option<&Path>) -> Result<PathBuf, String> {
    config_dir
        .map(Path::to_path_buf)
        .or_else(|| garyx_models::local_paths::home_dir().map(|home| home.join(".claude")))
        .map(|claude_dir| claude_dir.join(".credentials.json"))
        .ok_or_else(|| "Claude credentials directory is unavailable".to_owned())
}

async fn read_oauth_credentials_file_at_path(path: &Path) -> Result<Value, String> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .map_err(|_| format!("Claude credentials not found ({} missing)", path.display()))?;
    serde_json::from_str(&contents).map_err(|error| {
        format!(
            "Claude credentials file ({}) was not valid JSON: {error}",
            path.display()
        )
    })
}

#[cfg(all(test, target_os = "macos"))]
fn select_oauth_credentials(
    file: Result<Value, String>,
    keychain: Result<Value, String>,
) -> Result<Value, String> {
    keychain.or(file)
}

#[cfg(all(test, not(target_os = "macos")))]
fn select_oauth_credentials(
    file: Result<Value, String>,
    _keychain: Result<Value, String>,
) -> Result<Value, String> {
    file
}

#[cfg(target_os = "macos")]
async fn read_oauth_keychain_service(service: &str, account: &str) -> Result<Value, String> {
    let mut command = tokio::process::Command::new("security");
    command
        .args(["find-generic-password", "-s"])
        .arg(service)
        .arg("-a")
        .arg(account)
        .arg("-w");
    let output = tokio::time::timeout(KEYCHAIN_COMMAND_TIMEOUT, command.output())
        .await
        .map_err(|_| "Claude keychain lookup timed out".to_owned())?
        .map_err(|error| format!("Claude keychain lookup failed to launch: {error}"))?;
    if !output.status.success() {
        return Err("Claude credentials not found in keychain".to_string());
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(raw.trim())
        .map_err(|error| format!("Claude keychain entry was not JSON: {error}"))
}

async fn read_oauth_credentials_at_location(
    location: &OAuthCredentialsLocation,
) -> Result<Value, String> {
    match location {
        #[cfg(target_os = "macos")]
        OAuthCredentialsLocation::Keychain { service, account } => {
            read_oauth_keychain_service(service, account).await
        }
        OAuthCredentialsLocation::File(path) => read_oauth_credentials_file_at_path(path).await,
    }
}

async fn write_oauth_credentials_at_location(
    location: &OAuthCredentialsLocation,
    credentials: &Value,
) -> Result<(), String> {
    let contents = serde_json::to_string(credentials)
        .map_err(|error| format!("Claude credentials could not be encoded: {error}"))?;
    match location {
        #[cfg(target_os = "macos")]
        OAuthCredentialsLocation::Keychain { service, account } => {
            let mut command = tokio::process::Command::new("security");
            command
                .args(["add-generic-password", "-U", "-s"])
                .arg(service)
                .arg("-a")
                .arg(account)
                .arg("-w")
                .arg(contents);
            let output = tokio::time::timeout(KEYCHAIN_COMMAND_TIMEOUT, command.output())
                .await
                .map_err(|_| "Claude keychain update timed out".to_owned())?
                .map_err(|error| format!("Claude keychain update failed to launch: {error}"))?;
            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!("Claude keychain update failed: {}", stderr.trim()))
            }
        }
        OAuthCredentialsLocation::File(path) => {
            let parent = path.parent().ok_or_else(|| {
                format!("Claude credentials path has no parent: {}", path.display())
            })?;
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                format!("Claude credentials directory could not be created: {error}")
            })?;
            let temporary = parent.join(format!(
                ".credentials.json.garyx-refresh-{}",
                uuid::Uuid::new_v4()
            ));
            let write_result = async {
                let mut options = tokio::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    options.mode(0o600);
                }
                let mut file = options.open(&temporary).await.map_err(|error| {
                    format!("Claude credentials temporary file could not be created: {error}")
                })?;
                file.write_all(contents.as_bytes()).await.map_err(|error| {
                    format!("Claude credentials temporary file could not be written: {error}")
                })?;
                file.flush().await.map_err(|error| {
                    format!("Claude credentials temporary file could not be flushed: {error}")
                })?;
                file.sync_all().await.map_err(|error| {
                    format!("Claude credentials temporary file could not be synced: {error}")
                })?;
                drop(file);
                tokio::fs::rename(&temporary, path).await.map_err(|error| {
                    format!("Claude credentials file could not be replaced atomically: {error}")
                })
            }
            .await;
            if write_result.is_err() {
                let _ = tokio::fs::remove_file(&temporary).await;
            }
            write_result
        }
    }
}

async fn persist_refreshed_credentials(
    location: &OAuthCredentialsLocation,
    original: &Value,
    updated: &Value,
    now_ms: i64,
) -> Result<ClaudeOAuthCredentialSnapshot, OAuthRefreshError> {
    let expected_token = oauth_token_from_credentials(updated)
        .map_err(|error| OAuthRefreshError::new(OAuthRefreshErrorKind::InvalidResponse, error))?;
    let mut last_error = "Claude Code refreshed credentials could not be saved.".to_owned();
    for attempt in 0..3 {
        if let Err(error) = write_oauth_credentials_at_location(location, updated).await {
            last_error = error;
        }
        match read_oauth_credentials_at_location(location).await {
            Ok(persisted) => {
                if let Ok(snapshot) = credential_snapshot(&persisted)
                    && snapshot.access_token == expected_token
                {
                    return Ok(snapshot);
                }
                if let Some(winner) = concurrent_refresh_winner(original, &persisted, now_ms) {
                    return Ok(winner);
                }
                if persisted != *original {
                    return Err(OAuthRefreshError::new(
                        OAuthRefreshErrorKind::ConcurrentUpdate,
                        "Claude Code credentials changed while refreshed credentials were being saved.",
                    ));
                }
            }
            Err(error) => last_error = error,
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    Err(OAuthRefreshError::new(
        OAuthRefreshErrorKind::Persistence,
        last_error,
    ))
}

#[cfg(target_os = "macos")]
pub(crate) async fn delete_scoped_oauth_keychain(config_dir: &Path) -> Result<(), String> {
    let service = claude_keychain_service(Some(config_dir));
    let mut command = tokio::process::Command::new("security");
    command
        .args(["delete-generic-password", "-s"])
        .arg(&service)
        .arg("-a")
        .arg(keychain_account());
    let output = tokio::time::timeout(KEYCHAIN_COMMAND_TIMEOUT, command.output())
        .await
        .map_err(|_| "Claude keychain cleanup timed out".to_owned())?
        .map_err(|error| format!("Claude keychain cleanup failed to launch: {error}"))?;
    if output.status.success() || output.status.code() == Some(44) {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.to_ascii_lowercase().contains("could not be found") {
        return Ok(());
    }
    Err(format!(
        "Claude keychain cleanup failed for service {service}: {}",
        stderr.trim()
    ))
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn delete_scoped_oauth_keychain(_config_dir: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn keychain_account() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_owned())
}

pub(crate) fn claude_keychain_service(config_dir: Option<&Path>) -> String {
    let Some(config_dir) = config_dir else {
        return "Claude Code-credentials".to_owned();
    };
    let path = absolute_config_dir(config_dir);
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let hash = format!("{digest:x}");
    format!("Claude Code-credentials-{}", &hash[..8])
}

fn absolute_config_dir(config_dir: &Path) -> PathBuf {
    if config_dir.is_absolute() {
        config_dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(config_dir))
            .unwrap_or_else(|_| config_dir.to_path_buf())
    }
}

#[cfg(test)]
fn token_source_label() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "keychain"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "file"
    }
}

fn oauth_token_from_credentials(credentials: &Value) -> Result<String, String> {
    credentials
        .get("claudeAiOauth")
        .and_then(|oauth| oauth.get("accessToken"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "Claude credentials missing claudeAiOauth.accessToken".to_string())
}

fn oauth_subscription_from_credentials(credentials: &Value) -> Option<String> {
    credentials
        .get("claudeAiOauth")
        .and_then(|oauth| oauth.get("subscriptionType"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn credential_snapshot(credentials: &Value) -> Result<ClaudeOAuthCredentialSnapshot, String> {
    let oauth = credentials
        .get("claudeAiOauth")
        .and_then(Value::as_object)
        .ok_or_else(|| "Claude credentials missing claudeAiOauth".to_owned())?;
    let access_token = oauth_token_from_credentials(credentials).unwrap_or_default();
    let has_refresh_token = oauth
        .get("refreshToken")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    Ok(ClaudeOAuthCredentialSnapshot {
        access_token,
        subscription: oauth_subscription_from_credentials(credentials),
        expires_at_ms: oauth.get("expiresAt").and_then(Value::as_i64),
        refresh_token_expires_at_ms: oauth.get("refreshTokenExpiresAt").and_then(Value::as_i64),
        has_refresh_token,
    })
}

fn refresh_token_from_credentials(credentials: &Value) -> Option<String> {
    credentials
        .get("claudeAiOauth")
        .and_then(|oauth| oauth.get("refreshToken"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn concurrent_refresh_winner(
    original: &Value,
    current: &Value,
    now_ms: i64,
) -> Option<ClaudeOAuthCredentialSnapshot> {
    if current == original {
        return None;
    }
    let snapshot = credential_snapshot(current).ok()?;
    let token_changed =
        oauth_token_from_credentials(current).ok() != oauth_token_from_credentials(original).ok();
    let refresh_changed =
        refresh_token_from_credentials(current) != refresh_token_from_credentials(original);
    (token_changed || refresh_changed)
        .then_some(snapshot)
        .filter(|snapshot| !snapshot.needs_refresh(now_ms))
}

async fn recover_concurrent_refresh(
    stored: &StoredOAuthCredentials,
    now_ms: i64,
) -> Option<ClaudeOAuthCredentialSnapshot> {
    let current = read_oauth_credentials_at_location(&stored.location)
        .await
        .ok()?;
    concurrent_refresh_winner(&stored.value, &current, now_ms)
}

fn merge_refreshed_credentials(
    credentials: &Value,
    response: &Value,
    now_ms: i64,
) -> Result<Value, OAuthRefreshError> {
    let access_token = response
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OAuthRefreshError::new(
                OAuthRefreshErrorKind::InvalidResponse,
                "Claude OAuth refresh response did not include an access token.",
            )
        })?;
    let mut updated = credentials.clone();
    let oauth = updated
        .get_mut("claudeAiOauth")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            OAuthRefreshError::new(
                OAuthRefreshErrorKind::InvalidResponse,
                "Claude credentials did not contain an OAuth object.",
            )
        })?;
    oauth.insert(
        "accessToken".to_owned(),
        Value::String(access_token.to_owned()),
    );
    if let Some(expires_in) = response.get("expires_in").and_then(Value::as_i64) {
        oauth.insert(
            "expiresAt".to_owned(),
            Value::from(now_ms.saturating_add(expires_in.saturating_mul(1_000))),
        );
    }
    if let Some(refresh_token) = response
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        oauth.insert(
            "refreshToken".to_owned(),
            Value::String(refresh_token.to_owned()),
        );
    }
    if let Some(scope) = response
        .get("scope")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        oauth.insert(
            "scopes".to_owned(),
            Value::Array(
                scope
                    .split_ascii_whitespace()
                    .map(|value| Value::String(value.to_owned()))
                    .collect(),
            ),
        );
    }
    Ok(updated)
}

pub(crate) async fn refresh_stored_oauth_credentials_for_config_dir_with(
    config_dir: Option<&Path>,
    client: &reqwest::Client,
    token_url: &str,
    now_ms: i64,
) -> Result<ClaudeOAuthCredentialSnapshot, OAuthRefreshError> {
    let stored = read_stored_oauth_credentials(config_dir)
        .await
        .map_err(|error| OAuthRefreshError::new(OAuthRefreshErrorKind::Persistence, error))?;
    let before = credential_snapshot(&stored.value)
        .map_err(|error| OAuthRefreshError::new(OAuthRefreshErrorKind::InvalidResponse, error))?;
    let refresh_token = refresh_token_from_credentials(&stored.value).ok_or_else(|| {
        OAuthRefreshError::new(
            OAuthRefreshErrorKind::MissingRefreshToken,
            "Claude Code credentials cannot be refreshed; sign in again.",
        )
    })?;
    if !before.can_refresh(now_ms) {
        return Err(OAuthRefreshError::new(
            OAuthRefreshErrorKind::RefreshTokenExpired,
            "Claude Code refresh credentials expired; sign in again.",
        ));
    }

    let response = tokio::time::timeout(
        OAUTH_REFRESH_TIMEOUT,
        client
            .post(token_url)
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("client_id", CLAUDE_OAUTH_CLIENT_ID),
            ])
            .send(),
    )
    .await
    .map_err(|_| {
        OAuthRefreshError::new(
            OAuthRefreshErrorKind::Network,
            "Claude Code credential refresh timed out.",
        )
    })?
    .map_err(|error| {
        OAuthRefreshError::new(
            OAuthRefreshErrorKind::Network,
            format!("Claude Code credential refresh failed: {error}"),
        )
    })?;
    let status = response.status();
    let retry_after = parse_retry_after_seconds(response.headers());
    let text = response.text().await.map_err(|error| {
        OAuthRefreshError::new(
            OAuthRefreshErrorKind::InvalidResponse,
            format!("Claude Code credential refresh response was unreadable: {error}"),
        )
    })?;
    if !status.is_success() {
        if let Some(winner) = recover_concurrent_refresh(&stored, now_ms).await {
            return Ok(winner);
        }
        let kind = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            OAuthRefreshErrorKind::RateLimited
        } else if matches!(status.as_u16(), 400 | 401 | 403) {
            OAuthRefreshErrorKind::ReauthRequired
        } else {
            OAuthRefreshErrorKind::Upstream
        };
        let message = match kind {
            OAuthRefreshErrorKind::RateLimited => {
                "Claude Code credential refresh is temporarily rate limited.".to_owned()
            }
            OAuthRefreshErrorKind::ReauthRequired => {
                "Claude Code credentials could not be refreshed; sign in again.".to_owned()
            }
            _ => format!("Claude Code credential refresh returned HTTP {status}."),
        };
        return Err(OAuthRefreshError::new(kind, message).with_retry_after(retry_after));
    }
    let response_value: Value = serde_json::from_str(&text).map_err(|error| {
        OAuthRefreshError::new(
            OAuthRefreshErrorKind::InvalidResponse,
            format!("Claude Code credential refresh response was not JSON: {error}"),
        )
    })?;
    let updated = merge_refreshed_credentials(&stored.value, &response_value, now_ms)?;

    let current = read_oauth_credentials_at_location(&stored.location)
        .await
        .map_err(|error| OAuthRefreshError::new(OAuthRefreshErrorKind::Persistence, error))?;
    if current != stored.value {
        if let Some(winner) = concurrent_refresh_winner(&stored.value, &current, now_ms) {
            return Ok(winner);
        }
        return Err(OAuthRefreshError::new(
            OAuthRefreshErrorKind::ConcurrentUpdate,
            "Claude Code credentials changed during refresh; retrying later.",
        ));
    }

    persist_refreshed_credentials(&stored.location, &stored.value, &updated, now_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::sync::{Mutex, OnceLock};

    use serde_json::json;
    use tempfile::tempdir;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvRestore {
        values: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvRestore {
        fn capture(keys: &[&'static str]) -> Self {
            let values = keys
                .iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect();
            Self { values }
        }

        fn set(key: &str, value: impl AsRef<OsStr>) {
            unsafe {
                std::env::set_var(key, value);
            }
        }

        fn remove(key: &str) {
            unsafe {
                std::env::remove_var(key);
            }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (key, value) in &self.values {
                match value {
                    Some(value) => Self::set(key, value),
                    None => Self::remove(key),
                }
            }
        }
    }

    #[test]
    fn reads_token_and_subscription_from_credentials_payload() {
        let credentials = json!({
            "claudeAiOauth": {
                "accessToken": " test-token ",
                "subscriptionType": "max"
            }
        });

        assert_eq!(
            oauth_token_from_credentials(&credentials).as_deref(),
            Ok("test-token")
        );
        assert_eq!(
            oauth_subscription_from_credentials(&credentials).as_deref(),
            Some("max")
        );
    }

    #[test]
    fn rejects_credentials_without_access_token() {
        let credentials = json!({ "claudeAiOauth": { "subscriptionType": "max" } });

        let error = oauth_token_from_credentials(&credentials).unwrap_err();
        assert!(error.contains("claudeAiOauth.accessToken"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn credentials_file_reader_ignores_env_oauth_token() {
        let _lock = env_lock().lock().expect("env lock");
        let temp = tempdir().expect("temp dir");
        let claude_dir = temp.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).expect("claude dir");
        std::fs::write(
            claude_dir.join(".credentials.json"),
            json!({
                "claudeAiOauth": {
                    "accessToken": "file-token",
                    "subscriptionType": "max"
                }
            })
            .to_string(),
        )
        .expect("credentials");

        let _env = EnvRestore::capture(&[
            "HOME",
            "USERPROFILE",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_OAUTH_TOKEN",
        ]);
        EnvRestore::set("HOME", temp.path());
        EnvRestore::remove("USERPROFILE");
        EnvRestore::set("CLAUDE_CODE_OAUTH_TOKEN", "ignored-env-token");
        EnvRestore::remove("ANTHROPIC_AUTH_TOKEN");
        EnvRestore::remove("CLAUDE_OAUTH_TOKEN");

        let credentials = read_oauth_credentials_file(None).await.unwrap();
        let token = oauth_token_from_credentials(&credentials).unwrap();
        let subscription = oauth_subscription_from_credentials(&credentials);

        assert_eq!(token, "file-token");
        assert_eq!(subscription.as_deref(), Some("max"));
    }

    #[test]
    fn keychain_service_scopes_managed_config_directory() {
        assert_eq!(claude_keychain_service(None), "Claude Code-credentials");
        let first = claude_keychain_service(Some(Path::new("/tmp/garyx/claude/work")));
        let second = claude_keychain_service(Some(Path::new("/tmp/garyx/claude/personal")));
        assert!(first.starts_with("Claude Code-credentials-"));
        assert_eq!(first.len(), "Claude Code-credentials-".len() + 8);
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn managed_credentials_reader_uses_requested_config_directory() {
        let temp = tempdir().expect("temp dir");
        std::fs::write(
            temp.path().join(".credentials.json"),
            json!({
                "claudeAiOauth": {
                    "accessToken": "managed-token",
                    "subscriptionType": "max"
                }
            })
            .to_string(),
        )
        .expect("credentials");

        let credentials = read_oauth_credentials_file(Some(temp.path()))
            .await
            .unwrap();
        assert_eq!(
            oauth_token_from_credentials(&credentials).unwrap(),
            "managed-token"
        );
    }

    #[test]
    fn stored_credentials_source_priority_matches_platform() {
        let file = json!({
            "claudeAiOauth": {
                "accessToken": "file-token",
                "subscriptionType": "file-plan"
            }
        });
        let keychain = json!({
            "claudeAiOauth": {
                "accessToken": "keychain-token",
                "subscriptionType": "keychain-plan"
            }
        });

        let selected = select_oauth_credentials(Ok(file), Ok(keychain)).unwrap();
        let token = oauth_token_from_credentials(&selected).unwrap();
        let subscription = oauth_subscription_from_credentials(&selected);

        match token_source_label() {
            "keychain" => {
                assert_eq!(token, "keychain-token");
                assert_eq!(subscription.as_deref(), Some("keychain-plan"));
            }
            "file" => {
                assert_eq!(token, "file-token");
                assert_eq!(subscription.as_deref(), Some("file-plan"));
            }
            other => panic!("unexpected token source label: {other}"),
        }
    }

    #[tokio::test]
    async fn refreshes_expired_credentials_and_persists_rotated_refresh_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=old-refresh-token"))
            .and(body_string_contains(
                "client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "new-access-token",
                "refresh_token": "new-refresh-token",
                "expires_in": 3600,
                "scope": "user:profile user:inference"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let temp = tempdir().expect("temp dir");
        let credentials_path = temp.path().join(".credentials.json");
        std::fs::write(
            &credentials_path,
            json!({
                "claudeAiOauth": {
                    "accessToken": "expired-access-token",
                    "refreshToken": "old-refresh-token",
                    "expiresAt": 1_700_000_000_000_i64,
                    "refreshTokenExpiresAt": 1_800_000_000_000_i64,
                    "subscriptionType": "max",
                    "rateLimitTier": "default_claude_max_20x"
                }
            })
            .to_string(),
        )
        .expect("credentials");

        let client = reqwest::Client::new();
        let refreshed = refresh_stored_oauth_credentials_for_config_dir_with(
            Some(temp.path()),
            &client,
            &format!("{}/v1/oauth/token", server.uri()),
            1_750_000_000_000,
        )
        .await
        .expect("refresh credentials");

        assert_eq!(refreshed.access_token, "new-access-token");
        assert_eq!(refreshed.subscription.as_deref(), Some("max"));
        let persisted: Value = serde_json::from_str(
            &std::fs::read_to_string(credentials_path).expect("persisted credentials"),
        )
        .expect("persisted JSON");
        let oauth = persisted
            .get("claudeAiOauth")
            .and_then(Value::as_object)
            .expect("oauth object");
        assert_eq!(
            oauth.get("accessToken").and_then(Value::as_str),
            Some("new-access-token")
        );
        assert_eq!(
            oauth.get("refreshToken").and_then(Value::as_str),
            Some("new-refresh-token")
        );
        assert_eq!(
            oauth.get("expiresAt").and_then(Value::as_i64),
            Some(1_750_003_600_000)
        );
        assert_eq!(
            oauth.get("rateLimitTier").and_then(Value::as_str),
            Some("default_claude_max_20x"),
            "refresh must preserve Claude-owned credential metadata"
        );
    }

    #[tokio::test]
    async fn concurrent_claude_refresh_winner_is_never_overwritten() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/oauth/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(100))
                    .set_body_json(json!({
                        "access_token": "garyx-access-token",
                        "refresh_token": "garyx-refresh-token",
                        "expires_in": 3600
                    })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let temp = tempdir().expect("temp dir");
        let credentials_path = temp.path().join(".credentials.json");
        std::fs::write(
            &credentials_path,
            json!({
                "claudeAiOauth": {
                    "accessToken": "expired-access-token",
                    "refreshToken": "shared-refresh-token",
                    "expiresAt": 1_700_000_000_000_i64,
                    "refreshTokenExpiresAt": 1_800_000_000_000_i64
                }
            })
            .to_string(),
        )
        .expect("credentials");
        let config_dir = temp.path().to_path_buf();
        let client = reqwest::Client::new();
        let token_url = format!("{}/v1/oauth/token", server.uri());

        let refresh = tokio::spawn(async move {
            refresh_stored_oauth_credentials_for_config_dir_with(
                Some(&config_dir),
                &client,
                &token_url,
                1_750_000_000_000,
            )
            .await
        });
        let mut request_started = false;
        for _ in 0..100 {
            if server
                .received_requests()
                .await
                .is_some_and(|requests| !requests.is_empty())
            {
                request_started = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            request_started,
            "refresh request should start before the competing write"
        );
        std::fs::write(
            &credentials_path,
            json!({
                "claudeAiOauth": {
                    "accessToken": "claude-access-token",
                    "refreshToken": "claude-refresh-token",
                    "expiresAt": 1_750_003_600_000_i64,
                    "refreshTokenExpiresAt": 1_800_000_000_000_i64
                }
            })
            .to_string(),
        )
        .expect("concurrent credentials");

        let winner = refresh
            .await
            .expect("refresh task")
            .expect("concurrent winner");
        assert_eq!(winner.access_token, "claude-access-token");
        let persisted = std::fs::read_to_string(credentials_path).expect("persisted credentials");
        assert!(persisted.contains("claude-access-token"));
        assert!(!persisted.contains("garyx-access-token"));
    }
}
