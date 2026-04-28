use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ChatType {
    Direct,
    Group,
    Channel,
}

/// Message queueing strategy for a thread. All variants are part of the serde
/// contract for persisted thread/session state; not all are constructed in
/// Rust code today.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    Steer,
    Followup,
    Collect,
    #[serde(rename = "steer-backlog")]
    SteerBacklog,
    Queue,
    Interrupt,
}

/// Policy for dropping messages when a queue is full. All variants are part of
/// the serde contract for persisted thread state; none are constructed in Rust
/// code today.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum QueueDrop {
    Old,
    New,
    Summarize,
}

/// How a group thread is activated. Variants are part of the serde contract
/// for persisted thread/session state; they arrive via JSON deserialization.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GroupActivation {
    Mention,
    Always,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SendPolicy {
    Allow,
    Deny,
}

/// Origin metadata for a persisted product thread.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct ThreadOrigin {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub from_id: String,
    #[serde(default)]
    pub to_id: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// Token usage accounting for a product thread.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct ThreadTokenUsage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub total_cost_usd: f64,
}
