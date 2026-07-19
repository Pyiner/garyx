//! Weixin media handling: outbound media references, markdown
//! extraction/plain-text rendering, CDN upload/download, and inbound
//! attachment extraction. Moved verbatim from weixin.rs (Phase-7
//! pure code motion).

use super::*;

pub(super) fn markdown_image_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"!\[[^\]]*\]\(([^)]+)\)").expect("valid markdown image regex"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OutboundMediaRef {
    RemoteUrl(String),
    LocalPath(String),
    InlineImage {
        id: String,
        bytes: Vec<u8>,
        file_name: String,
    },
}

impl OutboundMediaRef {
    pub(super) fn dedupe_key(&self) -> String {
        match self {
            Self::RemoteUrl(url) => format!("url:{url}"),
            Self::LocalPath(path) => format!("path:{path}"),
            Self::InlineImage { id, .. } => format!("inline:{id}"),
        }
    }

    pub(super) fn classify_media_type(&self) -> i64 {
        match self {
            Self::RemoteUrl(url) => classify_media_type_from_url(url),
            Self::LocalPath(path) => classify_media_type_from_url(path),
            Self::InlineImage { .. } => 1,
        }
    }

    /// Best-effort filename for WeChat `file_item.file_name`. Strips query
    /// strings from URLs and takes the last path segment; returns `None` when
    /// nothing usable can be recovered (callers fall back to `attachment.bin`).
    pub(super) fn file_name(&self) -> Option<String> {
        let raw = match self {
            Self::RemoteUrl(url) => url.as_str(),
            Self::LocalPath(path) => path.as_str(),
            Self::InlineImage { file_name, .. } => return Some(file_name.clone()),
        };
        // Drop URL query/fragment before taking the basename.
        let without_query = raw.split(['?', '#']).next().unwrap_or(raw);
        let last_segment = without_query
            .rsplit(['/', '\\'])
            .find(|s| !s.is_empty())?
            .to_owned();
        if last_segment.is_empty() {
            return None;
        }
        Some(last_segment)
    }
}

pub(super) fn is_media_item(item: &WeixinMessageItem) -> bool {
    matches!(item.r#type, 2..=5)
}

pub(super) fn body_from_message_item(item: &WeixinMessageItem) -> String {
    match item.r#type {
        1 => {
            let text = item
                .text_item
                .as_ref()
                .map(|value| value.text.trim().to_owned())
                .unwrap_or_default();
            let Some(ref_msg) = item.ref_msg.as_ref() else {
                return text;
            };
            let Some(ref_item) = ref_msg.message_item.as_ref() else {
                return text;
            };
            if is_media_item(ref_item) {
                return text;
            }
            let mut quote_parts = Vec::new();
            if !ref_msg.title.trim().is_empty() {
                quote_parts.push(ref_msg.title.trim().to_owned());
            }
            let ref_body = body_from_message_item(ref_item);
            if !ref_body.trim().is_empty() {
                quote_parts.push(ref_body.trim().to_owned());
            }
            if quote_parts.is_empty() {
                text
            } else if text.trim().is_empty() {
                format!("[引用: {}]", quote_parts.join(" | "))
            } else {
                format!("[引用: {}]\n{text}", quote_parts.join(" | "))
            }
        }
        2 => {
            let Some(image_item) = item.image_item.as_ref() else {
                return "[图片]".to_owned();
            };
            let url = image_item.url.trim();
            if !url.is_empty() {
                format!("[图片] {url}")
            } else {
                "[图片]".to_owned()
            }
        }
        3 => {
            let text = item
                .voice_item
                .as_ref()
                .map(|value| value.text.trim())
                .unwrap_or_default();
            if text.is_empty() {
                "[语音]".to_owned()
            } else {
                text.to_owned()
            }
        }
        4 => "[文件]".to_owned(),
        5 => "[视频]".to_owned(),
        _ => String::new(),
    }
}

pub(super) fn extract_text(items: &[WeixinMessageItem]) -> String {
    items
        .iter()
        .map(body_from_message_item)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) async fn extract_inline_image_attachments(
    items: &[WeixinMessageItem],
) -> Vec<PromptAttachment> {
    let mut attachments = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if item.r#type != 2 {
            continue;
        }
        let Some(url) = item
            .image_item
            .as_ref()
            .map(|value| value.url.trim())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(rest) = url.strip_prefix("data:") else {
            continue;
        };
        let Some((meta, data)) = rest.split_once(',') else {
            continue;
        };
        if !meta.contains(";base64") {
            continue;
        }
        let media_type = meta
            .split(';')
            .next()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("image/jpeg")
            .to_owned();
        if data.trim().is_empty() {
            continue;
        }
        let Ok(bytes) = STANDARD.decode(data.trim()) else {
            warn!(index, "failed to decode inline weixin image payload");
            continue;
        };
        let name = sanitize_filename(&format!(
            "weixin-inline-{}.{}",
            index + 1,
            media_type.rsplit('/').next().unwrap_or("jpg")
        ));
        let Some(path) = persist_inbound_media_bytes("image", Some(&name), &bytes).await else {
            warn!(index, "failed to persist inline weixin image payload");
            continue;
        };
        attachments.push(PromptAttachment {
            kind: PromptAttachmentKind::Image,
            path,
            name,
            media_type,
        });
    }
    attachments
}

pub(super) async fn extract_cdn_image_attachments(
    http: &Client,
    items: &[WeixinMessageItem],
) -> Vec<PromptAttachment> {
    let mut attachments = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if item.r#type != 2 {
            continue;
        }
        let Some(image_item) = item.image_item.as_ref() else {
            continue;
        };
        let Some(media) = image_item.media.as_ref() else {
            continue;
        };
        let encrypted_query_param = media.encrypt_query_param.trim();
        if encrypted_query_param.is_empty() {
            continue;
        }
        let aes_key = if !image_item.aeskey.trim().is_empty() {
            parse_hex_key_16(image_item.aeskey.trim())
        } else {
            parse_aes_key_base64(media.aes_key.trim())
        };
        let Some(aes_key) = aes_key else {
            warn!("weixin image item missing parsable aes key");
            continue;
        };
        match download_and_decrypt_cdn(
            http,
            DEFAULT_WEIXIN_CDN_BASE_URL,
            encrypted_query_param,
            &aes_key,
        )
        .await
        {
            Ok(bytes) => {
                let name_hint = if encrypted_query_param.len() >= 12 {
                    encrypted_query_param[..12].to_owned()
                } else {
                    format!("{:02}", index + 1)
                };
                let name = sanitize_filename(&format!("weixin-cdn-{name_hint}.jpg"));
                let Some(path) = persist_inbound_media_bytes("image", Some(&name), &bytes).await
                else {
                    warn!(name_hint, "failed to persist weixin image from cdn");
                    continue;
                };
                attachments.push(PromptAttachment {
                    kind: PromptAttachmentKind::Image,
                    path,
                    name,
                    media_type: "image/jpeg".to_owned(),
                });
            }
            Err(error) => warn!(error = %error, "failed to decrypt weixin image from cdn"),
        }
    }
    attachments
}

pub(super) fn sanitize_filename(name: &str) -> String {
    crate::sanitize_filename(name.trim())
}

pub(super) async fn persist_inbound_media_bytes(
    media_kind: &str,
    suggested_name: Option<&str>,
    bytes: &[u8],
) -> Option<String> {
    let base_dir = std::env::temp_dir().join("garyx-weixin").join("inbound");
    if fs::create_dir_all(&base_dir).await.is_err() {
        return None;
    }
    let stem = suggested_name
        .map(sanitize_filename)
        .unwrap_or_else(|| format!("{media_kind}.bin"));
    let path = base_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), stem));
    if fs::write(&path, bytes).await.is_err() {
        return None;
    }
    Some(path.to_string_lossy().to_string())
}

pub(super) async fn extract_cdn_non_image_media_metadata(
    http: &Client,
    cdn_base_url: &str,
    items: &[WeixinMessageItem],
) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    let mut video_paths = Vec::new();
    let mut file_paths = Vec::new();

    for item in items {
        let (kind, media, name_hint) = match item.r#type {
            // Skip voice (type 3): SILK format cannot be processed by AI.
            // The transcribed text is already extracted via voice_item.text
            // in body_from_message_item, so we only lose the raw audio file.
            3 => continue,
            4 => (
                "file",
                item.file_item
                    .as_ref()
                    .and_then(|value| value.media.as_ref()),
                item.file_item
                    .as_ref()
                    .map(|value| value.file_name.as_str())
                    .or(Some("file.bin")),
            ),
            5 => (
                "video",
                item.video_item
                    .as_ref()
                    .and_then(|value| value.media.as_ref()),
                Some("video.mp4"),
            ),
            _ => continue,
        };
        let Some(media) = media else {
            continue;
        };
        let encrypted_query_param = media.encrypt_query_param.trim();
        if encrypted_query_param.is_empty() {
            continue;
        }
        let Some(aes_key) = parse_aes_key_base64(media.aes_key.trim()) else {
            warn!(kind = %kind, "weixin media item missing parsable aes key");
            continue;
        };
        let decrypted =
            match download_and_decrypt_cdn(http, cdn_base_url, encrypted_query_param, &aes_key)
                .await
            {
                Ok(bytes) => bytes,
                Err(error) => {
                    warn!(kind = %kind, error = %error, "failed to decrypt weixin media from cdn");
                    continue;
                }
            };
        if let Some(saved_path) = persist_inbound_media_bytes(kind, name_hint, &decrypted).await {
            match kind {
                "video" => video_paths.push(Value::String(saved_path)),
                "file" => file_paths.push(Value::String(saved_path)),
                _ => {}
            }
        }
    }

    if !video_paths.is_empty() {
        metadata.insert("video_paths".to_owned(), Value::Array(video_paths));
    }
    if !file_paths.is_empty() {
        metadata.insert("file_paths".to_owned(), Value::Array(file_paths));
    }
    metadata
}

pub(super) async fn upload_media_to_cdn(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    plaintext: &[u8],
    media_type: i64,
    file_name: Option<String>,
) -> Result<UploadedWeixinMedia, ChannelError> {
    upload_media_to_cdn_with_base(
        http,
        account,
        to_user_id,
        plaintext,
        media_type,
        DEFAULT_WEIXIN_CDN_BASE_URL,
        file_name,
    )
    .await
}

pub(super) async fn upload_media_to_cdn_with_base(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    plaintext: &[u8],
    media_type: i64,
    cdn_base_url: &str,
    file_name: Option<String>,
) -> Result<UploadedWeixinMedia, ChannelError> {
    let aes_key_raw = random_16_bytes();
    let aes_key_hex = bytes_to_hex(&aes_key_raw);
    let ciphertext_size = aes_ecb_padded_size(plaintext.len());
    let filekey = bytes_to_hex(uuid::Uuid::new_v4().as_bytes());
    let upload_url_result = get_upload_url(
        http,
        account,
        to_user_id,
        media_type,
        &filekey,
        plaintext,
        ciphertext_size,
        &aes_key_hex,
    )
    .await?;
    let ciphertext = encrypt_aes_ecb(plaintext, &aes_key_raw)?;
    let cdn_url = match &upload_url_result {
        UploadUrlResult::FullUrl(full_url) => full_url.clone(),
        UploadUrlResult::Param(param) => build_cdn_upload_url(cdn_base_url, param, &filekey),
    };
    // SDK parity: retry up to CDN_UPLOAD_MAX_RETRIES times on 5xx errors.
    let mut last_error = String::new();
    let mut upload_ok = false;
    let mut response_holder: Option<reqwest::Response> = None;
    for attempt in 1..=CDN_UPLOAD_MAX_RETRIES {
        let resp = http
            .post(&cdn_url)
            .header("Content-Type", "application/octet-stream")
            .body(ciphertext.clone())
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                response_holder = Some(r);
                upload_ok = true;
                break;
            }
            Ok(r) if r.status().is_server_error() => {
                let status = r.status();
                let raw = r.text().await.unwrap_or_default();
                last_error = format!("HTTP {status}: {raw}");
                warn!(
                    attempt = attempt,
                    max = CDN_UPLOAD_MAX_RETRIES,
                    error = %last_error,
                    "weixin CDN upload 5xx, retrying"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Ok(r) => {
                // Client error (4xx) — don't retry
                let status = r.status();
                let raw = r.text().await.unwrap_or_default();
                return Err(ChannelError::SendFailed(format!(
                    "Weixin CDN upload HTTP {status}: {raw}"
                )));
            }
            Err(e) => {
                last_error = e.to_string();
                warn!(
                    attempt = attempt,
                    max = CDN_UPLOAD_MAX_RETRIES,
                    error = %last_error,
                    "weixin CDN upload network error, retrying"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    if !upload_ok {
        return Err(ChannelError::SendFailed(format!(
            "Weixin CDN upload failed after {CDN_UPLOAD_MAX_RETRIES} retries: {last_error}"
        )));
    }
    let response = response_holder.unwrap();
    let download_encrypted_query_param = response
        .headers()
        .get("x-encrypted-param")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    if download_encrypted_query_param.is_empty() {
        return Err(ChannelError::SendFailed(
            "Weixin CDN upload missing x-encrypted-param".to_owned(),
        ));
    }
    Ok(UploadedWeixinMedia {
        download_encrypted_query_param,
        aes_key_raw,
        plaintext_size: plaintext.len(),
        ciphertext_size,
        media_type,
        file_name: file_name
            .map(|n| sanitize_filename(&n))
            .filter(|n| !n.is_empty()),
    })
}

pub(super) async fn download_remote_bytes(
    http: &Client,
    url: &str,
) -> Result<Vec<u8>, ChannelError> {
    let response = http.get(url).send().await.map_err(|error| {
        ChannelError::SendFailed(format!("Weixin media download failed: {error}"))
    })?;
    let status = response.status();
    if !status.is_success() {
        let raw = response.text().await.unwrap_or_default();
        return Err(ChannelError::SendFailed(format!(
            "Weixin media download HTTP {status}: {raw}"
        )));
    }
    response
        .bytes()
        .await
        .map(|value| value.to_vec())
        .map_err(|error| ChannelError::SendFailed(format!("Weixin media read failed: {error}")))
}

pub(super) async fn download_and_decrypt_cdn(
    http: &Client,
    cdn_base_url: &str,
    encrypted_query_param: &str,
    aes_key: &[u8; 16],
) -> Result<Vec<u8>, ChannelError> {
    let url = build_cdn_download_url(cdn_base_url, encrypted_query_param);
    let encrypted = download_remote_bytes(http, &url).await?;
    decrypt_aes_ecb(&encrypted, aes_key)
}

pub(super) struct MarkdownRegexes {
    pub(super) fenced_code: Regex,
    pub(super) link: Regex,
    pub(super) blockquote: Regex,
    pub(super) heading: Regex,
    pub(super) hr: Regex,
    pub(super) bold_italic_star: Regex,
    pub(super) bold_italic_under: Regex,
    pub(super) bold_star: Regex,
    pub(super) bold_under: Regex,
    pub(super) italic_star: Regex,
    pub(super) strikethrough: Regex,
    pub(super) inline_code: Regex,
    pub(super) table_separator: Regex,
    pub(super) table_pipe: Regex,
}

/// Pre-compiled regexes for markdown-to-plain-text conversion.
/// Compiled once and cached for the process lifetime.
pub(super) fn markdown_regexes() -> &'static MarkdownRegexes {
    static REGEXES: OnceLock<MarkdownRegexes> = OnceLock::new();
    REGEXES.get_or_init(|| MarkdownRegexes {
        fenced_code: Regex::new(r"```[^\n]*\n?([\s\S]*?)```").unwrap(),
        link: Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap(),
        blockquote: Regex::new(r"(?m)^>\s?").unwrap(),
        heading: Regex::new(r"(?m)^#{1,6}\s+").unwrap(),
        hr: Regex::new(r"(?m)^[\s]*([-*_]){3,}[\s]*$").unwrap(),
        bold_italic_star: Regex::new(r"\*{3}(.+?)\*{3}").unwrap(),
        bold_italic_under: Regex::new(r"_{3}(.+?)_{3}").unwrap(),
        bold_star: Regex::new(r"\*{2}(.+?)\*{2}").unwrap(),
        bold_under: Regex::new(r"_{2}(.+?)_{2}").unwrap(),
        italic_star: Regex::new(r"\*(.+?)\*").unwrap(),
        strikethrough: Regex::new(r"~~(.+?)~~").unwrap(),
        inline_code: Regex::new(r"`([^`]+)`").unwrap(),
        // Only match lines that look like table rows (start/end with |)
        table_separator: Regex::new(r"(?m)^\|[\s:-]+(\|[\s:-]*)+\|?\s*$").unwrap(),
        table_pipe: Regex::new(r"(?m)^(\|.+\|)\s*$").unwrap(),
    })
}

pub(super) fn markdown_to_plain_text(text: &str) -> String {
    let re = markdown_regexes();
    let mut result = text.to_owned();

    // 1. Fenced code blocks: ```lang\ncode``` → code
    result = re.fenced_code.replace_all(&result, "$1").to_string();
    // 2. Markdown images: ![alt](url) → (remove entirely)
    result = markdown_image_regex().replace_all(&result, "").to_string();
    // 3. Markdown links: [text](url) → text
    result = re.link.replace_all(&result, "$1").to_string();
    // 4. Blockquotes: > text → text
    result = re.blockquote.replace_all(&result, "").to_string();
    // 5. Headings: # ... → strip leading hashes
    result = re.heading.replace_all(&result, "").to_string();
    // 6. Horizontal rules: ---, ***, ___ → empty
    result = re.hr.replace_all(&result, "").to_string();
    // 7. Bold + italic: ***text*** / ___text___
    result = re.bold_italic_star.replace_all(&result, "$1").to_string();
    result = re.bold_italic_under.replace_all(&result, "$1").to_string();
    // 8. Bold: **text** / __text__
    result = re.bold_star.replace_all(&result, "$1").to_string();
    result = re.bold_under.replace_all(&result, "$1").to_string();
    // 9. Italic: *text*
    result = re.italic_star.replace_all(&result, "$1").to_string();
    // 10. Strikethrough: ~~text~~
    result = re.strikethrough.replace_all(&result, "$1").to_string();
    // 11. Inline code: `code`
    result = re.inline_code.replace_all(&result, "$1").to_string();
    // 12. Tables: remove separator rows, then strip pipes only from table rows
    //     (rows that start and end with |)
    result = re.table_separator.replace_all(&result, "").to_string();
    result = re
        .table_pipe
        .replace_all(&result, |caps: &regex::Captures| {
            caps[1].replace('|', " ").trim().to_owned()
        })
        .to_string();

    result
}

pub(super) fn looks_like_local_media_path(input: &str) -> bool {
    let candidate = input.trim();
    if !candidate.starts_with('/') {
        return false;
    }
    Path::new(candidate).extension().is_some()
}

/// Check whether a URL/path looks like a known media file (image, video, or
/// a handful of common document types).  Arbitrary URLs that don't match a
/// known media extension are **not** treated as media – this prevents tool
/// results containing API URLs (e.g. `https://api.github.com/…`) from being
/// downloaded and sent as `attachment.bin`.
pub(super) fn looks_like_known_media_url(url: &str) -> bool {
    let lower = url
        .split('?')
        .next()
        .unwrap_or(url)
        .split('#')
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
        || lower.ends_with(".mp4")
        || lower.ends_with(".mov")
        || lower.ends_with(".mkv")
        || lower.ends_with(".webm")
        || lower.ends_with(".pdf")
}

pub(super) fn media_ref_from_string(input: &str) -> Option<OutboundMediaRef> {
    let candidate = input
        .trim()
        .trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`')
        .trim();
    if candidate.is_empty() {
        return None;
    }
    if candidate.starts_with("http://") || candidate.starts_with("https://") {
        // Only treat remote URLs as media when they have a recognisable media
        // extension; otherwise tool-result URLs like GitHub API endpoints get
        // downloaded and sent as `attachment.bin`.
        if looks_like_known_media_url(candidate) {
            return Some(OutboundMediaRef::RemoteUrl(candidate.to_owned()));
        }
        return None;
    }
    if let Some(rest) = candidate.strip_prefix("file://") {
        let decoded = urlencoding::decode(rest).ok()?.to_string();
        if looks_like_local_media_path(&decoded) {
            return Some(OutboundMediaRef::LocalPath(decoded));
        }
        return None;
    }
    if looks_like_local_media_path(candidate) {
        return Some(OutboundMediaRef::LocalPath(candidate.to_owned()));
    }
    None
}

pub(super) fn extract_markdown_media_refs(text: &str) -> Vec<OutboundMediaRef> {
    markdown_image_regex()
        .captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_owned()))
        .filter_map(|raw| media_ref_from_string(&raw))
        .collect()
}

pub(super) fn extract_media_refs_from_value(
    value: &Value,
    out: &mut Vec<OutboundMediaRef>,
    limit: usize,
) {
    if out.len() >= limit {
        return;
    }
    match value {
        Value::String(text) => {
            if let Some(media_ref) = media_ref_from_string(text) {
                out.push(media_ref);
            }
            for media_ref in extract_markdown_media_refs(text) {
                if out.len() >= limit {
                    break;
                }
                out.push(media_ref);
            }
        }
        Value::Array(items) => {
            for item in items {
                if out.len() >= limit {
                    break;
                }
                extract_media_refs_from_value(item, out, limit);
            }
        }
        Value::Object(object) => {
            for key in ["image_path", "image", "image_url", "url", "path"] {
                if let Some(value) = object.get(key) {
                    extract_media_refs_from_value(value, out, limit);
                }
            }
            for value in object.values() {
                if out.len() >= limit {
                    break;
                }
                extract_media_refs_from_value(value, out, limit);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

pub(super) fn extract_media_refs_from_provider_message(
    message: &garyx_models::provider::ProviderMessage,
) -> Vec<OutboundMediaRef> {
    if let Some(image) = extract_image_generation_result(message) {
        return vec![OutboundMediaRef::InlineImage {
            id: image.id.clone(),
            file_name: image.file_name(),
            bytes: image.bytes,
        }];
    }

    let mut refs = Vec::new();
    if let Some(text) = message.text.as_deref() {
        extract_media_refs_from_value(&Value::String(text.to_owned()), &mut refs, 4);
    }
    extract_media_refs_from_value(&message.content, &mut refs, 4);
    refs
}

pub(super) async fn load_media_bytes(
    http: &Client,
    media_ref: &OutboundMediaRef,
) -> Result<Vec<u8>, ChannelError> {
    match media_ref {
        OutboundMediaRef::RemoteUrl(url) => download_remote_bytes(http, url).await,
        OutboundMediaRef::LocalPath(path) => fs::read(path).await.map_err(|error| {
            ChannelError::SendFailed(format!("Weixin local media read failed ({path}): {error}"))
        }),
        OutboundMediaRef::InlineImage { bytes, .. } => Ok(bytes.clone()),
    }
}

pub(super) fn classify_media_type_from_url(url: &str) -> i64 {
    let lower = url
        .split('?')
        .next()
        .unwrap_or(url)
        .split('#')
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if lower.ends_with(".mp4")
        || lower.ends_with(".mov")
        || lower.ends_with(".mkv")
        || lower.ends_with(".webm")
    {
        return 2;
    }
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
    {
        return 1;
    }
    3
}
