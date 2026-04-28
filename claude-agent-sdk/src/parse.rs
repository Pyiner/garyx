use serde_json::Value;

use crate::error::{ClaudeSDKError, Result};
use crate::types::{
    AssistantMessage, ContentBlock, ImageBlock, ImageSource, Message, ResultMessage, StreamEvent,
    SystemMessage, TextBlock, ThinkingBlock, ToolResultBlock, ToolUseBlock, UserContent,
    UserMessage,
};

/// Parse a raw JSON value (from CLI JSONL output) into a typed [`Message`].
pub fn parse_message(data: &Value) -> Result<Message> {
    let obj = data
        .as_object()
        .ok_or_else(|| ClaudeSDKError::MessageParse {
            message: format!(
                "Invalid message data type (expected object, got {})",
                json_type_name(data)
            ),
            data: Some(data.clone()),
        })?;

    let msg_type =
        obj.get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ClaudeSDKError::MessageParse {
                message: "Message missing 'type' field".into(),
                data: Some(data.clone()),
            })?;

    match msg_type {
        "user" => parse_user_message(data),
        "assistant" => parse_assistant_message(data),
        "system" => parse_system_message(data),
        "result" => parse_result_message(data),
        "stream_event" => parse_stream_event(data),
        // Pass through any unrecognized types (e.g. rate_limit_event) as
        // System messages so consumers can handle or ignore them without
        // triggering errors.
        other => Ok(Message::System(SystemMessage {
            subtype: other.to_string(),
            data: data.clone(),
        })),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn parse_content_blocks(blocks: &[Value]) -> Result<Vec<ContentBlock>> {
    blocks.iter().map(parse_content_block).collect()
}

fn parse_content_block(block: &Value) -> Result<ContentBlock> {
    let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match block_type {
        "text" => Ok(ContentBlock::Text(TextBlock {
            text: block
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })),
        "image" => {
            let source = block.get("source");
            Ok(ContentBlock::Image(ImageBlock {
                source: ImageSource {
                    source_type: source
                        .and_then(|s| s.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("base64")
                        .to_string(),
                    media_type: source
                        .and_then(|s| s.get("media_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("image/jpeg")
                        .to_string(),
                    data: source
                        .and_then(|s| s.get("data"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                },
            }))
        }
        "thinking" => Ok(ContentBlock::Thinking(ThinkingBlock {
            thinking: block
                .get("thinking")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            signature: block
                .get("signature")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })),
        "tool_use" => Ok(ContentBlock::ToolUse(ToolUseBlock {
            id: block
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            name: block
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            input: block
                .get("input")
                .cloned()
                .unwrap_or(Value::Object(Default::default())),
        })),
        "tool_result" => Ok(ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: block
                .get("tool_use_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            content: block.get("content").cloned(),
            is_error: block.get("is_error").and_then(|v| v.as_bool()),
        })),
        other => Err(ClaudeSDKError::MessageParse {
            message: format!("Unknown content block type: {other}"),
            data: Some(block.clone()),
        }),
    }
}

fn parse_user_message(data: &Value) -> Result<Message> {
    let uuid = data.get("uuid").and_then(|v| v.as_str()).map(String::from);
    let parent_tool_use_id = data
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let tool_use_result = data
        .get("tool_use_result")
        .cloned()
        .filter(|v| !v.is_null());

    let message = data
        .get("message")
        .ok_or_else(|| ClaudeSDKError::MessageParse {
            message: "Missing 'message' field in user message".into(),
            data: Some(data.clone()),
        })?;

    let content_val = message
        .get("content")
        .ok_or_else(|| ClaudeSDKError::MessageParse {
            message: "Missing 'content' in user message.message".into(),
            data: Some(data.clone()),
        })?;

    let content = if let Some(text) = content_val.as_str() {
        UserContent::Text(text.to_string())
    } else if let Some(arr) = content_val.as_array() {
        UserContent::Blocks(parse_content_blocks(arr)?)
    } else {
        UserContent::Text(content_val.to_string())
    };

    Ok(Message::User(UserMessage {
        content,
        uuid,
        parent_tool_use_id,
        tool_use_result,
    }))
}

fn parse_assistant_message(data: &Value) -> Result<Message> {
    let message = data
        .get("message")
        .ok_or_else(|| ClaudeSDKError::MessageParse {
            message: "Missing 'message' field in assistant message".into(),
            data: Some(data.clone()),
        })?;

    let content_arr = message
        .get("content")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ClaudeSDKError::MessageParse {
            message: "Missing 'content' array in assistant message".into(),
            data: Some(data.clone()),
        })?;

    let content = parse_content_blocks(content_arr)?;
    let model = message
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let parent_tool_use_id = data
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let error = message
        .get("error")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    Ok(Message::Assistant(AssistantMessage {
        content,
        model,
        parent_tool_use_id,
        error,
    }))
}

fn parse_system_message(data: &Value) -> Result<Message> {
    let subtype = data
        .get("subtype")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ClaudeSDKError::MessageParse {
            message: "Missing 'subtype' in system message".into(),
            data: Some(data.clone()),
        })?
        .to_string();

    Ok(Message::System(SystemMessage {
        subtype,
        data: data.clone(),
    }))
}

fn parse_result_message(data: &Value) -> Result<Message> {
    let subtype = str_field(data, "subtype")?;
    let session_id = str_field(data, "session_id")?;

    Ok(Message::Result(ResultMessage {
        subtype,
        duration_ms: data
            .get("duration_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        duration_api_ms: data
            .get("duration_api_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        is_error: data
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        num_turns: data.get("num_turns").and_then(|v| v.as_i64()).unwrap_or(0),
        session_id,
        total_cost_usd: data.get("total_cost_usd").and_then(|v| v.as_f64()),
        usage: data
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
        result: data
            .get("result")
            .and_then(|v| v.as_str())
            .map(String::from),
        structured_output: data
            .get("structured_output")
            .cloned()
            .filter(|v| !v.is_null()),
    }))
}

fn parse_stream_event(data: &Value) -> Result<Message> {
    let uuid = str_field(data, "uuid")?;
    let session_id = str_field(data, "session_id")?;
    let event = data
        .get("event")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let parent_tool_use_id = data
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(Message::StreamEvent(StreamEvent {
        uuid,
        session_id,
        event,
        parent_tool_use_id,
    }))
}

fn str_field(data: &Value, field: &str) -> Result<String> {
    data.get(field)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| ClaudeSDKError::MessageParse {
            message: format!("Missing required field '{field}'"),
            data: Some(data.clone()),
        })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
