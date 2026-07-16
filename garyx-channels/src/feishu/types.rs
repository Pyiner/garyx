use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};
use serde_json::Value;
use std::fmt;

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

fn deserialize_string_or_integer<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringOrInteger;

    impl<'de> Visitor<'de> for StringOrInteger {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a string or JSON integer")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value.to_owned())
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value.to_string())
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value.to_string())
        }
    }

    deserializer.deserialize_any(StringOrInteger)
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FeishuMeetingRef {
    #[serde(deserialize_with = "deserialize_string_or_integer")]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_optional_string_or_integer")]
    pub meeting_no: Option<String>,
    #[serde(default)]
    pub topic: String,
}

fn deserialize_optional_string_or_integer<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Value>::deserialize(deserializer)?.map_or(Ok(None), |value| match value {
        Value::String(value) => Ok(Some(value)),
        Value::Number(value) if value.is_i64() || value.is_u64() => Ok(Some(value.to_string())),
        _ => Err(de::Error::custom("expected a string or JSON integer")),
    })
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FeishuMeetingActor {
    #[serde(deserialize_with = "deserialize_actor_id")]
    pub id: String,
}

/// Real `vc.bot.meeting_invited_v1` payloads wrap actor ids in an object
/// (`{"id": {"open_id": "ou_…"}}`), matching the `speaker.id.open_id`
/// shape of activity items, while fixtures derived from mino_server tests
/// use a bare string. Accept string, integer, and object forms
/// (`open_id` > `user_id` > `union_id` > `id`).
fn deserialize_actor_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(value) => Ok(value),
        Value::Number(value) if value.is_i64() || value.is_u64() => Ok(value.to_string()),
        Value::Object(map) => {
            for key in ["open_id", "user_id", "union_id", "id"] {
                match map.get(key) {
                    Some(Value::String(value)) if !value.is_empty() => {
                        return Ok(value.clone());
                    }
                    Some(Value::Number(value)) if value.is_i64() || value.is_u64() => {
                        return Ok(value.to_string());
                    }
                    _ => {}
                }
            }
            Err(de::Error::custom(
                "actor id object carries no open_id/user_id/union_id/id string",
            ))
        }
        _ => Err(de::Error::custom(
            "expected a string, JSON integer, or actor id object",
        )),
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MeetingInvitedEvent {
    pub meeting: FeishuMeetingRef,
    pub bot: FeishuMeetingActor,
    pub inviter: FeishuMeetingActor,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeetingActivityEvent {
    #[serde(default)]
    pub meeting_activity_items: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MeetingEndedEvent {
    pub meeting: FeishuMeetingRef,
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
