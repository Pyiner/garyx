use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use super::*;

#[test]
fn custom_agent_profile_defaults_standalone_to_true() {
    let legacy = serde_json::json!({
        "agent_id": "legacy-agent",
        "display_name": "Legacy Agent",
        "provider_type": "claude_code",
        "model": "claude-opus-4-1",
        "system_prompt": "",
        "built_in": false,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    });
    let profile: CustomAgentProfile = serde_json::from_value(legacy).expect("legacy profile");
    assert!(profile.standalone);
    assert_eq!(profile.model, "claude-opus-4-1");
    assert!(profile.model_reasoning_effort.is_empty());
    assert!(profile.model_service_tier.is_empty());
    assert!(profile.provider_env.is_empty());
    assert!(profile.default_workspace_dir.is_none());
    assert!(profile.avatar_data_url.is_none());

    let explicit = serde_json::json!({
        "agent_id": "test-agent",
        "display_name": "Test Agent",
        "provider_type": "claude_code",
        "model": "",
        "modelReasoningEffort": "high",
        "modelServiceTier": "priority",
        "default_workspace_dir": "/tmp/test-agent",
        "avatar_data_url": "data:image/png;base64,dGVzdA==",
        "system_prompt": "",
        "built_in": true,
        "standalone": false,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    });
    let profile: CustomAgentProfile =
        serde_json::from_value(explicit).expect("explicit standalone profile");
    assert!(!profile.standalone);
    assert_eq!(profile.model_reasoning_effort, "high");
    assert_eq!(profile.model_service_tier, "priority");
    assert_eq!(
        profile.default_workspace_dir.as_deref(),
        Some("/tmp/test-agent")
    );
    assert_eq!(
        profile.avatar_data_url.as_deref(),
        Some("data:image/png;base64,dGVzdA==")
    );
}

#[test]
fn builtin_provider_agent_id_detection_is_limited_to_builtin_profiles() {
    assert!(is_builtin_provider_agent_id("claude"));
    assert!(is_builtin_provider_agent_id(" codex "));
    assert!(is_builtin_provider_agent_id("traex"));
    assert!(is_builtin_provider_agent_id("grok"));
    assert!(!is_builtin_provider_agent_id("removed-provider"));
    assert!(!is_builtin_provider_agent_id("plain-claude"));
    assert!(!is_builtin_provider_agent_id("codex-reviewer"));
    assert!(!is_builtin_provider_agent_id("reviewer"));
}

#[test]
fn is_valid_env_key_matches_posix_env_names() {
    assert!(is_valid_env_key("OPENAI_API_KEY"));
    assert!(is_valid_env_key("_PRIVATE"));
    assert!(is_valid_env_key("PATH"));
    assert!(is_valid_env_key("A1_B2"));

    assert!(!is_valid_env_key("")); // empty
    assert!(!is_valid_env_key("1LEADING_DIGIT"));
    assert!(!is_valid_env_key("HAS SPACE"));
    assert!(!is_valid_env_key("HAS=EQUALS"));
    assert!(!is_valid_env_key("HAS-DASH"));
    assert!(!is_valid_env_key("lower.dot"));
}

#[test]
fn builtin_provider_profiles_include_desktop_provider_avatars() {
    let profiles = builtin_provider_agent_profiles();
    for agent_id in ["claude", "codex", "traex", "antigravity", "grok"] {
        let avatar_data_url = profiles
            .iter()
            .find(|profile| profile.agent_id == agent_id)
            .and_then(|profile| profile.avatar_data_url.as_deref())
            .expect("built-in provider avatar");
        let encoded = avatar_data_url
            .strip_prefix("data:image/png;base64,")
            .expect("png avatar data URL");
        let bytes = BASE64.decode(encoded).expect("valid base64 png");
        assert!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "built-in provider avatar should be PNG data"
        );
        assert!(
            bytes.len() >= 600,
            "built-in provider avatar should contain the desktop provider icon artwork"
        );
    }

    let codex_avatar = profiles
        .iter()
        .find(|profile| profile.agent_id == "codex")
        .and_then(|profile| profile.avatar_data_url.as_deref())
        .expect("Codex avatar");
    let trae_avatar = profiles
        .iter()
        .find(|profile| profile.agent_id == "traex")
        .and_then(|profile| profile.avatar_data_url.as_deref())
        .expect("Trae avatar");
    assert_eq!(
        trae_avatar,
        builtin_avatar_data_url(BUILTIN_TRAE_AVATAR_PNG),
        "the Traex Agent must ship the canonical Trae artwork"
    );
    assert_ne!(
        trae_avatar, codex_avatar,
        "Trae must use its own brand artwork instead of the Codex avatar"
    );
}
