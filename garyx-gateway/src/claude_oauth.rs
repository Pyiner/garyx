use serde_json::Value;

pub(crate) const CLAUDE_USER_AGENT: &str = "claude-code/2.0.32";
pub(crate) const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";
pub(crate) const CLAUDE_ANTHROPIC_VERSION: &str = "2023-06-01";

const CLAUDE_OAUTH_ENV_KEYS: &[&str] = &[
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_OAUTH_TOKEN",
];

pub(crate) async fn read_oauth_token() -> Result<String, String> {
    read_oauth_token_and_subscription()
        .await
        .map(|(token, _)| token)
}

pub(crate) async fn read_oauth_token_and_subscription() -> Result<(String, Option<String>), String>
{
    if let Some(token) = read_env_oauth_token() {
        return Ok((token, None));
    }

    let credentials = read_oauth_credentials().await?;
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

async fn read_oauth_credentials() -> Result<Value, String> {
    if let Some(home) = garyx_models::local_paths::home_dir() {
        let path = home.join(".claude").join(".credentials.json");
        if let Ok(contents) = tokio::fs::read_to_string(&path).await {
            return serde_json::from_str(&contents).map_err(|error| {
                format!(
                    "Claude credentials file (~/.claude/.credentials.json) was not valid JSON: {error}"
                )
            });
        }
    }

    #[cfg(target_os = "macos")]
    {
        read_oauth_keychain().await
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Claude credentials not found (~/.claude/.credentials.json missing)".to_string())
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

#[cfg(target_os = "macos")]
async fn read_oauth_keychain() -> Result<Value, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
