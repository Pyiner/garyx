use super::*;
use crate::types::AssistantMessageError;
use serde_json::json;

#[test]
fn test_parse_user_message_string() {
    let data = json!({
        "type": "user",
        "message": { "role": "user", "content": "Hello!" },
        "uuid": "abc-123",
        "parent_tool_use_id": null
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::User(u) => {
            assert_eq!(u.content, UserContent::Text("Hello!".into()));
            assert_eq!(u.uuid.as_deref(), Some("abc-123"));
        }
        other => panic!("Expected User, got {other:?}"),
    }
}

#[test]
fn test_parse_user_message_preserves_task_notification_origin() {
    let data = json!({
        "type": "user",
        "message": { "role": "user", "content": "task completed" },
        "origin": {
            "kind": "task-notification",
            "overageInUse": true
        }
    });

    let msg = parse_message(&data).unwrap();
    let Message::User(user) = msg else {
        panic!("expected user message");
    };
    let origin = user.origin.expect("origin should be preserved");
    assert!(origin.is_task_notification());
    assert_eq!(origin.metadata.get("overageInUse"), Some(&json!(true)));
}

#[test]
fn test_parse_user_message_blocks() {
    let data = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [
                { "type": "text", "text": "hello" },
                { "type": "tool_result", "tool_use_id": "tu-1", "content": "ok" }
            ]
        }
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::User(u) => {
            if let UserContent::Blocks(blocks) = &u.content {
                assert_eq!(blocks.len(), 2);
            } else {
                panic!("Expected blocks");
            }
        }
        other => panic!("Expected User, got {other:?}"),
    }
}

#[test]
fn test_parse_user_message_blocks_with_image() {
    let data = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": "abc123=="
                    }
                },
                { "type": "text", "text": "what is this?" }
            ]
        }
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::User(u) => match &u.content {
            UserContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                match &blocks[0] {
                    ContentBlock::Image(image) => {
                        assert_eq!(image.source.source_type, "base64");
                        assert_eq!(image.source.media_type, "image/png");
                        assert_eq!(image.source.data, "abc123==");
                    }
                    other => panic!("Expected Image block, got {other:?}"),
                }
                match &blocks[1] {
                    ContentBlock::Text(text) => assert_eq!(text.text, "what is this?"),
                    other => panic!("Expected Text block, got {other:?}"),
                }
            }
            other => panic!("Expected block content, got {other:?}"),
        },
        other => panic!("Expected User, got {other:?}"),
    }
}

#[test]
fn test_parse_document_content_block() {
    let block = json!({
        "type": "document",
        "source": {
            "type": "base64",
            "media_type": "application/pdf",
            "data": "JVBERi0xLjQK"
        },
        "title": "sample.pdf",
        "context": "Synthetic fixture metadata",
        "citations": { "enabled": true }
    });

    let parsed = parse_content_block(&block).expect("document block should parse");
    match parsed {
        ContentBlock::Document(document) => {
            assert_eq!(document.source.source_type, "base64");
            assert_eq!(
                document.source.media_type.as_deref(),
                Some("application/pdf")
            );
            assert_eq!(document.source.data.as_deref(), Some("JVBERi0xLjQK"));
            assert_eq!(document.title.as_deref(), Some("sample.pdf"));
            assert_eq!(
                document.context.as_deref(),
                Some("Synthetic fixture metadata")
            );
            assert_eq!(document.citations, Some(json!({ "enabled": true })));
        }
        other => panic!("Expected Document block, got {other:?}"),
    }
}

#[test]
fn test_parse_assistant_message() {
    let data = json!({
        "type": "assistant",
        "message": {
            "role": "assistant",
            "model": "claude-opus-4-6",
            "content": [
                { "type": "text", "text": "Hello there!" },
                { "type": "thinking", "thinking": "Let me think...", "signature": "sig123" },
                { "type": "tool_use", "id": "tu-1", "name": "Bash", "input": { "command": "ls" } }
            ]
        },
        "parent_tool_use_id": null
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::Assistant(a) => {
            assert_eq!(a.model, "claude-opus-4-6");
            assert_eq!(a.content.len(), 3);
            match &a.content[0] {
                ContentBlock::Text(t) => assert_eq!(t.text, "Hello there!"),
                other => panic!("Expected Text, got {other:?}"),
            }
            match &a.content[1] {
                ContentBlock::Thinking(t) => {
                    assert_eq!(t.thinking, "Let me think...");
                    assert_eq!(t.signature, "sig123");
                }
                other => panic!("Expected Thinking, got {other:?}"),
            }
            match &a.content[2] {
                ContentBlock::ToolUse(t) => {
                    assert_eq!(t.id, "tu-1");
                    assert_eq!(t.name, "Bash");
                }
                other => panic!("Expected ToolUse, got {other:?}"),
            }
        }
        other => panic!("Expected Assistant, got {other:?}"),
    }
}

#[test]
fn test_parse_unknown_content_block_as_raw_block() {
    let data = json!({
        "type": "assistant",
        "message": {
            "role": "assistant",
            "model": "claude-opus-4-6",
            "content": [
                { "type": "future_block", "payload": { "ok": true } }
            ]
        }
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::Assistant(a) => match &a.content[0] {
            ContentBlock::Unknown(block) => {
                assert_eq!(block.block_type, "future_block");
                assert_eq!(block.data["payload"]["ok"], true);
            }
            other => panic!("Expected Unknown block, got {other:?}"),
        },
        other => panic!("Expected Assistant, got {other:?}"),
    }
}

#[test]
fn test_parse_system_message() {
    let data = json!({
        "type": "system",
        "subtype": "init",
        "session_id": "sess-1"
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::System(s) => {
            assert_eq!(s.subtype, "init");
            assert!(s.data.get("session_id").is_some());
        }
        other => panic!("Expected System, got {other:?}"),
    }
}

#[test]
fn test_parse_result_message() {
    let data = json!({
        "type": "result",
        "subtype": "success",
        "duration_ms": 1234,
        "duration_api_ms": 1000,
        "is_error": false,
        "num_turns": 3,
        "session_id": "sess-1",
        "total_cost_usd": 0.05,
        "result": "Done!",
        "origin": { "kind": "human" }
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::Result(r) => {
            assert_eq!(r.subtype, "success");
            assert_eq!(r.duration_ms, 1234);
            assert_eq!(r.duration_api_ms, 1000);
            assert!(!r.is_error);
            assert_eq!(r.num_turns, 3);
            assert_eq!(r.session_id, "sess-1");
            assert_eq!(r.total_cost_usd, Some(0.05));
            assert_eq!(r.result.as_deref(), Some("Done!"));
            assert_eq!(
                r.origin.as_ref().map(|origin| origin.kind.as_str()),
                Some("human")
            );
        }
        other => panic!("Expected Result, got {other:?}"),
    }
}

#[test]
fn test_parse_stream_event() {
    let data = json!({
        "type": "stream_event",
        "uuid": "ev-1",
        "session_id": "sess-1",
        "event": { "type": "content_block_delta" },
        "parent_tool_use_id": "tu-2"
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::StreamEvent(e) => {
            assert_eq!(e.uuid, "ev-1");
            assert_eq!(e.session_id, "sess-1");
            assert_eq!(e.parent_tool_use_id.as_deref(), Some("tu-2"));
        }
        other => panic!("Expected StreamEvent, got {other:?}"),
    }
}

#[test]
fn test_parse_unknown_type_as_system() {
    let data = json!({ "type": "foobar", "extra": 42 });
    let msg = parse_message(&data).unwrap();
    match msg {
        Message::System(s) => {
            assert_eq!(s.subtype, "foobar");
            assert_eq!(s.data["extra"], 42);
        }
        other => panic!("Expected System, got {other:?}"),
    }
}

#[test]
fn test_parse_rate_limit_event() {
    let data = json!({
        "type": "rate_limit_event",
        "rate_limit_info": { "status": "allowed" }
    });
    let msg = parse_message(&data).unwrap();
    match msg {
        Message::System(s) => assert_eq!(s.subtype, "rate_limit_event"),
        other => panic!("Expected System, got {other:?}"),
    }
}

#[test]
fn test_parse_missing_type() {
    let data = json!({ "subtype": "init" });
    assert!(parse_message(&data).is_err());
}

#[test]
fn test_parse_non_object() {
    let data = json!("hello");
    assert!(parse_message(&data).is_err());
}

#[test]
fn test_parse_result_message_terminal_state_fields() {
    let data = json!({
        "type": "result",
        "subtype": "error_during_execution",
        "duration_ms": 10,
        "duration_api_ms": 5,
        "is_error": true,
        "num_turns": 1,
        "session_id": "sess-2",
        "stop_reason": "max_tokens",
        "terminal_reason": "blocking_limit",
        "api_error_status": 429,
        "errors": ["usage limit reached", "", "   "],
        "permission_denials": [
            { "tool_name": "Bash", "tool_use_id": "toolu_1", "tool_input": {} }
        ],
        "modelUsage": {
            "claude-x": { "inputTokens": 10, "outputTokens": 20, "contextWindow": 200000 }
        }
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::Result(r) => {
            assert_eq!(r.stop_reason.as_deref(), Some("max_tokens"));
            assert_eq!(r.terminal_reason.as_deref(), Some("blocking_limit"));
            assert_eq!(r.api_error_status, Some(429));
            // Blank error strings are dropped.
            assert_eq!(r.errors, vec!["usage limit reached".to_owned()]);
            assert_eq!(r.permission_denials.len(), 1);
            assert_eq!(
                r.permission_denials[0]
                    .get("tool_name")
                    .and_then(|v| v.as_str()),
                Some("Bash")
            );
            let model_usage = r.model_usage.expect("modelUsage should parse");
            assert_eq!(
                model_usage
                    .get("claude-x")
                    .and_then(|usage| usage.get("contextWindow"))
                    .and_then(|v| v.as_i64()),
                Some(200000)
            );
        }
        other => panic!("Expected Result, got {other:?}"),
    }
}

#[test]
fn test_parse_result_message_new_fields_absent_defaults() {
    let data = json!({
        "type": "result",
        "subtype": "success",
        "duration_ms": 1,
        "duration_api_ms": 1,
        "is_error": false,
        "num_turns": 1,
        "session_id": "sess-3"
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::Result(r) => {
            assert_eq!(r.stop_reason, None);
            assert_eq!(r.terminal_reason, None);
            assert_eq!(r.api_error_status, None);
            assert!(r.errors.is_empty());
            assert!(r.permission_denials.is_empty());
            assert_eq!(r.model_usage, None);
        }
        other => panic!("Expected Result, got {other:?}"),
    }
}

#[test]
fn test_parse_assistant_error_unknown_category_degrades_to_unknown() {
    let data = json!({
        "type": "assistant",
        "message": {
            "content": [{ "type": "text", "text": "hi" }],
            "model": "claude-x"
        },
        "error": "brand_new_error_category_from_future_cli"
    });

    let msg = parse_message(&data).unwrap();
    match msg {
        Message::Assistant(a) => {
            // Unknown categories must degrade to Unknown, not drop the
            // classification entirely.
            assert_eq!(a.error, Some(AssistantMessageError::Unknown));
        }
        other => panic!("Expected Assistant, got {other:?}"),
    }
}
