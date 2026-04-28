use super::{ImMention, MentionTarget};

/// Check if the bot was mentioned in the given mentions list.
pub fn is_mentioned(mentions: &[ImMention], bot_open_id: &str) -> bool {
    if mentions.is_empty() {
        return false;
    }
    if bot_open_id.is_empty() {
        // If we don't know the bot's open_id, assume any mention targets the bot
        return true;
    }
    mentions.iter().any(|m| {
        m.id.as_ref()
            .map(|id| id.open_id == bot_open_id)
            .unwrap_or(false)
    })
}

/// Strip @mention tokens from text, normalizing whitespace.
pub fn strip_mention_tokens(text: &str, mentions: &[ImMention]) -> String {
    if text.is_empty() {
        return String::new();
    }

    let mut cleaned = text.to_string();
    for mention in mentions {
        // Python parity: only strip mentions that include a valid open_id.
        let has_open_id = mention
            .id
            .as_ref()
            .map(|id| !id.open_id.trim().is_empty())
            .unwrap_or(false);
        if !has_open_id {
            continue;
        }
        if !mention.key.is_empty() {
            cleaned = cleaned.replace(&mention.key, " ");
        }
        if !mention.name.is_empty() {
            let at_name = format!("@{}", mention.name);
            cleaned = cleaned.replace(&at_name, " ");
        }
    }

    // Normalize whitespace
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn extract_mention_targets(
    mentions: &[ImMention],
    bot_open_id: Option<&str>,
) -> Vec<MentionTarget> {
    let mut targets = Vec::new();
    for mention in mentions {
        let open_id = mention
            .id
            .as_ref()
            .map(|id| id.open_id.as_str())
            .unwrap_or("")
            .trim();
        if open_id.is_empty() {
            continue;
        }
        if bot_open_id.is_some_and(|id| id == open_id) {
            continue;
        }
        let name = if mention.name.trim().is_empty() {
            open_id.to_owned()
        } else {
            mention.name.trim().to_owned()
        };
        targets.push(MentionTarget {
            open_id: open_id.to_owned(),
            name,
        });
    }
    targets
}

pub(super) fn build_mention_prefix(targets: &[MentionTarget]) -> String {
    targets
        .iter()
        .map(|target| format!("<at user_id=\"{}\">{}</at>", target.open_id, target.name))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn is_mention_forward_request(
    mentions: &[ImMention],
    bot_open_id: &str,
    chat_type: &str,
) -> bool {
    if mentions.len() < 2 {
        return false;
    }
    let mut has_bot = false;
    let mut has_other = false;
    for mention in mentions {
        let open_id = mention
            .id
            .as_ref()
            .map(|id| id.open_id.as_str())
            .unwrap_or("")
            .trim();
        if open_id.is_empty() {
            continue;
        }
        if !bot_open_id.is_empty() && open_id == bot_open_id {
            has_bot = true;
        } else {
            has_other = true;
        }
    }
    if chat_type == "p2p" {
        has_other
    } else {
        has_bot && has_other
    }
}
