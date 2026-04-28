use serde::{Deserialize, Serialize};

/// A registered Wiki knowledge base.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WikiEntry {
    pub wiki_id: String,
    pub display_name: String,
    pub path: String,
    pub topic: String,
    #[serde(default = "default_agent_id")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub source_count: u32,
    #[serde(default)]
    pub page_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

fn default_agent_id() -> Option<String> {
    Some("wiki-curator".to_owned())
}
