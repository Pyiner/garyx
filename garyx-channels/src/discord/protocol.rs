use super::*;

const DISCORD_GATEWAY_INTENTS: u64 = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscordUser {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub bot: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct DiscordMessageReference {
    #[serde(default)]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscordMessageCreateEvent {
    pub id: String,
    pub channel_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub content: String,
    pub author: DiscordUser,
    #[serde(default)]
    pub mentions: Vec<DiscordUser>,
    #[serde(default)]
    #[allow(dead_code)]
    pub message_reference: Option<DiscordMessageReference>,
    #[serde(default)]
    pub attachments: Vec<DiscordAttachment>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscordAttachment {
    pub id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DiscordGatewayEnvelope {
    pub(super) op: u64,
    #[serde(default)]
    pub(super) t: Option<String>,
    #[serde(default)]
    pub(super) s: Option<u64>,
    #[serde(default)]
    pub(super) d: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DiscordHello {
    pub(super) heartbeat_interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DiscordReady {
    pub(super) session_id: String,
    #[serde(default)]
    pub(super) resume_gateway_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DiscordCurrentUser {
    pub(super) id: String,
    #[serde(default)]
    pub(super) username: Option<String>,
}

pub(super) fn discord_identify_payload(token: &str) -> Value {
    json!({
        "op": 2,
        "d": {
            "token": token,
            "intents": DISCORD_GATEWAY_INTENTS,
            "properties": {
                "os": std::env::consts::OS,
                "browser": "garyx",
                "device": "garyx"
            }
        }
    })
}

pub(super) fn discord_resume_payload(token: &str, session_id: &str, sequence: u64) -> Value {
    json!({
        "op": 6,
        "d": {
            "token": token,
            "session_id": session_id,
            "seq": sequence
        }
    })
}

pub(super) fn discord_gateway_url_with_query(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.contains('?') {
        trimmed.to_owned()
    } else {
        let has_path = trimmed
            .split_once("://")
            .map(|(_, rest)| rest.contains('/'))
            .unwrap_or_else(|| trimmed.contains('/'));
        let separator = if has_path { "?" } else { "/?" };
        format!("{trimmed}{separator}v=10&encoding=json")
    }
}
