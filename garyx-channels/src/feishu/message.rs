use serde_json::Value;

/// Extract plain text from a Feishu message content JSON string.
pub fn extract_message_text(message_type: &str, content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }

    let parsed: Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => {
            return if message_type == "text" {
                content.to_string()
            } else {
                placeholder_for_message_type(message_type).to_string()
            };
        }
    };

    match message_type {
        "text" => parsed
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),

        "post" => extract_post_text(&parsed),

        _ => placeholder_for_message_type(message_type).to_string(),
    }
}

fn extract_post_text(parsed: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(title) = parsed.get("title").and_then(|v| v.as_str()) {
        let trimmed = title.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }

    if let Some(blocks) = parsed.get("content").and_then(|v| v.as_array()) {
        for paragraph in blocks {
            if let Some(items) = paragraph.as_array() {
                for item in items {
                    if let Some(obj) = item.as_object() {
                        let tag = obj.get("tag").and_then(|v| v.as_str()).unwrap_or("");
                        match tag {
                            "text" | "a" => {
                                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                                    parts.push(text.to_string());
                                }
                            }
                            "at" => {
                                if let Some(name) = obj.get("user_name").and_then(|v| v.as_str()) {
                                    if !name.is_empty() {
                                        parts.push(format!("@{name}"));
                                    }
                                }
                            }
                            "img" => {
                                parts.push("<media:image>".to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let text = parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if text.is_empty() {
        "[rich post]".to_string()
    } else {
        text
    }
}

fn placeholder_for_message_type(message_type: &str) -> &str {
    match message_type {
        "image" => "<media:image>",
        "file" => "<media:file>",
        "audio" => "<media:audio>",
        "video" => "<media:video>",
        "sticker" => "<media:sticker>",
        _ => "",
    }
}

/// Extract image keys from a Feishu message content JSON.
///
/// For "image" messages, extracts `image_key` from the content.
/// For "post" messages, extracts all `img` element keys from rich text.
pub fn extract_image_keys(message_type: &str, content: &str) -> Vec<String> {
    let parsed: Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    match message_type {
        "image" => {
            if let Some(key) = parsed.get("image_key").and_then(|v| v.as_str()) {
                if !key.is_empty() {
                    return vec![key.to_owned()];
                }
            }
            Vec::new()
        }
        "post" => {
            let mut keys = Vec::new();
            if let Some(blocks) = parsed.get("content").and_then(|v| v.as_array()) {
                for paragraph in blocks {
                    if let Some(items) = paragraph.as_array() {
                        for item in items {
                            if let Some(obj) = item.as_object() {
                                if obj.get("tag").and_then(|v| v.as_str()) == Some("img") {
                                    if let Some(key) = obj.get("image_key").and_then(|v| v.as_str())
                                    {
                                        if !key.is_empty() {
                                            keys.push(key.to_owned());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            keys
        }
        _ => Vec::new(),
    }
}

/// Build a Feishu interactive card content JSON string from markdown text.
/// Uses schema 2.0 for proper markdown rendering (tables, code blocks, etc.)
pub fn build_card_content(text: &str) -> String {
    let card = serde_json::json!({
        "schema": "2.0",
        "config": { "width_mode": "fill" },
        "body": {
            "elements": [
                {
                    "tag": "markdown",
                    "content": text,
                    "element_id": "content"
                }
            ]
        }
    });
    card.to_string()
}

/// Build a Card Kit card JSON body for streaming (schema 2.0, streaming_mode enabled).
pub fn build_streaming_card_body(initial_text: &str) -> String {
    let card = serde_json::json!({
        "schema": "2.0",
        "config": {
            "width_mode": "fill",
            "streaming_mode": true,
            "summary": { "content": "[生成中...]" },
            "streaming_config": {
                "print_frequency_ms": { "default": 50 },
                "print_step": { "default": 1 }
            }
        },
        "body": {
            "elements": [
                {
                    "tag": "markdown",
                    "content": initial_text,
                    "element_id": "content"
                }
            ]
        }
    });
    card.to_string()
}

/// Build a simple text content JSON string.
pub fn build_text_content(text: &str) -> String {
    serde_json::json!({ "text": text }).to_string()
}

pub(super) fn merge_stream_text(existing: &str, incoming: &str) -> String {
    crate::streaming_core::merge_stream_text(existing, incoming)
}
