use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AgentTeamProfile {
    pub team_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub leader_agent_id: String,
    #[serde(default)]
    pub member_agent_ids: Vec<String>,
    pub workflow_text: String,
    pub created_at: String,
    pub updated_at: String,
}
