use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use garyx_bridge::MultiProviderBridge;
use garyx_models::config::{DiscordAccount, DiscordConfig};
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, PromptAttachment, PromptAttachmentKind, ProviderMessage,
    StreamBoundaryKind, StreamEvent, attachments_to_metadata_value,
};
use garyx_router::{InboundRequest, MessageRouter, NATIVE_COMMAND_TEXT_METADATA_KEY, endpoint_key};

use crate::channel_trait::{Channel, ChannelError};
use crate::dispatcher::{
    ChannelDispatcher, DISCORD_MAX_MESSAGE_LENGTH, DiscordSender, split_discord_message,
};
use crate::generated_images::{extract_image_generation_result, write_generated_image_temp};
use crate::plugin_tools::{
    PluginStreamSendDecision, PluginStreamSendPolicy, PluginStreamSendState,
    should_hide_tool_call_display,
};

const DISCORD_GATEWAY_INTENTS: u64 = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);
const DISCORD_RECONNECT_DELAY: Duration = Duration::from_secs(5);
const DISCORD_TOOL_PLACEHOLDER_UPDATE_INTERVAL: Duration = Duration::from_secs(1);
const DISCORD_MAX_IMAGE_SIZE_BYTES: u64 = 5 * 1024 * 1024;
const DISCORD_MAX_FILE_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

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
struct DiscordGatewayEnvelope {
    op: u64,
    #[serde(default)]
    t: Option<String>,
    #[serde(default)]
    s: Option<u64>,
    #[serde(default)]
    d: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordHello {
    heartbeat_interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordReady {
    session_id: String,
    #[serde(default)]
    resume_gateway_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordCurrentUser {
    id: String,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Clone)]
struct DiscordInboundRuntime {
    http: Client,
    account_id: String,
    account: DiscordAccount,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    dispatcher: Arc<dyn ChannelDispatcher>,
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

fn discord_identify_payload(token: &str) -> Value {
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

fn discord_resume_payload(token: &str, session_id: &str, sequence: u64) -> Value {
    json!({
        "op": 6,
        "d": {
            "token": token,
            "session_id": session_id,
            "seq": sequence
        }
    })
}

fn discord_gateway_url_with_query(url: &str) -> String {
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

fn normalize_media_type(value: &str) -> Option<String> {
    let media_type = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    (!media_type.is_empty()).then_some(media_type)
}

fn is_supported_discord_image_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

fn media_type_from_extension(path_like: &str) -> Option<&'static str> {
    let extension = Path::new(path_like)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    match extension.as_deref() {
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("png") => Some("image/png"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

fn filename_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::sanitize_filename)
}

fn media_type_from_url_path(url: &str) -> Option<&'static str> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| media_type_from_extension(parsed.path()))
}

fn discord_attachment_media_type(attachment: &DiscordAttachment) -> String {
    attachment
        .content_type
        .as_deref()
        .and_then(normalize_media_type)
        .or_else(|| media_type_from_extension(&attachment.filename).map(str::to_owned))
        .unwrap_or_else(|| "application/octet-stream".to_owned())
}

fn fallback_attachment_name(attachment: &DiscordAttachment, media_type: &str) -> String {
    let sanitized = crate::sanitize_filename(&attachment.filename);
    if sanitized != "file.bin" || !attachment.filename.trim().is_empty() {
        return sanitized;
    }

    let extension = match media_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    };
    crate::sanitize_filename(&format!("discord-attachment.{extension}"))
}

struct DownloadedDiscordAttachment {
    path: String,
    name: String,
    media_type: String,
    is_image: bool,
}

async fn download_discord_attachment(
    runtime: &DiscordInboundRuntime,
    attachment: &DiscordAttachment,
) -> Option<DownloadedDiscordAttachment> {
    let url = attachment.url.trim();
    if url.is_empty() {
        warn!(
            account_id = %runtime.account_id,
            attachment_id = %attachment.id,
            "skipping Discord attachment without URL"
        );
        return None;
    }

    let declared_media_type = discord_attachment_media_type(attachment);
    let declared_is_image = is_supported_discord_image_media_type(&declared_media_type);
    let declared_max_bytes = if declared_is_image {
        DISCORD_MAX_IMAGE_SIZE_BYTES
    } else {
        DISCORD_MAX_FILE_DOWNLOAD_BYTES
    };
    if let Some(size) = attachment.size
        && size > declared_max_bytes
    {
        warn!(
            account_id = %runtime.account_id,
            attachment_id = %attachment.id,
            size,
            "skipping oversized Discord attachment"
        );
        return None;
    }

    let response = match runtime
        .http
        .get(url)
        .header("Authorization", format!("Bot {}", runtime.account.token))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            warn!(
                account_id = %runtime.account_id,
                attachment_id = %attachment.id,
                error = %error,
                "failed to download Discord attachment"
            );
            return None;
        }
    };

    let status = response.status();
    if !status.is_success() {
        warn!(
            account_id = %runtime.account_id,
            attachment_id = %attachment.id,
            status = %status,
            "Discord attachment download returned non-success status"
        );
        return None;
    }

    let header_media_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_media_type);
    let media_type = header_media_type.unwrap_or(declared_media_type);
    let is_image = is_supported_discord_image_media_type(&media_type);
    let max_bytes = if is_image {
        DISCORD_MAX_IMAGE_SIZE_BYTES
    } else {
        DISCORD_MAX_FILE_DOWNLOAD_BYTES
    };
    if let Some(content_length) = response.content_length()
        && content_length > max_bytes
    {
        warn!(
            account_id = %runtime.account_id,
            attachment_id = %attachment.id,
            size = content_length,
            "skipping oversized Discord attachment payload"
        );
        return None;
    }

    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            warn!(
                account_id = %runtime.account_id,
                attachment_id = %attachment.id,
                error = %error,
                "failed to read Discord attachment bytes"
            );
            return None;
        }
    };
    if bytes.is_empty() || bytes.len() as u64 > max_bytes {
        warn!(
            account_id = %runtime.account_id,
            attachment_id = %attachment.id,
            size = bytes.len(),
            "skipping empty or oversized Discord attachment payload"
        );
        return None;
    }

    let base_dir = std::env::temp_dir().join("garyx-discord").join("inbound");
    if let Err(error) = tokio::fs::create_dir_all(&base_dir).await {
        warn!(
            account_id = %runtime.account_id,
            error = %error,
            "failed to create Discord inbound attachment directory"
        );
        return None;
    }

    let name = filename_from_url(url)
        .filter(|value| value != "file.bin")
        .unwrap_or_else(|| fallback_attachment_name(attachment, &media_type));
    let path = base_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), name));
    if let Err(error) = tokio::fs::write(&path, &bytes).await {
        warn!(
            account_id = %runtime.account_id,
            attachment_id = %attachment.id,
            error = %error,
            "failed to write Discord attachment to disk"
        );
        return None;
    }

    Some(DownloadedDiscordAttachment {
        path: path.to_string_lossy().to_string(),
        name,
        media_type,
        is_image,
    })
}

async fn enrich_inbound_request_with_discord_attachments(
    runtime: &DiscordInboundRuntime,
    event: &DiscordMessageCreateEvent,
    request: &mut InboundRequest,
) {
    if event.attachments.is_empty() {
        return;
    }

    let mut image_attachments = Vec::new();
    for attachment in &event.attachments {
        let Some(downloaded) = download_discord_attachment(runtime, attachment).await else {
            continue;
        };
        if downloaded.is_image {
            image_attachments.push(PromptAttachment {
                kind: PromptAttachmentKind::Image,
                path: downloaded.path,
                name: downloaded.name,
                media_type: downloaded.media_type,
            });
        } else {
            request.file_paths.push(downloaded.path);
        }
    }

    let attachment_count = image_attachments.len() + request.file_paths.len();
    if attachment_count > 0 {
        request.extra_metadata.insert(
            "attachment_count".to_owned(),
            Value::Number((attachment_count as u64).into()),
        );
    }
    if !request.file_paths.is_empty() {
        request.extra_metadata.insert(
            "file_count".to_owned(),
            Value::Number((request.file_paths.len() as u64).into()),
        );
    }
    if !image_attachments.is_empty() {
        request.extra_metadata.insert(
            "image_count".to_owned(),
            Value::Number((image_attachments.len() as u64).into()),
        );
        request.extra_metadata.insert(
            ATTACHMENTS_METADATA_KEY.to_owned(),
            attachments_to_metadata_value(&image_attachments),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownImageRef {
    target: MarkdownImageTarget,
    alt: Option<String>,
    source_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkdownImageTarget {
    Local(PathBuf),
    Remote(String),
}

fn supported_markdown_image_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

fn supported_remote_markdown_image_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(parsed.to_string())
}

fn markdown_image_target(raw_target: &str) -> Option<MarkdownImageTarget> {
    let mut target = raw_target.trim();
    if target.is_empty() {
        return None;
    }

    if let Some(stripped) = target
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
    {
        target = stripped.trim();
    } else if let Some(index) = target.find(char::is_whitespace) {
        target = target[..index].trim();
    }

    let target = target.trim_matches(|value| value == '"' || value == '\'');
    if target.starts_with("data:") {
        return None;
    }
    if let Some(url) = supported_remote_markdown_image_url(target) {
        return Some(MarkdownImageTarget::Remote(url));
    }

    let path = target
        .strip_prefix("file://")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(target));
    path.is_absolute()
        .then_some(path)
        .filter(|path| supported_markdown_image_extension(path))
        .filter(|path| path.is_file())
        .map(MarkdownImageTarget::Local)
}

fn scan_markdown_image_refs(text: &str) -> Vec<MarkdownImageRef> {
    let mut refs = Vec::new();
    let mut offset = 0;

    while let Some(relative_start) = text[offset..].find("![") {
        let start = offset + relative_start;
        let alt_start = start + 2;
        let Some(alt_end_relative) = text[alt_start..].find("](") else {
            offset = alt_start;
            continue;
        };
        let alt_end = alt_start + alt_end_relative;
        let target_start = alt_end + 2;
        let Some(target_end_relative) = text[target_start..].find(')') else {
            offset = target_start;
            continue;
        };
        let target_end = target_start + target_end_relative;
        let target = &text[target_start..target_end];
        if let Some(target) = markdown_image_target(target) {
            refs.push(MarkdownImageRef {
                target,
                alt: {
                    let alt = text[alt_start..alt_end].trim();
                    (!alt.is_empty()).then(|| alt.to_owned())
                },
                source_range: start..target_end + 1,
            });
        }
        offset = target_end + 1;
    }

    refs
}

fn extract_markdown_image_refs(text: &str) -> Vec<MarkdownImageRef> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for image_ref in scan_markdown_image_refs(text) {
        let key = match &image_ref.target {
            MarkdownImageTarget::Local(path) => path.to_string_lossy().to_string(),
            MarkdownImageTarget::Remote(url) => url.clone(),
        };
        if seen.insert(key) {
            refs.push(image_ref);
        }
    }
    refs
}

fn strip_deliverable_markdown_images(text: &str) -> String {
    let image_refs = scan_markdown_image_refs(text);
    if image_refs.is_empty() {
        return text.to_owned();
    }

    let mut stripped = String::with_capacity(text.len());
    let mut cursor = 0;
    for image_ref in image_refs {
        if image_ref.source_range.start > cursor {
            stripped.push_str(&text[cursor..image_ref.source_range.start]);
        }
        cursor = image_ref.source_range.end;
    }
    if cursor < text.len() {
        stripped.push_str(&text[cursor..]);
    }
    stripped.trim().to_owned()
}

fn discord_editable_text(text: &str) -> String {
    let display = strip_deliverable_markdown_images(text);
    if display.len() <= DISCORD_MAX_MESSAGE_LENGTH {
        return display;
    }
    split_discord_message(&display)
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn markdown_image_key(image_ref: &MarkdownImageRef) -> String {
    match &image_ref.target {
        MarkdownImageTarget::Local(path) => path.to_string_lossy().to_string(),
        MarkdownImageTarget::Remote(url) => url.clone(),
    }
}

fn image_extension_for_media_type(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

fn remote_markdown_image_name(url: &str, alt: Option<&str>, media_type: &str) -> String {
    if let Some(name) = filename_from_url(url)
        && name != "file.bin"
        && supported_markdown_image_extension(Path::new(&name))
    {
        return name;
    }

    let extension = image_extension_for_media_type(media_type);
    let raw_name = alt
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("{value}.{extension}"))
        .unwrap_or_else(|| format!("discord-markdown-image.{extension}"));
    crate::sanitize_filename(&raw_name)
}

async fn download_remote_markdown_image(
    http: &Client,
    url: &str,
    alt: Option<&str>,
) -> Option<PathBuf> {
    let response = match http.get(url).send().await {
        Ok(response) => response,
        Err(error) => {
            warn!(
                error = %error,
                "failed to download Discord remote markdown image"
            );
            return None;
        }
    };
    let status = response.status();
    if !status.is_success() {
        warn!(
            status = %status,
            "Discord remote markdown image returned non-success status"
        );
        return None;
    }

    let media_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_media_type)
        .or_else(|| media_type_from_url_path(url).map(str::to_owned));
    let Some(media_type) = media_type else {
        warn!("skipping Discord remote markdown image with unknown media type");
        return None;
    };
    if !is_supported_discord_image_media_type(&media_type) {
        warn!(
            media_type = %media_type,
            "skipping unsupported Discord remote markdown image"
        );
        return None;
    }
    if let Some(content_length) = response.content_length()
        && content_length > DISCORD_MAX_IMAGE_SIZE_BYTES
    {
        warn!(
            size = content_length,
            "skipping oversized Discord remote markdown image"
        );
        return None;
    }

    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            warn!(
                error = %error,
                "failed to read Discord remote markdown image bytes"
            );
            return None;
        }
    };
    if bytes.is_empty() || bytes.len() as u64 > DISCORD_MAX_IMAGE_SIZE_BYTES {
        warn!(
            size = bytes.len(),
            "skipping empty or oversized Discord remote markdown image"
        );
        return None;
    }

    let base_dir = std::env::temp_dir()
        .join("garyx-discord")
        .join("outbound-markdown");
    if let Err(error) = tokio::fs::create_dir_all(&base_dir).await {
        warn!(
            error = %error,
            "failed to create Discord outbound markdown image directory"
        );
        return None;
    }
    let name = remote_markdown_image_name(url, alt, &media_type);
    let path = base_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), name));
    if let Err(error) = tokio::fs::write(&path, &bytes).await {
        warn!(
            error = %error,
            "failed to write Discord remote markdown image to disk"
        );
        return None;
    }
    Some(path)
}

pub(crate) struct DiscordStreamingCallbackConfig {
    pub sender: DiscordSender,
    pub chat_id: String,
    pub reply_to_message_id: Option<String>,
}

struct DiscordStreamState {
    stream_text: PluginStreamSendState,
    markdown_image_scan_text: String,
    sent_markdown_image_keys: HashSet<String>,
    message_id: Option<String>,
    last_rendered_text: String,
    finalized: bool,
}

fn discord_stream_send_policy() -> PluginStreamSendPolicy {
    PluginStreamSendPolicy::buffered_until_tool_or_done()
        .with_tool_min_flush_interval(DISCORD_TOOL_PLACEHOLDER_UPDATE_INTERVAL)
}

impl Default for DiscordStreamState {
    fn default() -> Self {
        Self {
            stream_text: PluginStreamSendState::new(discord_stream_send_policy()),
            markdown_image_scan_text: String::new(),
            sent_markdown_image_keys: HashSet::new(),
            message_id: None,
            last_rendered_text: String::new(),
            finalized: false,
        }
    }
}

struct DiscordStreamingCallbackShared {
    cfg: DiscordStreamingCallbackConfig,
    state: Mutex<DiscordStreamState>,
}

impl DiscordStreamingCallbackShared {
    fn reset_for_fresh_message(state: &mut DiscordStreamState) {
        state.stream_text = PluginStreamSendState::new(discord_stream_send_policy());
        state.markdown_image_scan_text.clear();
        state.sent_markdown_image_keys.clear();
        state.message_id = None;
        state.last_rendered_text.clear();
        state.finalized = false;
    }

    async fn send_initial_text(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        display_text: &str,
    ) -> bool {
        if display_text.trim().is_empty() {
            return true;
        }
        match self
            .cfg
            .sender
            .send_text(
                &self.cfg.chat_id,
                display_text,
                self.cfg.reply_to_message_id.as_deref(),
            )
            .await
        {
            Ok(message_ids) => {
                state.message_id = message_ids.last().cloned();
                state.last_rendered_text = display_text.to_owned();
                true
            }
            Err(error) => {
                warn!(
                    account_id = %self.cfg.sender.account_id,
                    chat_id = %self.cfg.chat_id,
                    thread_id,
                    error = %error,
                    "failed to send Discord streamed response"
                );
                false
            }
        }
    }

    async fn edit_existing_text(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        display_text: &str,
    ) -> bool {
        let Some(message_id) = state.message_id.as_deref() else {
            return self.send_initial_text(thread_id, state, display_text).await;
        };
        if display_text.trim().is_empty() || display_text.trim() == state.last_rendered_text.trim()
        {
            return true;
        }
        match self
            .cfg
            .sender
            .edit_text(&self.cfg.chat_id, message_id, display_text)
            .await
        {
            Ok(edited_id) => {
                state.last_rendered_text = display_text.to_owned();
                let _ = edited_id;
                true
            }
            Err(error) => {
                warn!(
                    account_id = %self.cfg.sender.account_id,
                    chat_id = %self.cfg.chat_id,
                    thread_id,
                    error = %error,
                    "failed to edit Discord streamed response"
                );
                state.message_id = None;
                state.last_rendered_text.clear();
                false
            }
        }
    }

    async fn apply_stream_send_decision(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        decision: PluginStreamSendDecision,
        mark_tool_flush: bool,
    ) {
        match decision {
            PluginStreamSendDecision::Wait | PluginStreamSendDecision::ScheduleFlush { .. } => {}
            PluginStreamSendDecision::FlushNow { content_text } => {
                let display_text = discord_editable_text(&content_text);
                if state.message_id.is_none() {
                    let _ = self
                        .send_initial_text(thread_id, state, &display_text)
                        .await;
                } else {
                    let _ = self
                        .edit_existing_text(thread_id, state, &display_text)
                        .await;
                }
                if mark_tool_flush {
                    state.stream_text.mark_tool_flushed(Instant::now());
                } else {
                    state.stream_text.mark_flushed(Instant::now());
                }
            }
        }
    }

    async fn flush_pending_tool_placeholder(self: Arc<Self>, thread_id: String) {
        let mut state = self.state.lock().await;
        if state.finalized {
            return;
        }
        let decision = state.stream_text.scheduled_tool_flush();
        self.apply_stream_send_decision(&thread_id, &mut state, decision, true)
            .await;
    }

    async fn delete_runtime_only_message(&self, thread_id: &str, state: &mut DiscordStreamState) {
        let Some(message_id) = state.message_id.clone() else {
            return;
        };

        match self
            .cfg
            .sender
            .delete_text(&self.cfg.chat_id, &message_id)
            .await
        {
            Ok(()) => {
                state.message_id = None;
                state.last_rendered_text.clear();
            }
            Err(error) => {
                warn!(
                    account_id = %self.cfg.sender.account_id,
                    chat_id = %self.cfg.chat_id,
                    thread_id,
                    error = %error,
                    "failed to delete Discord runtime-only stream message"
                );
                state.message_id = None;
                state.last_rendered_text.clear();
            }
        }
    }

    async fn clear_tool_placeholder(&self, thread_id: &str, state: &mut DiscordStreamState) {
        if !state.stream_text.is_tool_placeholder_active() {
            return;
        }

        state.stream_text.clear_tool_placeholder();
        let display_text = discord_editable_text(state.stream_text.accumulated_text());
        let Some(message_id) = state.message_id.clone() else {
            state.last_rendered_text.clear();
            return;
        };

        if display_text.trim().is_empty() {
            match self
                .cfg
                .sender
                .delete_text(&self.cfg.chat_id, &message_id)
                .await
            {
                Ok(()) => {
                    state.message_id = None;
                    state.last_rendered_text.clear();
                }
                Err(error) => {
                    warn!(
                        account_id = %self.cfg.sender.account_id,
                        chat_id = %self.cfg.chat_id,
                        thread_id,
                        error = %error,
                        "failed to delete Discord tool placeholder"
                    );
                }
            }
            return;
        }

        self.edit_existing_text(thread_id, state, &display_text)
            .await;
    }

    async fn process_boundary(&self, thread_id: &str, state: &mut DiscordStreamState) {
        let boundary_content_text = state.stream_text.accumulated_text().trim().to_owned();
        let boundary_text = strip_deliverable_markdown_images(&boundary_content_text);

        state.stream_text.clear_tool_placeholder();

        if boundary_content_text.is_empty() {
            self.delete_runtime_only_message(thread_id, state).await;
            Self::reset_for_fresh_message(state);
            return;
        }

        if boundary_text.trim().is_empty() {
            self.delete_runtime_only_message(thread_id, state).await;
            self.send_markdown_images_from_state(thread_id, state).await;
            Self::reset_for_fresh_message(state);
            return;
        }

        self.finalize_text(thread_id, state).await;
        self.send_markdown_images_from_state(thread_id, state).await;
        Self::reset_for_fresh_message(state);
    }

    async fn finalize_text(&self, thread_id: &str, state: &mut DiscordStreamState) {
        let display_text = strip_deliverable_markdown_images(state.stream_text.accumulated_text());
        if display_text.trim().is_empty() {
            return;
        }
        let chunks = split_discord_message(&display_text);
        let first_chunk = chunks.first().cloned().unwrap_or_default();
        if state.message_id.is_some() {
            if !self
                .edit_existing_text(thread_id, state, &first_chunk)
                .await
            {
                let _ = self
                    .send_initial_text(thread_id, state, &display_text)
                    .await;
                return;
            }
            if chunks.len() > 1 {
                match self
                    .cfg
                    .sender
                    .send_text(&self.cfg.chat_id, &chunks[1..].join(""), None)
                    .await
                {
                    Ok(_) => {}
                    Err(error) => {
                        warn!(
                            account_id = %self.cfg.sender.account_id,
                            chat_id = %self.cfg.chat_id,
                            thread_id,
                            error = %error,
                            "failed to send Discord overflow stream chunks"
                        );
                    }
                }
            }
        } else {
            let _ = self
                .send_initial_text(thread_id, state, &display_text)
                .await;
        }
    }

    async fn send_markdown_images_from_state(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
    ) {
        let image_refs = extract_markdown_image_refs(&state.markdown_image_scan_text);
        if image_refs.is_empty() {
            return;
        }

        for image_ref in image_refs {
            let key = markdown_image_key(&image_ref);
            if !state.sent_markdown_image_keys.insert(key) {
                continue;
            }
            let (path, remove_after_send) = match &image_ref.target {
                MarkdownImageTarget::Local(path) => (path.clone(), false),
                MarkdownImageTarget::Remote(url) => {
                    let Some(path) = download_remote_markdown_image(
                        &self.cfg.sender.http,
                        url,
                        image_ref.alt.as_deref(),
                    )
                    .await
                    else {
                        continue;
                    };
                    (path, true)
                }
            };
            match self
                .cfg
                .sender
                .send_file(
                    &self.cfg.chat_id,
                    &path,
                    image_ref.alt.as_deref(),
                    self.cfg.reply_to_message_id.as_deref(),
                )
                .await
            {
                Ok(_) => {}
                Err(error) => {
                    warn!(
                        account_id = %self.cfg.sender.account_id,
                        chat_id = %self.cfg.chat_id,
                        thread_id,
                        path = %path.display(),
                        error = %error,
                        "failed to send Discord markdown image"
                    );
                }
            }
            if remove_after_send {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
    }

    async fn process_generated_image_result(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        message: ProviderMessage,
    ) {
        let Some(image) = extract_image_generation_result(&message) else {
            return;
        };
        if state.stream_text.is_tool_placeholder_active() {
            self.clear_tool_placeholder(thread_id, state).await;
        }
        let image_path = match write_generated_image_temp("discord", &image).await {
            Ok(path) => path,
            Err(error) => {
                warn!(
                    account_id = %self.cfg.sender.account_id,
                    error = %error,
                    "failed to write Discord generated image temp file"
                );
                return;
            }
        };
        let send_result = self
            .cfg
            .sender
            .send_file(
                &self.cfg.chat_id,
                &image_path,
                None,
                self.cfg.reply_to_message_id.as_deref(),
            )
            .await;
        let _ = tokio::fs::remove_file(&image_path).await;
        match send_result {
            Ok(_) => {}
            Err(error) => {
                warn!(
                    account_id = %self.cfg.sender.account_id,
                    chat_id = %self.cfg.chat_id,
                    thread_id,
                    error = %error,
                    "failed to send Discord generated image"
                );
            }
        }
    }

    async fn process_event(self: &Arc<Self>, event: StreamEvent, thread_id: &str) {
        let mut state = self.state.lock().await;
        match event {
            StreamEvent::SessionBound { .. } | StreamEvent::ThreadTitleUpdated { .. } => {}
            StreamEvent::ToolUse { message } => {
                if should_hide_tool_call_display(&message) {
                    return;
                }
                let decision = state.stream_text.on_tool_call(&message, Instant::now());
                state.finalized = false;
                if let PluginStreamSendDecision::ScheduleFlush { after } = decision {
                    let shared = self.clone();
                    let thread_id = thread_id.to_owned();
                    tokio::spawn(async move {
                        tokio::time::sleep(after).await;
                        shared.flush_pending_tool_placeholder(thread_id).await;
                    });
                } else {
                    self.apply_stream_send_decision(thread_id, &mut state, decision, true)
                        .await;
                }
            }
            StreamEvent::ToolResult { message } => {
                self.process_generated_image_result(thread_id, &mut state, message)
                    .await;
            }
            StreamEvent::Boundary { kind, .. } => match kind {
                StreamBoundaryKind::UserAck => {
                    self.process_boundary(thread_id, &mut state).await;
                }
                StreamBoundaryKind::AssistantSegment => {
                    crate::streaming_core::apply_stream_boundary_text(
                        &mut state.markdown_image_scan_text,
                        StreamBoundaryKind::AssistantSegment,
                    );
                    state
                        .stream_text
                        .apply_boundary(StreamBoundaryKind::AssistantSegment);
                }
            },
            StreamEvent::Delta { text } => {
                if text.is_empty() {
                    return;
                }
                state.markdown_image_scan_text.push_str(&text);
                let decision = state.stream_text.on_delta(&text, Instant::now());
                state.finalized = false;
                self.apply_stream_send_decision(thread_id, &mut state, decision, false)
                    .await;
            }
            StreamEvent::Done => {
                if state.finalized {
                    return;
                }
                if state.stream_text.is_tool_placeholder_active() {
                    self.clear_tool_placeholder(thread_id, &mut state).await;
                }
                state.finalized = true;
                self.finalize_text(thread_id, &mut state).await;
                self.send_markdown_images_from_state(thread_id, &mut state)
                    .await;
                Self::reset_for_fresh_message(&mut state);
            }
        }
    }
}

pub(crate) fn build_discord_response_callback(
    cfg: DiscordStreamingCallbackConfig,
) -> (
    Arc<dyn Fn(StreamEvent) + Send + Sync>,
    watch::Sender<String>,
) {
    let shared = Arc::new(DiscordStreamingCallbackShared {
        cfg,
        state: Mutex::new(DiscordStreamState::default()),
    });
    let (thread_id_tx, thread_id_rx) = watch::channel(String::new());
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<StreamEvent>();

    let shared_for_worker = shared.clone();
    tokio::spawn(async move {
        let mut thread_id_rx = thread_id_rx;
        while let Some(event) = event_rx.recv().await {
            let mut thread_id = thread_id_rx.borrow().clone();
            if thread_id.is_empty() {
                while thread_id.is_empty() {
                    if thread_id_rx.changed().await.is_err() {
                        break;
                    }
                    thread_id = thread_id_rx.borrow().clone();
                }
            }
            if thread_id.is_empty() {
                continue;
            }
            shared_for_worker.process_event(event, &thread_id).await;
        }
    });

    let response_callback: Arc<dyn Fn(StreamEvent) + Send + Sync> =
        Arc::new(move |event: StreamEvent| {
            let _ = event_tx.send(event);
        });

    (response_callback, thread_id_tx)
}

pub struct DiscordChannel {
    config: DiscordConfig,
    http: Client,
    running: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    dispatcher: Arc<dyn ChannelDispatcher>,
}

impl DiscordChannel {
    pub fn new(
        config: DiscordConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
        dispatcher: Arc<dyn ChannelDispatcher>,
    ) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|err| {
                warn!(
                    error = %err,
                    "failed to build Discord reqwest client; falling back to default client"
                );
                Client::new()
            });
        Self {
            config,
            http,
            running: Arc::new(AtomicBool::new(false)),
            tasks: Vec::new(),
            router,
            bridge,
            dispatcher,
        }
    }

    async fn verify_bot(
        http: &Client,
        account_id: &str,
        account: &DiscordAccount,
    ) -> Result<DiscordCurrentUser, ChannelError> {
        let token = account.token.trim();
        if token.is_empty() {
            return Err(ChannelError::Config(format!(
                "Discord account '{account_id}' token is required"
            )));
        }
        let response = http
            .get(format!(
                "{}/users/@me",
                account.api_base.trim_end_matches('/')
            ))
            .header("Authorization", format!("Bot {token}"))
            .send()
            .await
            .map_err(|error| {
                ChannelError::Connection(format!("Discord users/@me request failed: {error}"))
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(ChannelError::Connection(format!(
                "Discord users/@me HTTP {status}"
            )));
        }
        response
            .json::<DiscordCurrentUser>()
            .await
            .map_err(|error| {
                ChannelError::Connection(format!("Discord users/@me parse failed: {error}"))
            })
    }

    fn inbound_runtime(&self, account_id: &str, account: &DiscordAccount) -> DiscordInboundRuntime {
        DiscordInboundRuntime {
            http: self.http.clone(),
            account_id: account_id.to_owned(),
            account: account.clone(),
            router: self.router.clone(),
            bridge: self.bridge.clone(),
            dispatcher: self.dispatcher.clone(),
        }
    }

    async fn gateway_loop(
        runtime: DiscordInboundRuntime,
        bot: DiscordCurrentUser,
        running: Arc<AtomicBool>,
    ) {
        let mut last_sequence: Option<u64> = None;
        let mut session_id: Option<String> = None;
        let mut resume_gateway_url: Option<String> = None;

        while running.load(Ordering::Relaxed) {
            let gateway_url = resume_gateway_url
                .as_deref()
                .map(discord_gateway_url_with_query)
                .unwrap_or_else(|| runtime.account.gateway_url.clone());
            let connection = connect_async(&gateway_url).await;
            let (socket, _) = match connection {
                Ok(connection) => connection,
                Err(error) => {
                    warn!(
                        account_id = %runtime.account_id,
                        error = %error,
                        "Discord Gateway connect failed; retrying"
                    );
                    tokio::time::sleep(DISCORD_RECONNECT_DELAY).await;
                    continue;
                }
            };

            info!(
                account_id = %runtime.account_id,
                bot_id = %bot.id,
                "Discord Gateway connected"
            );
            let (mut write, mut read) = socket.split();
            let mut heartbeat = tokio::time::interval(Duration::from_secs(45));
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut identified = false;

            loop {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                tokio::select! {
                    _ = heartbeat.tick(), if identified => {
                        let heartbeat_payload = json!({
                            "op": 1,
                            "d": last_sequence
                        });
                        if let Err(error) = write.send(Message::Text(heartbeat_payload.to_string().into())).await {
                            warn!(account_id = %runtime.account_id, error = %error, "Discord heartbeat failed");
                            break;
                        }
                    }
                    maybe_message = read.next() => {
                        let Some(message) = maybe_message else {
                            break;
                        };
                        let message = match message {
                            Ok(message) => message,
                            Err(error) => {
                                warn!(account_id = %runtime.account_id, error = %error, "Discord Gateway read failed");
                                break;
                            }
                        };
                        let text = match message {
                            Message::Text(text) => text.to_string(),
                            Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                            Message::Close(_) => break,
                            _ => continue,
                        };
                        let envelope = match serde_json::from_str::<DiscordGatewayEnvelope>(&text) {
                            Ok(envelope) => envelope,
                            Err(error) => {
                                warn!(account_id = %runtime.account_id, error = %error, "Discord Gateway payload parse failed");
                                continue;
                            }
                        };
                        if let Some(sequence) = envelope.s {
                            last_sequence = Some(sequence);
                        }
                        match envelope.op {
                            10 => {
                                let hello = serde_json::from_value::<DiscordHello>(envelope.d)
                                    .unwrap_or(DiscordHello { heartbeat_interval: 45_000 });
                                heartbeat = tokio::time::interval(Duration::from_millis(
                                    hello.heartbeat_interval.max(1),
                                ));
                                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                                let (handshake, handshake_name) = if let (Some(session_id), Some(sequence)) =
                                    (session_id.as_deref(), last_sequence)
                                {
                                    (
                                        discord_resume_payload(&runtime.account.token, session_id, sequence),
                                        "resume",
                                    )
                                } else {
                                    (discord_identify_payload(&runtime.account.token), "identify")
                                };
                                if let Err(error) = write.send(Message::Text(handshake.to_string().into())).await {
                                    warn!(account_id = %runtime.account_id, error = %error, handshake = handshake_name, "Discord Gateway handshake failed");
                                    break;
                                }
                                if handshake_name == "resume" {
                                    info!(
                                        account_id = %runtime.account_id,
                                        sequence = last_sequence.unwrap_or_default(),
                                        "Discord Gateway resume requested"
                                    );
                                }
                                identified = true;
                            }
                            0 if envelope.t.as_deref() == Some("READY") => {
                                match serde_json::from_value::<DiscordReady>(envelope.d) {
                                    Ok(ready) => {
                                        session_id = Some(ready.session_id);
                                        resume_gateway_url = ready
                                            .resume_gateway_url
                                            .map(|url| discord_gateway_url_with_query(&url));
                                    }
                                    Err(error) => {
                                        warn!(account_id = %runtime.account_id, error = %error, "Discord READY parse failed");
                                    }
                                }
                            }
                            0 if envelope.t.as_deref() == Some("MESSAGE_CREATE") => {
                                match serde_json::from_value::<DiscordMessageCreateEvent>(envelope.d) {
                                    Ok(event) => {
                                        Self::handle_message_create(&runtime, &bot, event).await;
                                    }
                                    Err(error) => {
                                        warn!(account_id = %runtime.account_id, error = %error, "Discord MESSAGE_CREATE parse failed");
                                    }
                                }
                            }
                            1 => {
                                let heartbeat_payload = json!({
                                    "op": 1,
                                    "d": last_sequence
                                });
                                if let Err(error) = write.send(Message::Text(heartbeat_payload.to_string().into())).await {
                                    warn!(account_id = %runtime.account_id, error = %error, "Discord heartbeat request response failed");
                                    break;
                                }
                            }
                            7 => break,
                            9 => {
                                let resumable = envelope.d.as_bool().unwrap_or(false);
                                if !resumable {
                                    session_id = None;
                                    last_sequence = None;
                                    resume_gateway_url = None;
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if running.load(Ordering::Relaxed) {
                tokio::time::sleep(DISCORD_RECONNECT_DELAY).await;
            }
        }
        info!(account_id = %runtime.account_id, "Discord Gateway loop stopped");
    }

    async fn handle_message_create(
        runtime: &DiscordInboundRuntime,
        bot: &DiscordCurrentUser,
        event: DiscordMessageCreateEvent,
    ) {
        let Some(mut request) = build_inbound_request(
            &runtime.account_id,
            &runtime.account,
            &bot.id,
            event.clone(),
        ) else {
            return;
        };
        enrich_inbound_request_with_discord_attachments(runtime, &event, &mut request).await;
        let reply_target = event.channel_id.clone();
        let reply_to = event.id.clone();
        let thread_binding_key = request.thread_binding_key.clone();
        let sender = DiscordSender {
            account_id: runtime.account_id.clone(),
            token: runtime.account.token.clone(),
            http: runtime.http.clone(),
            api_base: runtime.account.api_base.clone(),
            is_running: true,
        };
        let (response_callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: sender.clone(),
                chat_id: reply_target.clone(),
                reply_to_message_id: Some(reply_to.clone()),
            });

        let origin_endpoint_identity =
            endpoint_key("discord", &runtime.account_id, &thread_binding_key);
        let deferred_fanout = crate::bound_fanout::DeferredBoundStreamFanout::new(
            runtime.router.clone(),
            runtime.dispatcher.clone(),
            request.run_id.clone(),
            origin_endpoint_identity,
        );
        let fanout_consumer = deferred_fanout.consumer(response_callback);

        // Read this run's stream from the durable committed transcript:
        // subscribe before dispatch and let the replay adapter drive the
        // Discord sender. Bound non-origin endpoints attach after
        // route_and_dispatch resolves the canonical thread id.
        let replay_subscription = match crate::committed_replay::committed_callback(
            &runtime.bridge,
            &request.run_id,
            fanout_consumer,
        )
        .await
        {
            Ok(subscription) => subscription,
            Err(error) => {
                tracing::error!(run_id = %request.run_id, error = %error, "committed replay bus missing for Discord dispatch");
                return;
            }
        };

        let thread_store = {
            let router = runtime.router.lock().await;
            router.thread_store()
        };
        let dispatch_delegate = crate::bound_fanout::DeferredFanoutAgentDispatcher::new(
            runtime.bridge.as_ref(),
            deferred_fanout.clone(),
            thread_store,
        );
        let dispatch_callback = replay_subscription.callback();

        let dispatch_result = {
            let mut router = runtime.router.lock().await;
            router
                .route_and_dispatch(request, &dispatch_delegate, dispatch_callback)
                .await
        };
        match dispatch_result {
            Ok(result) => {
                deferred_fanout.attach_thread(&result.thread_id).await;
                let _ = thread_id_tx.send(result.thread_id.clone());
                let local_reply = result.local_reply;
                if local_reply.is_some() {
                    replay_subscription.abort();
                } else {
                    replay_subscription.detach();
                }
                if let Some(local_reply) = local_reply {
                    match sender
                        .send_text(&reply_target, &local_reply, Some(&reply_to))
                        .await
                    {
                        Ok(_) => {}
                        Err(error) => {
                            warn!(
                                account_id = %runtime.account_id,
                                error = %error,
                                "failed to send local Discord reply"
                            );
                        }
                    }
                }
            }
            Err(error) => {
                replay_subscription.abort();
                warn!(
                    account_id = %runtime.account_id,
                    error = %error,
                    "failed to route Discord inbound message"
                );
                if let Err(send_error) = sender
                    .send_text(&reply_target, &format!("Error: {error}"), Some(&reply_to))
                    .await
                {
                    warn!(
                        account_id = %runtime.account_id,
                        error = %send_error,
                        "failed to send Discord routing error reply"
                    );
                }
            }
        }
    }
}

#[async_trait]
impl Channel for DiscordChannel {
    fn name(&self) -> &str {
        "discord"
    }

    async fn start(&mut self) -> Result<(), ChannelError> {
        if self.running.load(Ordering::Relaxed) {
            return Err(ChannelError::Internal("already running".into()));
        }
        self.running.store(true, Ordering::Relaxed);
        for (account_id, account) in &self.config.accounts {
            if !account.enabled {
                info!(account_id, "Discord account disabled, skipping");
                continue;
            }
            let bot = Self::verify_bot(&self.http, account_id, account).await?;
            info!(
                account_id,
                bot_id = %bot.id,
                bot_username = bot.username.as_deref().unwrap_or(""),
                "verified Discord bot"
            );
            let runtime = self.inbound_runtime(account_id, account);
            let running = self.running.clone();
            self.tasks
                .push(tokio::spawn(Self::gateway_loop(runtime, bot, running)));
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ChannelError> {
        self.running.store(false, Ordering::Relaxed);
        for task in self.tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use garyx_models::AgentBindingError;
    use garyx_models::config::DiscordAccount;
    use garyx_models::provider::StreamEvent;
    use garyx_router::{ThreadCreationError, ThreadCreator, ThreadEnsureOptions, ThreadStore};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct NoEnabledThreadCreator;

    #[async_trait]
    impl ThreadCreator for NoEnabledThreadCreator {
        async fn create_thread(
            &self,
            _thread_store: Arc<dyn ThreadStore>,
            _options: ThreadEnsureOptions,
        ) -> Result<(String, Value), ThreadCreationError> {
            Err(AgentBindingError::NoEnabledAgent.into())
        }
    }

    fn account(require_mention: bool) -> DiscordAccount {
        DiscordAccount {
            token: "discord-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            owner_target: None,
            require_mention,
            api_base: "https://discord.com/api/v10".to_owned(),
            gateway_url: "wss://gateway.discord.gg/?v=10&encoding=json".to_owned(),
        }
    }

    #[tokio::test]
    async fn inbound_no_enabled_agent_sends_visible_error_reply() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-error-reply"
            })))
            .mount(&server)
            .await;

        let router = crate::test_helpers::make_router();
        router
            .lock()
            .await
            .set_thread_creator(Arc::new(NoEnabledThreadCreator));
        let bridge = crate::test_helpers::make_bridge_with(Arc::new(
            crate::test_helpers::ConfigurableTestProvider::echo(),
        ))
        .await;
        let mut configured_account = account(false);
        configured_account.api_base = server.uri();
        let runtime = DiscordInboundRuntime {
            http: Client::new(),
            account_id: "main".to_owned(),
            account: configured_account,
            router,
            bridge,
            dispatcher: Arc::new(crate::dispatcher::ChannelDispatcherImpl::new()),
        };
        let bot = DiscordCurrentUser {
            id: "bot-999".to_owned(),
            username: Some("Garyx".to_owned()),
        };

        DiscordChannel::handle_message_create(
            &runtime,
            &bot,
            DiscordMessageCreateEvent {
                id: "message-001".to_owned(),
                channel_id: "dm-channel-123".to_owned(),
                guild_id: None,
                content: "hello".to_owned(),
                author: DiscordUser {
                    id: "user-123".to_owned(),
                    username: Some("Test User".to_owned()),
                    bot: false,
                },
                mentions: Vec::new(),
                message_reference: None,
                attachments: Vec::new(),
            },
        )
        .await;

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            body["content"],
            "Error: no enabled standalone agent is available"
        );
        assert_eq!(body["message_reference"]["message_id"], "message-001");
    }

    #[test]
    fn dm_message_does_not_require_mention() {
        let event = DiscordMessageCreateEvent {
            id: "message-001".to_owned(),
            channel_id: "dm-channel-123".to_owned(),
            guild_id: None,
            content: "hello from dm".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: Vec::new(),
            message_reference: None,
            attachments: Vec::new(),
        };

        let request = build_inbound_request("main", &account(true), "bot-999", event)
            .expect("dm should route without mention");

        assert_eq!(request.channel, "discord");
        assert_eq!(request.account_id, "main");
        assert_eq!(request.from_id, "user-123");
        assert!(!request.is_group);
        assert_eq!(request.thread_binding_key, "user-123");
        assert_eq!(request.message, "hello from dm");
        assert_eq!(request.extra_metadata["chat_id"], "dm-channel-123");
        assert_eq!(request.extra_metadata["display_label"], "Test User");
    }

    #[test]
    fn guild_message_requires_mention_by_default() {
        let event = DiscordMessageCreateEvent {
            id: "message-002".to_owned(),
            channel_id: "guild-channel-123".to_owned(),
            guild_id: Some("guild-456".to_owned()),
            content: "not for the bot".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: Vec::new(),
            message_reference: None,
            attachments: Vec::new(),
        };

        assert!(build_inbound_request("main", &account(true), "bot-999", event).is_none());
    }

    #[test]
    fn guild_mention_is_stripped_without_adding_reference_text() {
        let event = DiscordMessageCreateEvent {
            id: "message-003".to_owned(),
            channel_id: "guild-channel-123".to_owned(),
            guild_id: Some("guild-456".to_owned()),
            content: "<@bot-999> please help".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: vec![DiscordUser {
                id: "bot-999".to_owned(),
                username: Some("Garyx".to_owned()),
                bot: true,
            }],
            message_reference: Some(DiscordMessageReference {
                message_id: Some("reply-001".to_owned()),
            }),
            attachments: Vec::new(),
        };

        let request = build_inbound_request("main", &account(true), "bot-999", event)
            .expect("mentioned guild message should route");

        assert!(request.is_group);
        assert_eq!(request.thread_binding_key, "guild-channel-123");
        assert_eq!(request.message, "please help");
        assert_eq!(request.extra_metadata["guild_id"], "guild-456");
        assert_eq!(
            request.extra_metadata["delivery_thread_id"],
            "guild-channel-123"
        );
    }

    #[tokio::test]
    async fn referenced_guild_message_uses_current_binding() {
        let event = DiscordMessageCreateEvent {
            id: "message-current-binding".to_owned(),
            channel_id: "guild-channel-123".to_owned(),
            guild_id: Some("guild-456".to_owned()),
            content: "<@bot-999> follow up".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: vec![DiscordUser {
                id: "bot-999".to_owned(),
                username: Some("Garyx".to_owned()),
                bot: true,
            }],
            message_reference: Some(DiscordMessageReference {
                message_id: Some("message-from-another-thread".to_owned()),
            }),
            attachments: Vec::new(),
        };
        let request = build_inbound_request("main", &account(true), "bot-999", event)
            .expect("referenced guild message should route");
        assert_eq!(request.message, "follow up");

        let router = crate::test_helpers::make_router();
        {
            let mut router_guard = router.lock().await;
            router_guard
                .ensure_thread_entry(
                    "thread::discord-current",
                    "discord",
                    "main",
                    "guild-channel-123",
                    Some("Current"),
                )
                .await;
            let binding_key =
                MessageRouter::build_binding_context_key("discord", "main", "guild-channel-123");
            router_guard.switch_to_thread(&binding_key, "thread::discord-current");
        }
        let bridge = crate::test_helpers::make_bridge_with(Arc::new(
            crate::test_helpers::ConfigurableTestProvider::echo(),
        ))
        .await;
        let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(|_| {});
        let result = router
            .lock()
            .await
            .route_and_dispatch(request, bridge.as_ref(), Some(callback))
            .await
            .expect("referenced message should dispatch");

        assert_eq!(result.thread_id, "thread::discord-current");
    }

    #[test]
    fn mention_only_message_without_attachments_is_ignored() {
        let event = DiscordMessageCreateEvent {
            id: "message-004".to_owned(),
            channel_id: "guild-channel-123".to_owned(),
            guild_id: Some("guild-456".to_owned()),
            content: "<@bot-999>".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: vec![DiscordUser {
                id: "bot-999".to_owned(),
                username: Some("Garyx".to_owned()),
                bot: true,
            }],
            message_reference: None,
            attachments: Vec::new(),
        };

        assert!(build_inbound_request("main", &account(true), "bot-999", event).is_none());
    }

    #[test]
    fn empty_text_message_with_attachment_routes_without_fallback_text() {
        let event = DiscordMessageCreateEvent {
            id: "message-005".to_owned(),
            channel_id: "dm-channel-123".to_owned(),
            guild_id: None,
            content: String::new(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: Vec::new(),
            message_reference: None,
            attachments: vec![DiscordAttachment {
                id: "attachment-file".to_owned(),
                filename: "report.txt".to_owned(),
                content_type: Some("text/plain".to_owned()),
                size: Some(12),
                url: "https://example.invalid/files/report.txt".to_owned(),
            }],
        };

        let request = build_inbound_request("main", &account(true), "bot-999", event)
            .expect("attachment-only dm should route");

        assert_eq!(request.message, "");
        assert_eq!(request.extra_metadata[NATIVE_COMMAND_TEXT_METADATA_KEY], "");
    }

    #[test]
    fn bot_authored_messages_are_ignored() {
        let event = DiscordMessageCreateEvent {
            id: "message-006".to_owned(),
            channel_id: "dm-channel-123".to_owned(),
            guild_id: None,
            content: "ignore me".to_owned(),
            author: DiscordUser {
                id: "bot-999".to_owned(),
                username: Some("Garyx".to_owned()),
                bot: true,
            },
            mentions: Vec::new(),
            message_reference: None,
            attachments: Vec::new(),
        };

        assert!(build_inbound_request("main", &account(true), "bot-999", event).is_none());
    }

    #[test]
    fn discord_gateway_resume_payload_preserves_session_cursor() {
        let payload = discord_resume_payload("discord-token", "session-123", 42);

        assert_eq!(payload["op"], 6);
        assert_eq!(payload["d"]["token"], "discord-token");
        assert_eq!(payload["d"]["session_id"], "session-123");
        assert_eq!(payload["d"]["seq"], 42);
    }

    #[test]
    fn discord_gateway_url_query_keeps_root_path() {
        assert_eq!(
            discord_gateway_url_with_query("wss://gateway.discord.gg"),
            "wss://gateway.discord.gg/?v=10&encoding=json"
        );
        assert_eq!(
            discord_gateway_url_with_query("wss://gateway.discord.gg/"),
            "wss://gateway.discord.gg/?v=10&encoding=json"
        );
        assert_eq!(
            discord_gateway_url_with_query("wss://gateway.discord.gg/?v=10&encoding=json"),
            "wss://gateway.discord.gg/?v=10&encoding=json"
        );
    }

    #[tokio::test]
    async fn inbound_downloads_discord_images_and_files() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/files/plot.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .set_body_bytes(b"fake png".to_vec()),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/files/report.txt"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_bytes(b"report bytes".to_vec()),
            )
            .mount(&server)
            .await;

        let event = DiscordMessageCreateEvent {
            id: "message-005".to_owned(),
            channel_id: "dm-channel-123".to_owned(),
            guild_id: None,
            content: "see attached".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: Vec::new(),
            message_reference: None,
            attachments: vec![
                DiscordAttachment {
                    id: "attachment-image".to_owned(),
                    filename: "plot.png".to_owned(),
                    content_type: Some("image/png".to_owned()),
                    size: Some(8),
                    url: format!("{}/files/plot.png", server.uri()),
                },
                DiscordAttachment {
                    id: "attachment-file".to_owned(),
                    filename: "report.txt".to_owned(),
                    content_type: Some("text/plain".to_owned()),
                    size: Some(12),
                    url: format!("{}/files/report.txt", server.uri()),
                },
            ],
        };
        let mut request = build_inbound_request("main", &account(true), "bot-999", event.clone())
            .expect("discord message should route");
        let runtime = DiscordInboundRuntime {
            http: Client::new(),
            account_id: "main".to_owned(),
            account: account(true),
            router: crate::test_helpers::make_router(),
            bridge: crate::test_helpers::make_bridge_with(Arc::new(
                crate::test_helpers::ConfigurableTestProvider::echo(),
            ))
            .await,
            dispatcher: Arc::new(crate::dispatcher::ChannelDispatcherImpl::new()),
        };

        enrich_inbound_request_with_discord_attachments(&runtime, &event, &mut request).await;

        let prompt_attachments =
            garyx_models::provider::attachments_from_metadata(&request.extra_metadata);
        assert_eq!(prompt_attachments.len(), 1);
        assert_eq!(prompt_attachments[0].kind, PromptAttachmentKind::Image);
        assert_eq!(prompt_attachments[0].name, "plot.png");
        assert_eq!(prompt_attachments[0].media_type, "image/png");
        assert!(Path::new(&prompt_attachments[0].path).is_file());
        assert_eq!(request.file_paths.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&request.file_paths[0]).expect("downloaded file"),
            "report bytes"
        );
        assert_eq!(request.extra_metadata["image_count"], 1);
        assert_eq!(request.extra_metadata["file_count"], 1);
        assert_eq!(request.extra_metadata["attachment_count"], 2);

        let _ = std::fs::remove_file(&prompt_attachments[0].path);
        let _ = std::fs::remove_file(&request.file_paths[0]);
    }

    #[tokio::test]
    async fn response_callback_sends_final_text_with_wire_reply() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-reply-001"
            })))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::Delta {
            text: "在".to_owned(),
        });
        callback(StreamEvent::Delta {
            text: "。".to_owned(),
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            server
                .received_requests()
                .await
                .expect("received requests")
                .is_empty(),
            "Discord text deltas should buffer until a tool call or final Done"
        );

        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if !requests.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 1);
        let create_body: Value =
            serde_json::from_slice(&requests[0].body).expect("discord create body");
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(create_body["content"], "在。");
        assert_eq!(
            create_body["message_reference"]["message_id"],
            "message-001"
        );
    }

    #[tokio::test]
    async fn response_callback_flushes_buffered_text_when_tool_starts() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-buffered-tool-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::Delta {
            text: "before ".to_owned(),
        });
        callback(StreamEvent::Delta {
            text: "tool".to_owned(),
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            server
                .received_requests()
                .await
                .expect("received requests")
                .is_empty(),
            "Discord should not send buffered text before a tool call"
        );

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Bash"}),
                Some("tool-bash-1".to_owned()),
                None,
            ),
        });
        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 2);
        let create_body: Value =
            serde_json::from_slice(&requests[0].body).expect("discord create body");
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(create_body["content"], "before tool\n\n🔧 #1 Bash");
        let edit_body: Value =
            serde_json::from_slice(&requests[1].body).expect("discord edit body");
        assert_eq!(requests[1].method.as_str(), "PATCH");
        assert_eq!(edit_body["content"], "before tool");
    }

    #[tokio::test]
    async fn response_callback_replaces_tool_placeholder_with_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-tool-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Read"}),
                Some("tool-read-1".to_owned()),
                None,
            ),
        });
        callback(StreamEvent::Delta {
            text: "done".to_owned(),
        });
        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 2);
        let create_body: Value =
            serde_json::from_slice(&requests[0].body).expect("discord create body");
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(create_body["content"], "🔧 #1 Read");
        let edit_body: Value =
            serde_json::from_slice(&requests[1].body).expect("discord edit body");
        assert_eq!(requests[1].method.as_str(), "PATCH");
        assert_eq!(edit_body["content"], "done");
    }

    #[tokio::test]
    async fn response_callback_falls_back_to_new_message_when_final_edit_fails() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-placeholder-001"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path(
                "/channels/dm-channel-123/messages/discord-placeholder-001",
            ))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "code": 10008,
                "message": "Unknown Message"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-final-001"
            })))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-edit-fallback-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Bash"}),
                Some("tool-bash-1".to_owned()),
                None,
            ),
        });
        callback(StreamEvent::Delta {
            text: "final text".to_owned(),
        });
        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..30 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(requests[1].method.as_str(), "PATCH");
        assert_eq!(requests[2].method.as_str(), "POST");
        let fallback_body: Value =
            serde_json::from_slice(&requests[2].body).expect("discord fallback body");
        assert_eq!(fallback_body["content"], "final text");
    }

    #[tokio::test]
    async fn response_callback_done_resets_state_before_later_user_ack() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-final-001"
            })))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-done-reset-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::Delta {
            text: "old final".to_owned(),
        });
        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(requests.len(), 1);

        callback(StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: None,
        });
        tokio::time::sleep(Duration::from_millis(150)).await;

        requests = server.received_requests().await.expect("received requests");
        assert_eq!(
            requests.len(),
            1,
            "a user ack after Done must not resend stale accumulated text"
        );
    }

    #[tokio::test]
    async fn discord_sender_retries_transient_create_message_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "message": "temporary upstream failure"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-retry-001"
            })))
            .mount(&server)
            .await;

        let sender = DiscordSender {
            account_id: "main".to_owned(),
            token: "discord-token".to_owned(),
            http: Client::new(),
            api_base: server.uri(),
            is_running: true,
        };

        let message_ids = sender
            .send_text("dm-channel-123", "retry me", Some("message-001"))
            .await
            .expect("transient Discord create failure should be retried");

        assert_eq!(message_ids, vec!["discord-retry-001".to_owned()]);
        let requests = server.received_requests().await.expect("received requests");
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn response_callback_deletes_runtime_only_tool_placeholder_on_done() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-tool-only-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Bash"}),
                Some("tool-bash-1".to_owned()),
                None,
            ),
        });
        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(requests[1].method.as_str(), "DELETE");
    }

    #[tokio::test]
    async fn response_callback_user_ack_boundary_splits_messages() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-boundary-001"
            })))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-boundary-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::Delta {
            text: "第一段".to_owned(),
        });
        callback(StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: None,
        });
        callback(StreamEvent::Delta {
            text: "第二段".to_owned(),
        });
        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 2);
        let first_body: Value =
            serde_json::from_slice(&requests[0].body).expect("discord first body");
        let second_body: Value =
            serde_json::from_slice(&requests[1].body).expect("discord second body");
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(requests[1].method.as_str(), "POST");
        assert_eq!(first_body["content"], "第一段");
        assert_eq!(second_body["content"], "第二段");
    }

    #[tokio::test]
    async fn response_callback_user_ack_deletes_runtime_only_tool_placeholder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-tool-boundary-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Bash"}),
                Some("tool-bash-1".to_owned()),
                None,
            ),
        });
        callback(StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: None,
        });
        callback(StreamEvent::Delta {
            text: "after".to_owned(),
        });
        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 3);
        let tool_body: Value =
            serde_json::from_slice(&requests[0].body).expect("discord tool body");
        let after_body: Value =
            serde_json::from_slice(&requests[2].body).expect("discord after body");
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(tool_body["content"], "🔧 #1 Bash");
        assert_eq!(requests[1].method.as_str(), "DELETE");
        assert_eq!(requests[2].method.as_str(), "POST");
        assert_eq!(after_body["content"], "after");
    }

    #[tokio::test]
    async fn response_callback_user_ack_cancels_scheduled_tool_placeholder_update() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-tool-boundary-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Bash"}),
                Some("tool-bash-1".to_owned()),
                None,
            ),
        });
        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if !requests.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(requests.len(), 1);

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Read"}),
                Some("tool-read-1".to_owned()),
                None,
            ),
        });
        callback(StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: None,
        });

        tokio::time::sleep(DISCORD_TOOL_PLACEHOLDER_UPDATE_INTERVAL + Duration::from_millis(150))
            .await;
        requests = server.received_requests().await.expect("received requests");

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(requests[1].method.as_str(), "DELETE");
    }

    #[tokio::test]
    async fn response_callback_coalesces_rapid_tool_placeholder_updates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-tool-001"
            })))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-tool-coalesce-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Bash"}),
                Some("tool-bash-1".to_owned()),
                None,
            ),
        });
        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(requests.len(), 1);

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Read"}),
                Some("tool-read-1".to_owned()),
                None,
            ),
        });
        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Write"}),
                Some("tool-write-1".to_owned()),
                None,
            ),
        });

        tokio::time::sleep(Duration::from_millis(200)).await;
        requests = server.received_requests().await.expect("received requests");
        assert_eq!(
            requests.len(),
            1,
            "rapid Discord tool placeholders should wait for the coalesced update"
        );

        for _ in 0..30 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 2);
        let create_body: Value =
            serde_json::from_slice(&requests[0].body).expect("discord create body");
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(create_body["content"], "🔧 #1 Bash");
        let edit_body: Value =
            serde_json::from_slice(&requests[1].body).expect("discord edit body");
        assert_eq!(requests[1].method.as_str(), "PATCH");
        assert_eq!(edit_body["content"], "🔧 #3 Write");
    }

    #[tokio::test]
    async fn response_callback_suppresses_child_agent_tool_placeholder() {
        let server = MockServer::start().await;
        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-child-tool-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::ToolUse {
            message: ProviderMessage::tool_use(
                json!({"name": "Bash"}),
                Some("tool-child-1".to_owned()),
                None,
            )
            .with_metadata_value("parent_tool_use_id", json!("tool-parent")),
        });
        callback(StreamEvent::Done);
        tokio::time::sleep(Duration::from_millis(150)).await;

        let requests = server.received_requests().await.expect("received requests");
        assert!(requests.is_empty());
    }

    #[tokio::test]
    async fn response_callback_sends_local_markdown_images_as_attachments() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-text-001"
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::TempDir::new().unwrap();
        let image_path = tmp.path().join("plot.png");
        std::fs::write(&image_path, b"fake png").unwrap();

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-image-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::Delta {
            text: format!("结果如下\n![plot]({})", image_path.display()),
        });
        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 2);
        let text_body: Value =
            serde_json::from_slice(&requests[0].body).expect("discord text body");
        assert_eq!(text_body["content"], "结果如下");
        assert_eq!(requests[1].method.as_str(), "POST");
        assert!(
            String::from_utf8_lossy(&requests[1].body).contains("plot.png"),
            "multipart body should include the local image filename"
        );
    }

    #[tokio::test]
    async fn response_callback_sends_remote_markdown_images_as_attachments() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/images/plot.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .set_body_bytes(b"fake png".to_vec()),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-text-001"
            })))
            .mount(&server)
            .await;

        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                chat_id: "dm-channel-123".to_owned(),
                reply_to_message_id: Some("message-001".to_owned()),
            });
        thread_id_tx
            .send("thread::discord-remote-image-test".to_owned())
            .expect("thread id receiver should still be alive");

        callback(StreamEvent::Delta {
            text: format!("结果如下\n![plot]({}/images/plot.png)", server.uri()),
        });
        callback(StreamEvent::Done);

        let mut requests = Vec::new();
        for _ in 0..20 {
            requests = server.received_requests().await.expect("received requests");
            if requests.len() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(requests.len(), 3);
        assert!(
            requests
                .iter()
                .any(|request| request.method.as_str() == "GET"
                    && request.url.path() == "/images/plot.png")
        );
        let post_bodies = requests
            .iter()
            .filter(|request| request.method.as_str() == "POST")
            .map(|request| String::from_utf8_lossy(&request.body).to_string())
            .collect::<Vec<_>>();
        assert_eq!(post_bodies.len(), 2);
        assert!(
            post_bodies.iter().any(|body| body.contains("结果如下")),
            "text message should strip the markdown image"
        );
        assert!(
            post_bodies.iter().any(|body| body.contains("plot.png")),
            "multipart body should include the downloaded remote image filename"
        );
    }
}
