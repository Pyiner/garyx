use std::sync::Arc;

use serde::Deserialize;

/// Telegram User object (subset of fields we need).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TgUser {
    pub id: i64,
    #[serde(default)]
    pub is_bot: bool,
    pub first_name: String,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

/// Telegram Chat object (subset).
#[derive(Debug, Clone, Deserialize)]
pub struct TgChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub is_forum: Option<bool>,
}

/// Telegram PhotoSize object.
#[derive(Debug, Clone, Deserialize)]
pub struct TgPhotoSize {
    pub file_id: String,
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
}

/// Telegram Voice object.
#[derive(Debug, Clone, Deserialize)]
pub struct TgVoice {
    pub file_id: String,
    #[serde(default)]
    pub duration: i64,
}

/// Telegram Audio object.
#[derive(Debug, Clone, Deserialize)]
pub struct TgAudio {
    pub file_id: String,
    #[serde(default)]
    pub duration: i64,
    #[serde(default)]
    pub title: Option<String>,
}

/// Telegram Document object.
#[derive(Debug, Clone, Deserialize)]
pub struct TgDocument {
    pub file_id: String,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// Telegram Video object.
#[derive(Debug, Clone, Deserialize)]
pub struct TgVideo {
    pub file_id: String,
    #[serde(default)]
    pub duration: i64,
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
}

/// Telegram Animation (GIF) object.
#[derive(Debug, Clone, Deserialize)]
pub struct TgAnimation {
    pub file_id: String,
    #[serde(default)]
    pub duration: i64,
}

/// Telegram Sticker object.
#[derive(Debug, Clone, Deserialize)]
pub struct TgSticker {
    pub file_id: String,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub is_animated: bool,
    #[serde(default)]
    pub is_video: bool,
}

/// Telegram Message object (subset).
#[derive(Debug, Clone, Deserialize)]
pub struct TgMessage {
    pub message_id: i64,
    pub chat: TgChat,
    #[serde(default)]
    pub from: Option<TgUser>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub date: i64,
    #[serde(default)]
    pub message_thread_id: Option<i64>,
    #[serde(default)]
    pub media_group_id: Option<String>,
    #[serde(default)]
    pub reply_to_message: Option<Box<TgMessage>>,
    #[serde(default)]
    pub entities: Option<Vec<TgMessageEntity>>,
    #[serde(default)]
    pub photo: Option<Vec<TgPhotoSize>>,
    #[serde(default)]
    pub voice: Option<TgVoice>,
    #[serde(default)]
    pub audio: Option<TgAudio>,
    #[serde(default)]
    pub document: Option<TgDocument>,
    #[serde(default)]
    pub video: Option<TgVideo>,
    #[serde(default)]
    pub animation: Option<TgAnimation>,
    #[serde(default)]
    pub sticker: Option<TgSticker>,
}

/// Telegram MessageEntity (for mention detection).
#[derive(Debug, Clone, Deserialize)]
pub struct TgMessageEntity {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub offset: usize,
    pub length: usize,
}

/// A single Update from getUpdates.
#[derive(Debug, Clone, Deserialize)]
pub struct TgUpdate {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<TgMessage>,
}

/// Response wrapper from Telegram Bot API.
#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub struct TgResponse<T> {
    pub ok: bool,
    #[serde(default = "Option::default")]
    pub result: Option<T>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Telegram file metadata from getFile.
#[derive(Debug, Clone, Deserialize)]
pub struct TgFile {
    pub file_id: String,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Callback signature for sending responses back through the channel.
/// Parameters: (account_id, chat_id, text, reply_to_message_id)
pub type ResponseCallback =
    Arc<dyn Fn(String, i64, String, Option<i64>) -> futures_stub::BoxFuture + Send + Sync>;

// We avoid pulling in a futures crate just for BoxFuture. Define a minimal alias.
mod futures_stub {
    use std::future::Future;
    use std::pin::Pin;
    pub type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
}
