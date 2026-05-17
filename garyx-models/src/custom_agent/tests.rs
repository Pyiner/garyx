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
    assert!(profile.default_workspace_dir.is_none());
    assert!(profile.avatar_data_url.is_none());

    let explicit = serde_json::json!({
        "agent_id": "team-member",
        "display_name": "Team Member",
        "provider_type": "claude_code",
        "model": "",
        "modelReasoningEffort": "high",
        "default_workspace_dir": "/tmp/team-member",
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
    assert_eq!(
        profile.default_workspace_dir.as_deref(),
        Some("/tmp/team-member")
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
    assert!(is_builtin_provider_agent_id("gemini"));
    assert!(is_builtin_provider_agent_id("garyx"));
    assert!(!is_builtin_provider_agent_id("plain-claude"));
    assert!(!is_builtin_provider_agent_id("codex-reviewer"));
    assert!(!is_builtin_provider_agent_id("reviewer"));
}

#[test]
fn builtin_provider_profiles_include_garyx_native_agent() {
    let profiles = builtin_provider_agent_profiles();
    let profile = profiles
        .iter()
        .find(|profile| profile.agent_id == "garyx")
        .expect("garyx native profile should exist");
    assert_eq!(profile.display_name, "Garyx");
    assert_eq!(profile.provider_type, ProviderType::GaryxNative);
    assert!(profile.built_in);
}
