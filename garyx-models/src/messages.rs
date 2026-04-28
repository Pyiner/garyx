use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// MessageMetadata — channel context attached during routing
// ---------------------------------------------------------------------------

/// Metadata enriched by the router when processing inbound messages.
///
/// Captures the channel context so that downstream components (bridge,
/// providers, callbacks) can access routing information without passing
/// many individual parameters.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct MessageMetadata {
    /// Channel name (e.g. "telegram", "feishu").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,

    /// Bot / app account identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,

    /// Sender identifier (user id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_id: Option<String>,

    /// Whether the message originates from a group chat.
    #[serde(default)]
    pub is_group: bool,

    /// Thread / topic identifier (group thread, forum topic, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,

    /// Resolved thread id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_thread_id: Option<String>,

    /// Arbitrary extra metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Value>,
}
