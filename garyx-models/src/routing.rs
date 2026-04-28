use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DELIVERY_TARGET_TYPE_CHAT_ID: &str = "chat_id";
pub const DELIVERY_TARGET_TYPE_OPEN_ID: &str = "open_id";

pub fn default_delivery_target_type() -> String {
    DELIVERY_TARGET_TYPE_CHAT_ID.to_owned()
}

pub fn normalize_delivery_target_type(value: Option<&str>) -> String {
    match value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .unwrap_or(DELIVERY_TARGET_TYPE_CHAT_ID)
    {
        DELIVERY_TARGET_TYPE_OPEN_ID => DELIVERY_TARGET_TYPE_OPEN_ID.to_owned(),
        _ => DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
    }
}

pub fn infer_delivery_target_type(
    channel: &str,
    explicit_type: Option<&str>,
    explicit_id: Option<&str>,
    chat_id: &str,
    peer_id: &str,
) -> String {
    if explicit_id.is_some_and(|value| !value.trim().is_empty()) {
        return normalize_delivery_target_type(explicit_type);
    }

    let chat_id = chat_id.trim();
    let peer_id = peer_id.trim();
    if channel == "feishu"
        && !chat_id.is_empty()
        && chat_id == peer_id
        && chat_id.starts_with("ou_")
    {
        return DELIVERY_TARGET_TYPE_OPEN_ID.to_owned();
    }

    DELIVERY_TARGET_TYPE_CHAT_ID.to_owned()
}

pub fn infer_delivery_target_id(
    channel: &str,
    explicit_type: Option<&str>,
    explicit_id: Option<&str>,
    chat_id: &str,
    peer_id: &str,
) -> String {
    if let Some(value) = explicit_id
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
    {
        return value.to_owned();
    }

    match infer_delivery_target_type(channel, explicit_type, explicit_id, chat_id, peer_id).as_str()
    {
        DELIVERY_TARGET_TYPE_OPEN_ID => {
            let peer_id = peer_id.trim();
            if !peer_id.is_empty() {
                peer_id.to_owned()
            } else {
                chat_id.trim().to_owned()
            }
        }
        _ => {
            let chat_id = chat_id.trim();
            if !chat_id.is_empty() {
                chat_id.to_owned()
            } else {
                peer_id.trim().to_owned()
            }
        }
    }
}

/// Context for routing and outbound message delivery.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct DeliveryContext {
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub chat_id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default = "default_delivery_target_type")]
    pub delivery_target_type: String,
    #[serde(default)]
    pub delivery_target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}
