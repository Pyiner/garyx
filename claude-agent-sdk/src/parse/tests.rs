use super::*;
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
        "result": "Done!"
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
