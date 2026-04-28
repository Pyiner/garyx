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

    let explicit = serde_json::json!({
        "agent_id": "team-member",
        "display_name": "Team Member",
        "provider_type": "claude_code",
        "model": "",
        "system_prompt": "",
        "built_in": true,
        "standalone": false,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    });
    let profile: CustomAgentProfile =
        serde_json::from_value(explicit).expect("explicit standalone profile");
    assert!(!profile.standalone);
}
