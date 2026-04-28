use super::*;

fn standalone_agent(agent_id: &str, provider_type: ProviderType) -> CustomAgentProfile {
    CustomAgentProfile {
        agent_id: agent_id.to_owned(),
        display_name: agent_id.to_owned(),
        provider_type,
        model: String::new(),
        system_prompt: String::new(),
        built_in: false,
        standalone: true,
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        updated_at: "2026-01-01T00:00:00Z".to_owned(),
    }
}

fn team_profile(team_id: &str, leader_agent_id: &str) -> AgentTeamProfile {
    AgentTeamProfile {
        team_id: team_id.to_owned(),
        display_name: team_id.to_owned(),
        leader_agent_id: leader_agent_id.to_owned(),
        member_agent_ids: vec![leader_agent_id.to_owned(), "reviewer".to_owned()],
        workflow_text: "ship it".to_owned(),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        updated_at: "2026-01-01T00:00:00Z".to_owned(),
    }
}

#[test]
fn resolves_standalone_agents() {
    let agents = vec![standalone_agent("claude", ProviderType::ClaudeCode)];

    let resolved = resolve_agent_reference("claude", &agents, &[]).expect("standalone");
    assert_eq!(resolved.bound_agent_id(), "claude");
    assert_eq!(resolved.provider_type(), ProviderType::ClaudeCode);
    assert!(resolved.team().is_none());
}

#[test]
fn resolves_team_by_team_id() {
    let agents = vec![
        standalone_agent("planner", ProviderType::CodexAppServer),
        standalone_agent("reviewer", ProviderType::ClaudeCode),
    ];
    let teams = vec![team_profile("product-ship", "planner")];

    let resolved = resolve_agent_reference("product-ship", &agents, &teams).expect("team by id");
    assert_eq!(resolved.bound_agent_id(), "product-ship");
    assert_eq!(resolved.provider_type(), ProviderType::AgentTeam);
    assert_eq!(
        resolved.team().map(|team| team.team_id.as_str()),
        Some("product-ship")
    );
}

#[test]
fn resolving_leader_agent_id_does_not_resolve_team() {
    let agents = vec![
        standalone_agent("planner", ProviderType::CodexAppServer),
        standalone_agent("reviewer", ProviderType::ClaudeCode),
    ];
    let teams = vec![team_profile("product-ship", "planner")];

    let resolved =
        resolve_agent_reference("planner", &agents, &teams).expect("leader as standalone");
    assert_eq!(resolved.bound_agent_id(), "planner");
    assert_eq!(resolved.provider_type(), ProviderType::CodexAppServer);
    assert!(resolved.team().is_none());
}

#[test]
fn rejects_non_standalone_agents() {
    let mut member = standalone_agent("reviewer", ProviderType::ClaudeCode);
    member.standalone = false;
    let agents = vec![member];

    let error = resolve_agent_reference("reviewer", &agents, &[]).expect_err("reject member");
    assert_eq!(error, "agent_id is not standalone: reviewer");
}

#[test]
fn rejects_team_with_missing_leader() {
    let agents = vec![standalone_agent("reviewer", ProviderType::ClaudeCode)];
    let teams = vec![team_profile("product-ship", "planner")];

    let error =
        resolve_agent_reference("product-ship", &agents, &teams).expect_err("missing leader");
    assert!(
        error.contains("unknown leader_agent_id 'planner'"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_team_with_missing_member() {
    let agents = vec![standalone_agent("planner", ProviderType::CodexAppServer)];
    let teams = vec![team_profile("product-ship", "planner")];

    let error =
        resolve_agent_reference("product-ship", &agents, &teams).expect_err("missing member");
    assert!(
        error.contains("unknown member agent_id 'reviewer'"),
        "unexpected error: {error}"
    );
}

#[test]
fn uniqueness_accepts_disjoint_namespaces() {
    let agents = vec![
        standalone_agent("planner", ProviderType::CodexAppServer),
        standalone_agent("reviewer", ProviderType::ClaudeCode),
    ];
    let teams = vec![team_profile("product-ship", "planner")];

    validate_agent_team_registry_uniqueness(&agents, &teams).expect("ok");
}

#[test]
fn uniqueness_rejects_team_id_colliding_with_agent_id() {
    let agents = vec![
        standalone_agent("planner", ProviderType::CodexAppServer),
        standalone_agent("reviewer", ProviderType::ClaudeCode),
    ];
    let teams = vec![team_profile("planner", "planner")];

    let error =
        validate_agent_team_registry_uniqueness(&agents, &teams).expect_err("should collide");
    assert!(
        error.contains("team_id 'planner' collides"),
        "unexpected error: {error}"
    );
}

#[test]
fn uniqueness_rejects_duplicate_team_ids() {
    let agents = vec![
        standalone_agent("planner", ProviderType::CodexAppServer),
        standalone_agent("reviewer", ProviderType::ClaudeCode),
    ];
    let teams = vec![
        team_profile("product-ship", "planner"),
        team_profile("product-ship", "planner"),
    ];

    let error =
        validate_agent_team_registry_uniqueness(&agents, &teams).expect_err("duplicate team");
    assert!(
        error.contains("duplicate team_id 'product-ship'"),
        "unexpected error: {error}"
    );
}

#[test]
fn uniqueness_rejects_duplicate_agent_ids() {
    let agents = vec![
        standalone_agent("planner", ProviderType::CodexAppServer),
        standalone_agent("planner", ProviderType::ClaudeCode),
    ];
    let teams: Vec<AgentTeamProfile> = vec![];

    let error =
        validate_agent_team_registry_uniqueness(&agents, &teams).expect_err("duplicate agent");
    assert!(
        error.contains("duplicate agent_id 'planner'"),
        "unexpected error: {error}"
    );
}
