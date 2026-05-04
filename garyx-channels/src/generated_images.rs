use std::path::PathBuf;

use base64::{Engine as _, engine::general_purpose};
use garyx_models::provider::ProviderMessage;

#[derive(Debug, Clone)]
pub struct GeneratedImageResult {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
    pub id: String,
    pub media_type: Option<String>,
}

impl GeneratedImageResult {
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.id, self.extension)
    }
}

pub fn provider_message_item_type(message: &ProviderMessage) -> Option<&str> {
    message
        .metadata
        .get("item_type")
        .and_then(|value| value.as_str())
        .or_else(|| message.content.get("type").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub fn generated_image_extension(result: &str, content_type: Option<&str>) -> &'static str {
    if let Some(content_type) = content_type {
        let lower = content_type.trim().to_ascii_lowercase();
        if lower.contains("jpeg") || lower.contains("jpg") {
            return "jpg";
        }
        if lower.contains("webp") {
            return "webp";
        }
        if lower.contains("gif") {
            return "gif";
        }
    }

    if let Some(prefix) = result
        .strip_prefix("data:")
        .and_then(|value| value.split_once(';').map(|(prefix, _)| prefix))
    {
        let lower = prefix.trim().to_ascii_lowercase();
        if lower.contains("jpeg") || lower.contains("jpg") {
            return "jpg";
        }
        if lower.contains("webp") {
            return "webp";
        }
        if lower.contains("gif") {
            return "gif";
        }
    }

    "png"
}

fn generated_image_media_type(result: &str, content_type: Option<&str>) -> Option<String> {
    if let Some(content_type) = content_type {
        let trimmed = content_type.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }

    result
        .strip_prefix("data:")
        .and_then(|value| value.split_once(';').map(|(prefix, _)| prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn extract_image_generation_result(message: &ProviderMessage) -> Option<GeneratedImageResult> {
    if provider_message_item_type(message) != Some("imageGeneration") {
        return None;
    }

    let result = message
        .content
        .get("result")
        .and_then(|value| value.as_str())?;
    let result = result.trim();
    if result.is_empty() {
        return None;
    }

    let encoded = result
        .split_once(',')
        .filter(|(prefix, _)| prefix.trim_start().starts_with("data:"))
        .map(|(_, payload)| payload)
        .unwrap_or(result)
        .trim();
    let bytes = general_purpose::STANDARD.decode(encoded).ok()?;
    if bytes.is_empty() {
        return None;
    }

    let content_type = message
        .content
        .get("media_type")
        .or_else(|| message.content.get("mime_type"))
        .or_else(|| message.content.get("contentType"))
        .and_then(|value| value.as_str());
    let extension = generated_image_extension(result, content_type);
    let media_type = generated_image_media_type(result, content_type);
    let raw_id = message
        .content
        .get("id")
        .and_then(|value| value.as_str())
        .or(message.tool_use_id.as_deref())
        .unwrap_or("image-generation");
    let mut id = crate::sanitize_filename(raw_id);
    if id == "file.bin" {
        id = "image-generation".to_owned();
    }

    Some(GeneratedImageResult {
        bytes,
        extension,
        id,
        media_type,
    })
}

pub async fn write_generated_image_temp(
    channel: &str,
    image: &GeneratedImageResult,
) -> std::io::Result<PathBuf> {
    let image_dir = std::env::temp_dir()
        .join(format!("garyx-{channel}"))
        .join("generated-images");
    tokio::fs::create_dir_all(&image_dir).await?;

    let image_path = image_dir.join(format!(
        "{}-{}.{}",
        image.id,
        uuid::Uuid::new_v4(),
        image.extension
    ));
    tokio::fs::write(&image_path, &image.bytes).await?;
    Ok(image_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use garyx_models::provider::ProviderMessage;
    use serde_json::json;

    #[test]
    fn extracts_data_url_image_generation_result() {
        let message = ProviderMessage::tool_result(
            json!({
                "type": "imageGeneration",
                "id": "img:test",
                "result": "data:image/webp;base64,aGVsbG8="
            }),
            None,
            Some("imageGeneration".to_owned()),
            Some(false),
        )
        .with_metadata_value("item_type", json!("imageGeneration"));

        let image = extract_image_generation_result(&message).expect("image");
        assert_eq!(image.bytes, b"hello");
        assert_eq!(image.extension, "webp");
        assert_eq!(image.media_type.as_deref(), Some("image/webp"));
        assert_eq!(image.id, "img_test");
    }

    #[test]
    fn extracts_raw_base64_image_generation_result() {
        let message = ProviderMessage::tool_result(
            json!({
                "type": "imageGeneration",
                "id": "img_raw",
                "media_type": "image/jpeg",
                "result": "aGVsbG8="
            }),
            None,
            Some("imageGeneration".to_owned()),
            Some(false),
        );

        let image = extract_image_generation_result(&message).expect("image");
        assert_eq!(image.bytes, b"hello");
        assert_eq!(image.extension, "jpg");
        assert_eq!(image.media_type.as_deref(), Some("image/jpeg"));
        assert_eq!(image.id, "img_raw");
    }

    #[test]
    fn infers_png_when_media_type_is_missing() {
        assert_eq!(generated_image_extension("aGVsbG8=", None), "png");
    }
}
