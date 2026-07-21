use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::time::Duration;

pub(crate) const CLAUDE_USER_AGENT: &str = "claude-code/2.0.32";
pub(crate) const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";
pub(crate) const CLAUDE_ANTHROPIC_VERSION: &str = "2023-06-01";

const CLAUDE_OAUTH_ENV_KEYS: &[&str] = &[
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_OAUTH_TOKEN",
];
#[cfg(target_os = "macos")]
const KEYCHAIN_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

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
    let credentials = read_oauth_credentials(config_dir).await?;
    let token = oauth_token_from_credentials(&credentials)?;
    let subscription = oauth_subscription_from_credentials(&credentials);
    Ok((token, subscription))
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
    #[cfg(target_os = "macos")]
    {
        let keychain = read_oauth_keychain(config_dir).await;
        if keychain.is_ok() {
            return keychain;
        }
        return select_oauth_credentials(read_oauth_credentials_file(config_dir).await, keychain);
    }
    #[cfg(not(target_os = "macos"))]
    {
        read_oauth_credentials_file(config_dir).await
    }
}

async fn read_oauth_credentials_file(config_dir: Option<&Path>) -> Result<Value, String> {
    let claude_dir = config_dir
        .map(Path::to_path_buf)
        .or_else(|| garyx_models::local_paths::home_dir().map(|home| home.join(".claude")));
    if let Some(claude_dir) = claude_dir {
        let path = claude_dir.join(".credentials.json");
        if let Ok(contents) = tokio::fs::read_to_string(&path).await {
            return serde_json::from_str(&contents).map_err(|error| {
                format!(
                    "Claude credentials file ({}) was not valid JSON: {error}",
                    path.display()
                )
            });
        }
        return Err(format!(
            "Claude credentials not found ({} missing)",
            path.display()
        ));
    }

    Err("Claude credentials directory is unavailable".to_string())
}

#[cfg(target_os = "macos")]
fn select_oauth_credentials(
    file: Result<Value, String>,
    keychain: Result<Value, String>,
) -> Result<Value, String> {
    keychain.or(file)
}

#[cfg(not(target_os = "macos"))]
fn select_oauth_credentials(
    file: Result<Value, String>,
    _keychain: Result<Value, String>,
) -> Result<Value, String> {
    file
}

#[cfg(target_os = "macos")]
async fn read_oauth_keychain(config_dir: Option<&Path>) -> Result<Value, String> {
    let service = claude_keychain_service(config_dir);
    let mut command = tokio::process::Command::new("security");
    command
        .args(["find-generic-password", "-s"])
        .arg(&service)
        .arg("-a")
        .arg(keychain_account())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::sync::{Mutex, OnceLock};

    use serde_json::json;
    use tempfile::tempdir;

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
}
