use std::env;
use std::path::{Path, PathBuf};

use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::ProviderType;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeAvailability {
    pub(crate) available: bool,
    pub(crate) unavailable_reason: Option<String>,
}

pub(crate) fn detect_provider_runtime(
    config: &GaryxConfig,
    provider_type: &ProviderType,
) -> RuntimeAvailability {
    let candidates = runtime_candidates(config, provider_type);
    let available = candidates.iter().any(|candidate| command_exists(candidate));
    if available {
        RuntimeAvailability {
            available: true,
            unavailable_reason: None,
        }
    } else {
        RuntimeAvailability {
            available: false,
            unavailable_reason: Some(format!(
                "Install `{}` or make it available on PATH.",
                candidates
                    .first()
                    .map(String::as_str)
                    .unwrap_or("provider CLI")
            )),
        }
    }
}

fn runtime_candidates(config: &GaryxConfig, provider_type: &ProviderType) -> Vec<String> {
    match provider_type {
        ProviderType::ClaudeCode => vec!["claude".to_owned()],
        ProviderType::CodexAppServer => {
            let mut candidates = vec!["codex".to_owned()];
            if cfg!(target_os = "macos") {
                candidates.push("/Applications/Codex.app/Contents/Resources/codex".to_owned());
            }
            candidates
        }
        ProviderType::GeminiCli => vec![configured_gemini_bin(config)],
        ProviderType::AgentTeam => Vec::new(),
    }
}

fn configured_gemini_bin(config: &GaryxConfig) -> String {
    for key in ["gemini", "gemini_cli"] {
        if let Some(value) = config.agents.get(key)
            && let Some(bin) = gemini_bin_from_agent_config(value)
        {
            return bin;
        }
    }
    for value in config.agents.values() {
        if let Some(bin) = gemini_bin_from_agent_config(value) {
            return bin;
        }
    }
    "gemini".to_owned()
}

fn gemini_bin_from_agent_config(value: &Value) -> Option<String> {
    let config = serde_json::from_value::<AgentProviderConfig>(value.clone()).ok()?;
    if config.provider_type != "gemini_cli" {
        return None;
    }
    let bin = config.gemini_bin.trim();
    (!bin.is_empty()).then(|| bin.to_owned())
}

fn command_exists(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    let path = Path::new(trimmed);
    if path.is_absolute() || trimmed.contains(std::path::MAIN_SEPARATOR) {
        return is_executable_file(path);
    }
    search_path_dirs()
        .into_iter()
        .any(|dir| is_executable_file(&dir.join(trimmed)))
}

fn search_path_dirs() -> Vec<PathBuf> {
    let mut dirs = env::var_os("PATH")
        .map(|path| env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    for fallback in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/opt/homebrew/sbin",
        "/usr/local/sbin",
    ] {
        let path = PathBuf::from(fallback);
        if !dirs.iter().any(|existing| existing == &path) {
            dirs.push(path);
        }
    }
    dirs
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        return path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_detection_uses_configured_binary_name() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "gemini".to_owned(),
            serde_json::json!({
                "provider_type": "gemini_cli",
                "gemini_bin": "definitely-not-a-real-gemini-test-bin"
            }),
        );

        let result = detect_provider_runtime(&config, &ProviderType::GeminiCli);
        assert!(!result.available);
        assert_eq!(
            result.unavailable_reason.as_deref(),
            Some("Install `definitely-not-a-real-gemini-test-bin` or make it available on PATH.")
        );
    }

    #[test]
    fn codex_detection_mentions_cli_binary() {
        let result =
            detect_provider_runtime(&GaryxConfig::default(), &ProviderType::CodexAppServer);
        if !result.available {
            assert_eq!(
                result.unavailable_reason.as_deref(),
                Some("Install `codex` or make it available on PATH.")
            );
        }
    }
}
