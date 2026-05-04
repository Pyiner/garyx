use garyx_models::config::ReplyToMode;

use super::{TELEGRAM_GENERAL_TOPIC_ID, TgMessage};

/// Extract message content from a TgMessage. Returns (text, optional media_type).
/// For text messages, returns (Some(text), None).
/// For media messages, returns (Some(caption_or_description), Some(media_type)).
pub fn extract_message_content(msg: &TgMessage) -> (Option<String>, Option<String>) {
    // Text messages take priority
    if let Some(text) = &msg.text {
        return (Some(text.clone()), None);
    }

    // Photo
    if msg.photo.is_some() {
        let caption = msg.caption.clone().unwrap_or_else(|| "[photo]".to_string());
        return (Some(caption), Some("photo".to_string()));
    }

    // Voice / Audio
    if let Some(voice) = &msg.voice {
        let caption = msg
            .caption
            .clone()
            .unwrap_or_else(|| format!("[voice message, {}s]", voice.duration));
        return (Some(caption), Some("voice".to_string()));
    }
    if let Some(audio) = &msg.audio {
        let title = audio.title.as_deref().unwrap_or("audio");
        let caption = msg
            .caption
            .clone()
            .unwrap_or_else(|| format!("[{}, {}s]", title, audio.duration));
        return (Some(caption), Some("audio".to_string()));
    }

    // Document
    if let Some(doc) = &msg.document {
        let name = doc.file_name.as_deref().unwrap_or("document");
        let caption = msg
            .caption
            .clone()
            .unwrap_or_else(|| format!("[document: {name}]"));
        return (Some(caption), Some("document".to_string()));
    }

    // Video / Animation
    if let Some(video) = &msg.video {
        let caption = msg
            .caption
            .clone()
            .unwrap_or_else(|| format!("[video, {}s]", video.duration));
        return (Some(caption), Some("video".to_string()));
    }
    if msg.animation.is_some() {
        let caption = msg
            .caption
            .clone()
            .unwrap_or_else(|| "[animation/GIF]".to_string());
        return (Some(caption), Some("animation".to_string()));
    }

    // Sticker
    if let Some(sticker) = &msg.sticker {
        let emoji = sticker.emoji.as_deref().unwrap_or("");
        let text = format!("[sticker {emoji}]").trim().to_string();
        return (Some(text), Some("sticker".to_string()));
    }

    (None, None)
}

/// Resolve effective forum thread id:
/// - non-forum chat => None
/// - forum chat without explicit thread => General topic (1)
pub fn resolve_forum_thread_id(msg: &TgMessage) -> Option<i64> {
    if !msg.chat.is_forum.unwrap_or(false) {
        return None;
    }
    msg.message_thread_id.or(Some(TELEGRAM_GENERAL_TOPIC_ID))
}

/// Build group peer key used for thread routing.
///
/// Python parity:
/// - non-forum groups always use chat_id
/// - forum non-general topics use "{chat_id}_t{thread_id}"
/// - forum general topic uses chat_id
pub fn build_group_thread_key(
    chat_id: i64,
    is_forum: bool,
    forum_thread_id: Option<i64>,
) -> String {
    if is_forum
        && let Some(thread_id) = forum_thread_id
        && thread_id != TELEGRAM_GENERAL_TOPIC_ID
    {
        return format!("{chat_id}_t{thread_id}");
    }
    chat_id.to_string()
}

/// Build message_thread_id for outbound sendMessage/sendPhoto calls.
/// Telegram rejects message_thread_id=1 (General topic) on send calls.
pub fn resolve_outbound_thread_id(is_forum: bool, forum_thread_id: Option<i64>) -> Option<i64> {
    if !is_forum {
        return None;
    }
    match forum_thread_id {
        Some(tid) if tid == TELEGRAM_GENERAL_TOPIC_ID => None,
        _ => forum_thread_id,
    }
}

/// Build message_thread_id for typing actions.
/// Typing in forum chats should include message_thread_id, including General topic.
pub fn resolve_typing_thread_id(is_forum: bool, forum_thread_id: Option<i64>) -> Option<i64> {
    if !is_forum {
        return None;
    }
    forum_thread_id
}

/// Resolve reply_to_message_id based on the reply_to_mode setting.
///
/// - `Off`: never reply to a message
/// - `First`: only reply on the first response (is_first_reply = true)
/// - `All`: always reply
pub fn resolve_reply_to(mode: &ReplyToMode, message_id: i64, is_first_reply: bool) -> Option<i64> {
    match mode {
        ReplyToMode::Off => None,
        ReplyToMode::First => {
            if is_first_reply {
                Some(message_id)
            } else {
                None
            }
        }
        ReplyToMode::All => Some(message_id),
    }
}

/// Check if the bot is mentioned in a message (by @username or reply).
pub fn is_mentioned(text: &str, bot_username: &str, bot_id: i64, msg: &TgMessage) -> bool {
    if bot_username.is_empty() {
        return false;
    }

    // Check @username in text (case-insensitive)
    let mention = format!("@{bot_username}");
    if text.to_lowercase().contains(&mention.to_lowercase()) {
        return true;
    }

    // Check entities for mention type
    // Note: Telegram entity offset/length are in UTF-16 code units, so we
    // convert to char indices to safely slice the UTF-8 string.
    if let Some(entities) = &msg.entities {
        let chars: Vec<char> = text.chars().collect();
        for ent in entities {
            if ent.entity_type == "mention" {
                let end = ent.offset + ent.length;
                if end <= chars.len() {
                    let entity_text: String = chars[ent.offset..end].iter().collect();
                    if entity_text.eq_ignore_ascii_case(&mention) {
                        return true;
                    }
                }
            }
        }
    }

    // Check if replying to bot's message
    if let Some(reply) = &msg.reply_to_message
        && let Some(from) = &reply.from
        && from.id == bot_id
    {
        return true;
    }

    false
}
