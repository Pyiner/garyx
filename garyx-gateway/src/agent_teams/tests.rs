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
        })
        .await
        .expect_err("expected validation failure");
    assert_eq!(error, "leader_agent_id must appear in member_agent_ids");
}
