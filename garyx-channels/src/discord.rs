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
use garyx_router::{InboundRequest, MessageRouter, NATIVE_COMMAND_TEXT_METADATA_KEY};

use crate::channel_trait::{Channel, ChannelError};
use crate::dispatcher::{DISCORD_MAX_MESSAGE_LENGTH, DiscordSender, split_discord_message};
use crate::generated_images::{extract_image_generation_result, write_generated_image_temp};

const DISCORD_GATEWAY_INTENTS: u64 = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);
const DISCORD_RECONNECT_DELAY: Duration = Duration::from_secs(5);
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
        format!("{trimmed}?v=10&encoding=json")
    }
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

    let mut message = if mentioned {
        strip_discord_bot_mention(&event.content, bot_id)
    } else {
        event.content.trim().to_owned()
    };
    if message.trim().is_empty() {
        message = "(The user sent a message with no text content)".to_owned();
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
        reply_to_message_id: event
            .message_reference
            .as_ref()
            .and_then(|reference| reference.message_id.clone()),
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
    pub router: Arc<Mutex<MessageRouter>>,
    pub chat_id: String,
    pub thread_binding_key: String,
    pub reply_to_message_id: Option<String>,
}

struct DiscordStreamState {
    accumulated_text: String,
    markdown_image_scan_text: String,
    sent_markdown_image_keys: HashSet<String>,
    recorded_message_ids: HashSet<String>,
    message_id: Option<String>,
    last_rendered_text: String,
    last_edit_time: Instant,
    finalized: bool,
}

impl Default for DiscordStreamState {
    fn default() -> Self {
        Self {
            accumulated_text: String::new(),
            markdown_image_scan_text: String::new(),
            sent_markdown_image_keys: HashSet::new(),
            recorded_message_ids: HashSet::new(),
            message_id: None,
            last_rendered_text: String::new(),
            last_edit_time: Instant::now(),
            finalized: false,
        }
    }
}

struct DiscordStreamingCallbackShared {
    cfg: DiscordStreamingCallbackConfig,
    state: Mutex<DiscordStreamState>,
}

impl DiscordStreamingCallbackShared {
    async fn record_outbound_messages(&self, thread_id: &str, message_ids: &[String]) {
        if thread_id.trim().is_empty() || message_ids.is_empty() {
            return;
        }
        let thread_binding_key = if self.cfg.thread_binding_key.trim().is_empty() {
            None
        } else {
            Some(self.cfg.thread_binding_key.as_str())
        };
        let mut router = self.cfg.router.lock().await;
        for message_id in message_ids {
            router
                .record_outbound_message_with_persistence(
                    thread_id,
                    "discord",
                    &self.cfg.sender.account_id,
                    &self.cfg.chat_id,
                    thread_binding_key,
                    message_id,
                )
                .await;
        }
    }

    async fn record_new_outbound_messages(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        message_ids: Vec<String>,
    ) {
        let new_ids = message_ids
            .into_iter()
            .filter(|message_id| state.recorded_message_ids.insert(message_id.clone()))
            .collect::<Vec<_>>();
        if !new_ids.is_empty() {
            self.record_outbound_messages(thread_id, &new_ids).await;
        }
    }

    async fn send_initial_text(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        display_text: &str,
    ) {
        if display_text.trim().is_empty() {
            return;
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
                state.last_edit_time = Instant::now();
                self.record_new_outbound_messages(thread_id, state, message_ids)
                    .await;
            }
            Err(error) => {
                warn!(
                    account_id = %self.cfg.sender.account_id,
                    chat_id = %self.cfg.chat_id,
                    thread_id,
                    error = %error,
                    "failed to send Discord streamed response"
                );
            }
        }
    }

    async fn edit_existing_text(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        display_text: &str,
    ) {
        let Some(message_id) = state.message_id.as_deref() else {
            self.send_initial_text(thread_id, state, display_text).await;
            return;
        };
        if display_text.trim().is_empty() || display_text.trim() == state.last_rendered_text.trim()
        {
            return;
        }
        match self
            .cfg
            .sender
            .edit_text(&self.cfg.chat_id, message_id, display_text)
            .await
        {
            Ok(edited_id) => {
                state.last_rendered_text = display_text.to_owned();
                state.last_edit_time = Instant::now();
                self.record_new_outbound_messages(thread_id, state, vec![edited_id])
                    .await;
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
            }
        }
    }

    async fn finalize_text(&self, thread_id: &str, state: &mut DiscordStreamState) {
        let display_text = strip_deliverable_markdown_images(&state.accumulated_text);
        if display_text.trim().is_empty() {
            return;
        }
        let chunks = split_discord_message(&display_text);
        let first_chunk = chunks.first().cloned().unwrap_or_default();
        if state.message_id.is_some() {
            self.edit_existing_text(thread_id, state, &first_chunk)
                .await;
            if chunks.len() > 1 {
                match self
                    .cfg
                    .sender
                    .send_text(&self.cfg.chat_id, &chunks[1..].join(""), None)
                    .await
                {
                    Ok(message_ids) => {
                        self.record_new_outbound_messages(thread_id, state, message_ids)
                            .await;
                    }
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
            self.send_initial_text(thread_id, state, &display_text)
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
                Ok(message_ids) => {
                    self.record_new_outbound_messages(thread_id, state, message_ids)
                        .await;
                }
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
            Ok(message_ids) => {
                self.record_new_outbound_messages(thread_id, state, message_ids)
                    .await;
            }
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

    async fn process_event(&self, event: StreamEvent, thread_id: &str) {
        let mut state = self.state.lock().await;
        match event {
            StreamEvent::SessionBound { .. } | StreamEvent::ThreadTitleUpdated { .. } => {}
            StreamEvent::ToolUse { .. } => {}
            StreamEvent::ToolResult { message } => {
                self.process_generated_image_result(thread_id, &mut state, message)
                    .await;
            }
            StreamEvent::Boundary { kind, .. } => {
                if kind == StreamBoundaryKind::AssistantSegment {
                    crate::streaming_core::apply_stream_boundary_text(
                        &mut state.accumulated_text,
                        StreamBoundaryKind::AssistantSegment,
                    );
                    crate::streaming_core::apply_stream_boundary_text(
                        &mut state.markdown_image_scan_text,
                        StreamBoundaryKind::AssistantSegment,
                    );
                }
            }
            StreamEvent::Delta { text } => {
                if text.is_empty() {
                    return;
                }
                state.markdown_image_scan_text.push_str(&text);
                state.accumulated_text =
                    crate::streaming_core::merge_stream_text(&state.accumulated_text, &text);
                state.finalized = false;
                let display_text = discord_editable_text(&state.accumulated_text);
                if state.message_id.is_none() {
                    self.send_initial_text(thread_id, &mut state, &display_text)
                        .await;
                } else if state.last_edit_time.elapsed() >= Duration::from_millis(300) {
                    self.edit_existing_text(thread_id, &mut state, &display_text)
                        .await;
                }
            }
            StreamEvent::Done => {
                if state.finalized {
                    return;
                }
                state.finalized = true;
                self.finalize_text(thread_id, &mut state).await;
                self.send_markdown_images_from_state(thread_id, &mut state)
                    .await;
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
}

impl DiscordChannel {
    pub fn new(
        config: DiscordConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
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
                router: runtime.router.clone(),
                chat_id: reply_target.clone(),
                thread_binding_key: thread_binding_key.clone(),
                reply_to_message_id: Some(reply_to.clone()),
            });
        let dispatch_result = {
            let mut router = runtime.router.lock().await;
            router
                .route_and_dispatch(request, runtime.bridge.as_ref(), Some(response_callback))
                .await
        };
        match dispatch_result {
            Ok(result) => {
                let _ = thread_id_tx.send(result.thread_id.clone());
                if let Some(local_reply) = result.local_reply {
                    match sender
                        .send_text(&reply_target, &local_reply, Some(&reply_to))
                        .await
                    {
                        Ok(message_ids) => {
                            let mut router = runtime.router.lock().await;
                            for message_id in message_ids {
                                router
                                    .record_outbound_message_with_persistence(
                                        &result.thread_id,
                                        "discord",
                                        &runtime.account_id,
                                        &reply_target,
                                        Some(&thread_binding_key),
                                        &message_id,
                                    )
                                    .await;
                            }
                        }
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
                warn!(
                    account_id = %runtime.account_id,
                    error = %error,
                    "failed to route Discord inbound message"
                );
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
    use garyx_models::config::DiscordAccount;
    use garyx_models::provider::StreamEvent;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn account(require_mention: bool) -> DiscordAccount {
        DiscordAccount {
            token: "discord-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            require_mention,
            api_base: "https://discord.com/api/v10".to_owned(),
            gateway_url: "wss://gateway.discord.gg/?v=10&encoding=json".to_owned(),
        }
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
    fn guild_mention_is_stripped_and_reply_id_is_preserved() {
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
        assert_eq!(request.reply_to_message_id.as_deref(), Some("reply-001"));
        assert_eq!(request.extra_metadata["guild_id"], "guild-456");
        assert_eq!(
            request.extra_metadata["delivery_thread_id"],
            "guild-channel-123"
        );
    }

    #[test]
    fn bot_authored_messages_are_ignored() {
        let event = DiscordMessageCreateEvent {
            id: "message-004".to_owned(),
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
    async fn response_callback_sends_final_text_and_records_outbound() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/channels/dm-channel-123/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-reply-001"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/channels/dm-channel-123/messages/discord-reply-001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "discord-reply-001"
            })))
            .mount(&server)
            .await;

        let router = crate::test_helpers::make_router();
        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                router: router.clone(),
                chat_id: "dm-channel-123".to_owned(),
                thread_binding_key: "user-123".to_owned(),
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
        assert_eq!(create_body["content"], "在");
        assert_eq!(
            create_body["message_reference"]["message_id"],
            "message-001"
        );
        let edit_body: Value =
            serde_json::from_slice(&requests[1].body).expect("discord edit body");
        assert_eq!(requests[1].method.as_str(), "PATCH");
        assert_eq!(edit_body["content"], "在。");

        let router = router.lock().await;
        assert_eq!(
            router.resolve_reply_thread_for_chat(
                "discord",
                "main",
                Some("dm-channel-123"),
                Some("user-123"),
                "discord-reply-001",
            ),
            Some("thread::discord-test"),
        );
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

        let router = crate::test_helpers::make_router();
        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                router,
                chat_id: "dm-channel-123".to_owned(),
                thread_binding_key: "user-123".to_owned(),
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

        let router = crate::test_helpers::make_router();
        let (callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: DiscordSender {
                    account_id: "main".to_owned(),
                    token: "discord-token".to_owned(),
                    http: Client::new(),
                    api_base: server.uri(),
                    is_running: true,
                },
                router,
                chat_id: "dm-channel-123".to_owned(),
                thread_binding_key: "user-123".to_owned(),
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
