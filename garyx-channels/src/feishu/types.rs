use serde::Deserialize;
use serde_json::Value;

/// Top-level event envelope pushed over WebSocket.
#[derive(Debug, Clone, Deserialize)]
pub struct FeishuEventEnvelope {
    #[serde(default)]
    pub schema: String,
    pub header: Option<FeishuEventHeader>,
    pub event: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeishuEventHeader {
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub create_time: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub tenant_key: String,
}

/// Parsed im.message.receive_v1 event body.
#[derive(Debug, Clone, Deserialize)]
pub struct ImMessageReceiveEvent {
    pub message: Option<ImMessage>,
    pub sender: Option<ImSender>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImMessage {
    #[serde(default)]
    pub chat_id: String,
    #[serde(default)]
    pub chat_type: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub message_type: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub mentions: Vec<ImMention>,
    #[serde(default)]
    pub parent_id: String,
    #[serde(default)]
    pub root_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImMention {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub name: String,
    pub id: Option<ImMentionId>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImMentionId {
    #[serde(default)]
    pub open_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImSender {
    pub sender_id: Option<ImSenderId>,
    #[serde(default)]
    pub sender_type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImSenderId {
    #[serde(default)]
    pub open_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct MentionTarget {
    pub(super) open_id: String,
    pub(super) name: String,
}

#[derive(Default)]
pub(super) struct FeishuResponseStreamState {
    pub(super) stream_text: String,
    pub(super) last_assistant_text_for_reply: String,
    pub(super) processing_reaction_id: Option<String>,
    pub(super) processing_reaction_removed: bool,
    pub(super) cot: FeishuCotState,
}
use super::cot::FeishuCotState;
