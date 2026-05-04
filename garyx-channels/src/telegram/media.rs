use std::path::Path;

use reqwest::Client;
use tokio::fs;
use tracing::warn;

use garyx_models::provider::{PromptAttachment, PromptAttachmentKind};

use crate::channel_trait::ChannelError;

use super::{MAX_IMAGE_SIZE_BYTES, TgDocument, TgFile, TgMessage, TgResponse};

fn is_supported_image_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

fn media_type_from_extension(path_like: &str) -> Option<&'static str> {
    let ext = Path::new(path_like)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("png") => Some("image/png"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

pub(super) fn resolve_document_image_media_type(doc: &TgDocument) -> Option<String> {
    if let Some(mime) = doc.mime_type.as_deref() {
        let lower = mime.to_ascii_lowercase();
        if is_supported_image_media_type(&lower) {
            return Some(lower);
        }
    }

    doc.file_name
        .as_deref()
        .and_then(media_type_from_extension)
        .map(str::to_owned)
}

async fn fetch_telegram_file_metadata(
    http: &Client,
    token: &str,
    file_id: &str,
    api_base: &str,
) -> Result<TgFile, ChannelError> {
    let encoded_file_id = urlencoding::encode(file_id);
    let url = format!("{api_base}/bot{token}/getFile?file_id={encoded_file_id}");
    let resp: TgResponse<TgFile> = http
        .get(&url)
        .send()
        .await
        .map_err(|e| ChannelError::Connection(format!("getFile request failed: {e}")))?
        .json()
        .await
        .map_err(|e| ChannelError::Connection(format!("getFile parse failed: {e}")))?;

    if !resp.ok {
        return Err(ChannelError::Connection(format!(
            "getFile failed: {}",
            resp.description.unwrap_or_default()
        )));
    }

    resp.result
        .ok_or_else(|| ChannelError::Connection("getFile succeeded but result is empty".to_owned()))
}

async fn download_telegram_file_bytes(
    http: &Client,
    token: &str,
    file_path: &str,
    api_base: &str,
) -> Result<Vec<u8>, ChannelError> {
    let trimmed_path = file_path.trim_start_matches('/');
    if trimmed_path.is_empty() {
        return Err(ChannelError::Connection(
            "empty file_path returned by getFile".to_owned(),
        ));
    }

    let url = format!("{api_base}/file/bot{token}/{trimmed_path}");
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| ChannelError::Connection(format!("file download failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ChannelError::Connection(format!(
            "file download failed with status {}",
            resp.status()
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ChannelError::Connection(format!("file download parse failed: {e}")))?;
    Ok(bytes.to_vec())
}

async fn build_image_attachment_from_file(
    http: &Client,
    token: &str,
    file_id: &str,
    hinted_media_type: Option<String>,
    api_base: &str,
) -> Option<PromptAttachment> {
    let file = match fetch_telegram_file_metadata(http, token, file_id, api_base).await {
        Ok(file) => file,
        Err(e) => {
            warn!(file_id, error = %e, "failed to fetch Telegram file metadata");
            return None;
        }
    };

    if let Some(size) = file.file_size
        && size as usize > MAX_IMAGE_SIZE_BYTES
    {
        warn!(
            file_id = %file.file_id,
            file_size = size,
            "skipping oversized image payload"
        );
        return None;
    }

    let file_path = match file.file_path.as_deref() {
        Some(path) if !path.trim().is_empty() => path,
        _ => {
            warn!(file_id = %file.file_id, "missing file_path in Telegram getFile response");
            return None;
        }
    };

    let media_type = hinted_media_type
        .or_else(|| media_type_from_extension(file_path).map(str::to_owned))
        .unwrap_or_else(|| "image/jpeg".to_owned());
    if !is_supported_image_media_type(&media_type) {
        warn!(
            file_id = %file.file_id,
            media_type = %media_type,
            "unsupported image media type from Telegram payload"
        );
        return None;
    }

    let bytes = match download_telegram_file_bytes(http, token, file_path, api_base).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(file_id = %file.file_id, error = %e, "failed to download Telegram file");
            return None;
        }
    };

    if bytes.is_empty() {
        warn!(file_id = %file.file_id, "skipping empty Telegram file payload");
        return None;
    }
    if bytes.len() > MAX_IMAGE_SIZE_BYTES {
        warn!(
            file_id = %file.file_id,
            file_size = bytes.len(),
            "skipping oversized Telegram image payload"
        );
        return None;
    }

    let base_dir = std::env::temp_dir().join("garyx-telegram").join("inbound");
    if fs::create_dir_all(&base_dir).await.is_err() {
        warn!("failed to create telegram inbound temp dir for image download");
        return None;
    }

    let name = Path::new(file_path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(sanitize_filename)
        .unwrap_or_else(|| {
            sanitize_filename(&format!(
                "telegram-image.{}",
                media_type.rsplit('/').next().unwrap_or("jpg")
            ))
        });
    let local_path = base_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), name));
    if fs::write(&local_path, &bytes).await.is_err() {
        warn!("failed to write telegram image to disk");
        return None;
    }

    Some(PromptAttachment {
        kind: PromptAttachmentKind::Image,
        path: local_path.to_string_lossy().to_string(),
        name,
        media_type,
    })
}

/// Maximum non-image file size for download-to-disk (50 MB).
const MAX_FILE_DOWNLOAD_BYTES: usize = 50 * 1024 * 1024;

fn sanitize_filename(name: &str) -> String {
    crate::sanitize_filename(name)
}

/// Download a Telegram file to a local temp directory and return its path.
///
/// Used for non-image attachments (documents, voice, audio, video) so
/// the agent thread can reference the local file path directly.
pub(super) async fn download_file_to_disk(
    http: &Client,
    token: &str,
    file_id: &str,
    suggested_name: Option<&str>,
    api_base: &str,
) -> Option<String> {
    let file = match fetch_telegram_file_metadata(http, token, file_id, api_base).await {
        Ok(file) => file,
        Err(e) => {
            warn!(file_id, error = %e, "failed to fetch Telegram file metadata for disk download");
            return None;
        }
    };

    if let Some(size) = file.file_size
        && size as usize > MAX_FILE_DOWNLOAD_BYTES
    {
        warn!(
            file_id = %file.file_id,
            file_size = size,
            "skipping oversized file for disk download"
        );
        return None;
    }

    let remote_path = match file.file_path.as_deref() {
        Some(path) if !path.trim().is_empty() => path,
        _ => {
            warn!(file_id = %file.file_id, "missing file_path in Telegram getFile response");
            return None;
        }
    };

    let bytes = match download_telegram_file_bytes(http, token, remote_path, api_base).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(file_id = %file.file_id, error = %e, "failed to download Telegram file to disk");
            return None;
        }
    };

    if bytes.is_empty() {
        warn!(file_id = %file.file_id, "skipping empty Telegram file for disk download");
        return None;
    }

    let base_dir = std::env::temp_dir().join("garyx-telegram").join("inbound");
    if fs::create_dir_all(&base_dir).await.is_err() {
        warn!("failed to create telegram inbound temp dir");
        return None;
    }

    let stem = suggested_name
        .map(sanitize_filename)
        .or_else(|| {
            Path::new(remote_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(sanitize_filename)
        })
        .unwrap_or_else(|| "file.bin".to_owned());

    let local_path = base_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), stem));
    if fs::write(&local_path, &bytes).await.is_err() {
        warn!("failed to write telegram file to disk");
        return None;
    }

    Some(local_path.to_string_lossy().to_string())
}

/// Extract local file paths for non-image media attachments in a Telegram message.
///
/// Downloads documents (non-image), voice messages, audio files, and video
/// files to a local temp directory so the agent can access them by path.
pub(super) async fn extract_file_paths(
    http: &Client,
    token: &str,
    msg: &TgMessage,
    api_base: &str,
) -> Vec<String> {
    let mut paths = Vec::new();

    // Non-image documents
    if let Some(doc) = &msg.document {
        // Only handle non-image documents here; images are handled by extract_image_attachments
        if resolve_document_image_media_type(doc).is_none()
            && let Some(path) = download_file_to_disk(
                http,
                token,
                &doc.file_id,
                doc.file_name.as_deref(),
                api_base,
            )
            .await
        {
            paths.push(path);
        }
    }

    // Voice messages
    if let Some(voice) = &msg.voice
        && let Some(path) =
            download_file_to_disk(http, token, &voice.file_id, Some("voice.ogg"), api_base).await
    {
        paths.push(path);
    }

    // Audio files
    if let Some(audio) = &msg.audio {
        let name = audio.title.as_deref().unwrap_or("audio.mp3");
        if let Some(path) =
            download_file_to_disk(http, token, &audio.file_id, Some(name), api_base).await
        {
            paths.push(path);
        }
    }

    // Video files
    if let Some(video) = &msg.video
        && let Some(path) =
            download_file_to_disk(http, token, &video.file_id, Some("video.mp4"), api_base).await
    {
        paths.push(path);
    }

    paths
}

pub(super) async fn extract_image_attachments(
    http: &Client,
    token: &str,
    msg: &TgMessage,
    api_base: &str,
) -> Vec<PromptAttachment> {
    let mut images = Vec::new();

    if let Some(photo_sizes) = &msg.photo
        && let Some(photo) = photo_sizes.last()
        && let Some(payload) =
            build_image_attachment_from_file(http, token, &photo.file_id, None, api_base).await
    {
        images.push(payload);
    }

    if let Some(document) = &msg.document
        && let Some(media_type) = resolve_document_image_media_type(document)
        && let Some(payload) = build_image_attachment_from_file(
            http,
            token,
            &document.file_id,
            Some(media_type),
            api_base,
        )
        .await
    {
        images.push(payload);
    }

    images
}
