use super::*;

#[derive(Clone)]
pub(super) struct DiscordInboundRuntime {
    pub(super) http: Client,
    pub(super) account_id: String,
    pub(super) account: DiscordAccount,
    pub(super) router: Arc<Mutex<MessageRouter>>,
    pub(super) bridge: Arc<MultiProviderBridge>,
    pub(super) dispatcher: Arc<dyn ChannelDispatcher>,
}

fn discord_user_mentioned(event: &DiscordMessageCreateEvent, bot_id: &str) -> bool {
    let bot_id = bot_id.trim();
    if bot_id.is_empty() {
        return false;
    }
    event.mentions.iter().any(|mention| mention.id == bot_id)
        || event.content.contains(&format!("<@{bot_id}>"))
        || event.content.contains(&format!("<@!{bot_id}>"))
}

fn strip_discord_bot_mention(content: &str, bot_id: &str) -> String {
    content
        .replace(&format!("<@{bot_id}>"), "")
        .replace(&format!("<@!{bot_id}>"), "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn compact_discord_identifier(value: &str) -> String {
    let trimmed = value.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 12 {
        return trimmed.to_owned();
    }
    let suffix: String = chars[chars.len().saturating_sub(6)..].iter().collect();
    format!("...{suffix}")
}

fn discord_inbound_display_label(event: &DiscordMessageCreateEvent, is_group: bool) -> String {
    if is_group {
        return format!(
            "Discord channel {}",
            compact_discord_identifier(&event.channel_id)
        );
    }
    event
        .author
        .username
        .as_deref()
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("DM {}", compact_discord_identifier(&event.author.id)))
}

pub(crate) fn build_inbound_request(
    account_id: &str,
    account: &DiscordAccount,
    bot_id: &str,
    event: DiscordMessageCreateEvent,
) -> Option<InboundRequest> {
    if event.author.bot || event.author.id == bot_id {
        return None;
    }

    let is_group = event.guild_id.is_some();
    let mentioned = discord_user_mentioned(&event, bot_id);
    if is_group && account.require_mention && !mentioned {
        return None;
    }

    let message = if mentioned {
        strip_discord_bot_mention(&event.content, bot_id)
    } else {
        event.content.trim().to_owned()
    };
    if message.trim().is_empty() && event.attachments.is_empty() {
        return None;
    }

    let mut metadata: HashMap<String, Value> = HashMap::new();
    metadata.insert("channel".to_owned(), Value::String("discord".to_owned()));
    metadata.insert(
        "account_id".to_owned(),
        Value::String(account_id.to_owned()),
    );
    metadata.insert(
        "chat_id".to_owned(),
        Value::String(event.channel_id.clone()),
    );
    metadata.insert("from_id".to_owned(), Value::String(event.author.id.clone()));
    metadata.insert("message_id".to_owned(), Value::String(event.id.clone()));
    metadata.insert(
        NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        Value::String(message.clone()),
    );
    metadata.insert(
        "display_label".to_owned(),
        Value::String(discord_inbound_display_label(&event, is_group)),
    );
    if let Some(username) = event.author.username.as_deref() {
        metadata.insert("from_name".to_owned(), Value::String(username.to_owned()));
    }
    if let Some(guild_id) = event.guild_id.as_deref() {
        metadata.insert("guild_id".to_owned(), Value::String(guild_id.to_owned()));
        metadata.insert("is_group".to_owned(), Value::Bool(true));
        metadata.insert(
            "delivery_thread_id".to_owned(),
            Value::String(event.channel_id.clone()),
        );
    } else {
        metadata.insert("delivery_thread_id".to_owned(), Value::Null);
    }

    Some(InboundRequest {
        channel: "discord".to_owned(),
        account_id: account_id.to_owned(),
        from_id: event.author.id.clone(),
        is_group,
        thread_binding_key: if is_group {
            event.channel_id.clone()
        } else {
            event.author.id.clone()
        },
        message,
        run_id: uuid::Uuid::new_v4().to_string(),
        images: Vec::new(),
        extra_metadata: metadata,
        file_paths: Vec::new(),
    })
}
