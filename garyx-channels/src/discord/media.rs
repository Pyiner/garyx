use super::*;

const DISCORD_MAX_IMAGE_SIZE_BYTES: u64 = 5 * 1024 * 1024;
const DISCORD_MAX_FILE_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

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
        warn!(target: "garyx_channels::discord",
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
        warn!(target: "garyx_channels::discord",
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
            warn!(target: "garyx_channels::discord",
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
        warn!(target: "garyx_channels::discord",
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
        warn!(target: "garyx_channels::discord",
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
            warn!(target: "garyx_channels::discord",
                account_id = %runtime.account_id,
                attachment_id = %attachment.id,
                error = %error,
                "failed to read Discord attachment bytes"
            );
            return None;
        }
    };
    if bytes.is_empty() || bytes.len() as u64 > max_bytes {
        warn!(target: "garyx_channels::discord",
            account_id = %runtime.account_id,
            attachment_id = %attachment.id,
            size = bytes.len(),
            "skipping empty or oversized Discord attachment payload"
        );
        return None;
    }

    let base_dir = std::env::temp_dir().join("garyx-discord").join("inbound");
    if let Err(error) = tokio::fs::create_dir_all(&base_dir).await {
        warn!(target: "garyx_channels::discord",
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
        warn!(target: "garyx_channels::discord",
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

pub(super) async fn enrich_inbound_request_with_discord_attachments(
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
pub(super) struct MarkdownImageRef {
    pub(super) target: MarkdownImageTarget,
    pub(super) alt: Option<String>,
    source_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MarkdownImageTarget {
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

pub(super) fn extract_markdown_image_refs(text: &str) -> Vec<MarkdownImageRef> {
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

pub(super) fn strip_deliverable_markdown_images(text: &str) -> String {
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

pub(super) fn discord_editable_text(text: &str) -> String {
    let display = strip_deliverable_markdown_images(text);
    if display.len() <= DISCORD_MAX_MESSAGE_LENGTH {
        return display;
    }
    split_discord_message(&display)
        .into_iter()
        .next()
        .unwrap_or_default()
}

pub(super) fn markdown_image_key(image_ref: &MarkdownImageRef) -> String {
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

pub(super) async fn download_remote_markdown_image(
    http: &Client,
    url: &str,
    alt: Option<&str>,
) -> Option<PathBuf> {
    let response = match http.get(url).send().await {
        Ok(response) => response,
        Err(error) => {
            warn!(target: "garyx_channels::discord",
                error = %error,
                "failed to download Discord remote markdown image"
            );
            return None;
        }
    };
    let status = response.status();
    if !status.is_success() {
        warn!(target: "garyx_channels::discord",
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
        warn!(target: "garyx_channels::discord", "skipping Discord remote markdown image with unknown media type");
        return None;
    };
    if !is_supported_discord_image_media_type(&media_type) {
        warn!(target: "garyx_channels::discord",
            media_type = %media_type,
            "skipping unsupported Discord remote markdown image"
        );
        return None;
    }
    if let Some(content_length) = response.content_length()
        && content_length > DISCORD_MAX_IMAGE_SIZE_BYTES
    {
        warn!(target: "garyx_channels::discord",
            size = content_length,
            "skipping oversized Discord remote markdown image"
        );
        return None;
    }

    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            warn!(target: "garyx_channels::discord",
                error = %error,
                "failed to read Discord remote markdown image bytes"
            );
            return None;
        }
    };
    if bytes.is_empty() || bytes.len() as u64 > DISCORD_MAX_IMAGE_SIZE_BYTES {
        warn!(target: "garyx_channels::discord",
            size = bytes.len(),
            "skipping empty or oversized Discord remote markdown image"
        );
        return None;
    }

    let base_dir = std::env::temp_dir()
        .join("garyx-discord")
        .join("outbound-markdown");
    if let Err(error) = tokio::fs::create_dir_all(&base_dir).await {
        warn!(target: "garyx_channels::discord",
            error = %error,
            "failed to create Discord outbound markdown image directory"
        );
        return None;
    }
    let name = remote_markdown_image_name(url, alt, &media_type);
    let path = base_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), name));
    if let Err(error) = tokio::fs::write(&path, &bytes).await {
        warn!(target: "garyx_channels::discord",
            error = %error,
            "failed to write Discord remote markdown image to disk"
        );
        return None;
    }
    Some(path)
}
