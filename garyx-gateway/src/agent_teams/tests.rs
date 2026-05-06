use super::*;

#[tokio::test]
async fn rejects_team_without_leader_in_members() {
    let store = AgentTeamStore::new();
    let error = store
        .upsert_team(UpsertAgentTeamRequest {
            team_id: "team-1".to_owned(),
            display_name: "Team 1".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec!["generator".to_owned()],
            workflow_text: "Do work".to_owned(),
            avatar_data_url: None,
        })
        .await
        .expect_err("expected validation failure");
    assert_eq!(error, "leader_agent_id must appear in member_agent_ids");
}

#[tokio::test]
async fn upsert_preserves_and_clears_avatar_data_url() {
    let store = AgentTeamStore::new();
    let created = store
        .upsert_team(UpsertAgentTeamRequest {
            team_id: "team-1".to_owned(),
            display_name: "Team 1".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec!["planner".to_owned(), "reviewer".to_owned()],
            workflow_text: "Do work".to_owned(),
            avatar_data_url: Some("  data:image/png;base64,dGVzdA==  ".to_owned()),
        })
        .await
        .expect("create team");
    assert_eq!(
        created.avatar_data_url.as_deref(),
        Some("data:image/png;base64,dGVzdA==")
    );

    let updated = store
        .upsert_team(UpsertAgentTeamRequest {
            team_id: "team-1".to_owned(),
            display_name: "Team 1".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec!["planner".to_owned(), "reviewer".to_owned()],
            workflow_text: "Do work".to_owned(),
            avatar_data_url: None,
        })
        .await
        .expect("update team");
    assert_eq!(
        updated.avatar_data_url.as_deref(),
        Some("data:image/png;base64,dGVzdA==")
    );

    let cleared = store
        .upsert_team(UpsertAgentTeamRequest {
            team_id: "team-1".to_owned(),
            display_name: "Team 1".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec!["planner".to_owned(), "reviewer".to_owned()],
            workflow_text: "Do work".to_owned(),
            avatar_data_url: Some("  ".to_owned()),
        })
        .await
        .expect("clear team avatar");
    assert!(cleared.avatar_data_url.is_none());
}
