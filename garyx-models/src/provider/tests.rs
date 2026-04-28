use super::*;

#[test]
fn test_provider_type_serde() {
    let pt = ProviderType::ClaudeCode;
    let json = serde_json::to_string(&pt).unwrap();
    assert_eq!(json, "\"claude_code\"");
    let back: ProviderType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProviderType::ClaudeCode);

    let pt = ProviderType::GeminiCli;
    let json = serde_json::to_string(&pt).unwrap();
    assert_eq!(json, "\"gemini_cli\"");
    let back: ProviderType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProviderType::GeminiCli);

    let pt = ProviderType::AgentTeam;
    let json = serde_json::to_string(&pt).unwrap();
    assert_eq!(json, "\"agent_team\"");
    let back: ProviderType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProviderType::AgentTeam);
}

#[test]
fn test_stream_boundary_kind_serde() {
    let kind = StreamBoundaryKind::UserAck;
    let json = serde_json::to_string(&kind).unwrap();
    assert_eq!(json, "\"user_ack\"");
    let back: StreamBoundaryKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, StreamBoundaryKind::UserAck);

    let kind = StreamBoundaryKind::AssistantSegment;
    let json = serde_json::to_string(&kind).unwrap();
    assert_eq!(json, "\"assistant_segment\"");
    let back: StreamBoundaryKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, StreamBoundaryKind::AssistantSegment);
}

#[test]
fn test_stream_event_serde() {
    let delta = StreamEvent::Delta {
        text: "hello".to_owned(),
    };
    let tool_use = StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            Value::Object(serde_json::Map::new()),
            Some("tool-1".to_owned()),
            Some("shell".to_owned()),
        ),
    };
    let tool_result = StreamEvent::ToolResult {
        message: ProviderMessage::tool_result(
            Value::String("ok".to_owned()),
            Some("tool-1".to_owned()),
            Some("shell".to_owned()),
            Some(false),
        ),
    };
    let boundary = StreamEvent::Boundary {
        kind: StreamBoundaryKind::UserAck,
        pending_input_id: None,
    };
    let boundary_with_input = StreamEvent::Boundary {
        kind: StreamBoundaryKind::UserAck,
        pending_input_id: Some("queued-input-1".to_owned()),
    };
    let done = StreamEvent::Done;

    let delta_json = serde_json::to_string(&delta).unwrap();
    assert_eq!(delta_json, "{\"type\":\"delta\",\"text\":\"hello\"}");
    let tool_use_json = serde_json::to_string(&tool_use).unwrap();
    assert_eq!(
        tool_use_json,
        "{\"type\":\"tool_use\",\"message\":{\"role\":\"tool_use\",\"content\":{},\"tool_use_id\":\"tool-1\",\"tool_name\":\"shell\"}}"
    );
    let tool_result_json = serde_json::to_string(&tool_result).unwrap();
    assert_eq!(
        tool_result_json,
        "{\"type\":\"tool_result\",\"message\":{\"role\":\"tool_result\",\"content\":\"ok\",\"tool_use_id\":\"tool-1\",\"tool_name\":\"shell\",\"is_error\":false}}"
    );
    let boundary_json = serde_json::to_string(&boundary).unwrap();
    assert_eq!(
        boundary_json,
        "{\"type\":\"boundary\",\"kind\":\"user_ack\"}"
    );
    let boundary_with_input_json = serde_json::to_string(&boundary_with_input).unwrap();
    assert_eq!(
        boundary_with_input_json,
        "{\"type\":\"boundary\",\"kind\":\"user_ack\",\"pending_input_id\":\"queued-input-1\"}"
    );
    let done_json = serde_json::to_string(&done).unwrap();
    assert_eq!(done_json, "{\"type\":\"done\"}");

    let delta_back: StreamEvent = serde_json::from_str(&delta_json).unwrap();
    let tool_use_back: StreamEvent = serde_json::from_str(&tool_use_json).unwrap();
    let tool_result_back: StreamEvent = serde_json::from_str(&tool_result_json).unwrap();
    let boundary_back: StreamEvent = serde_json::from_str(&boundary_json).unwrap();
    let boundary_with_input_back: StreamEvent =
        serde_json::from_str(&boundary_with_input_json).unwrap();
    let done_back: StreamEvent = serde_json::from_str(&done_json).unwrap();
    assert_eq!(delta_back, delta);
    assert_eq!(tool_use_back, tool_use);
    assert_eq!(tool_result_back, tool_result);
    assert_eq!(boundary_back, boundary);
    assert_eq!(boundary_with_input_back, boundary_with_input);
    assert_eq!(done_back, done);
}

#[test]
fn test_provider_message_text_helpers() {
    let assistant = ProviderMessage::assistant_text("hello")
        .with_timestamp("2026-03-08T00:00:00Z")
        .with_metadata_value("source", Value::String("claude_sdk".to_owned()));
    assert_eq!(assistant.role, ProviderMessageRole::Assistant);
    assert_eq!(assistant.text.as_deref(), Some("hello"));
    assert_eq!(assistant.content, Value::String("hello".to_owned()));
    assert_eq!(assistant.role_str(), "assistant");
    assert_eq!(
        assistant.metadata.get("source"),
        Some(&Value::String("claude_sdk".to_owned()))
    );

    let roundtrip =
        ProviderMessage::from_value(&assistant.to_json_value()).expect("roundtrip should work");
    assert_eq!(roundtrip, assistant);
}

#[test]
fn test_claude_code_config_defaults() {
    let cfg = ClaudeCodeConfig::default();
    assert_eq!(cfg.provider_type, ProviderType::ClaudeCode);
    assert_eq!(cfg.permission_mode, "bypassPermissions");
    assert_eq!(cfg.mcp_base_url, "http://127.0.0.1:31337");
}

#[test]
fn test_codex_config_defaults() {
    let cfg = CodexAppServerConfig::default();
    assert_eq!(cfg.provider_type, ProviderType::CodexAppServer);
    assert_eq!(cfg.approval_policy, "never");
    assert_eq!(cfg.sandbox_mode, "danger-full-access");
    assert!((cfg.request_timeout_seconds - 300.0).abs() < f64::EPSILON);
}

#[test]
fn test_gemini_config_defaults() {
    let cfg = GeminiCliConfig::default();
    assert_eq!(cfg.provider_type, ProviderType::GeminiCli);
    assert_eq!(cfg.approval_mode, "yolo");
    assert_eq!(cfg.default_model, "");
    assert_eq!(cfg.model, "");
    assert_eq!(cfg.mcp_base_url, "http://127.0.0.1:31337");
}

#[test]
fn test_image_payload_serde_roundtrip() {
    let payload = ImagePayload {
        name: "photo.png".to_owned(),
        data: "abc123==".to_owned(),
        media_type: "image/png".to_owned(),
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["name"], "photo.png");
    assert_eq!(json["data"], "abc123==");
    assert_eq!(json["media_type"], "image/png");
    let back: ImagePayload = serde_json::from_value(json).unwrap();
    assert_eq!(back, payload);
}

#[test]
fn test_run_options_typed_images() {
    let mut opts = ProviderRunOptions {
        thread_id: "thread::s1".to_owned(),
        message: "hi".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    // No images initially
    assert!(opts.images.as_deref().unwrap_or_default().is_empty());

    // Set images
    let payloads = vec![ImagePayload {
        name: "photo.png".to_owned(),
        data: "abc".to_owned(),
        media_type: "image/png".to_owned(),
    }];
    opts.images = Some(payloads.clone());
    assert_eq!(opts.images.as_ref().unwrap().len(), 1);
    assert_eq!(opts.images.as_ref().unwrap()[0], payloads[0]);

    // Clear images
    opts.images = None;
    assert!(opts.images.is_none());
}

/// Regression test: image payloads may now include `name`, but legacy JSON
/// without that field must still deserialize correctly.
#[test]
fn test_run_options_image_serde_compat() {
    // Build options with typed ImagePayload
    let opts = ProviderRunOptions {
        thread_id: "t1".to_owned(),
        message: "msg".to_owned(),
        workspace_dir: None,
        images: Some(vec![ImagePayload {
            name: "photo.png".to_owned(),
            data: "abc==".to_owned(),
            media_type: "image/png".to_owned(),
        }]),
        metadata: HashMap::new(),
    };
    let json = serde_json::to_value(&opts).unwrap();

    let img = &json["images"][0];
    assert_eq!(img["name"], "photo.png");
    assert_eq!(img["data"], "abc==");
    assert_eq!(img["media_type"], "image/png");

    // Verify round-trip
    let back: ProviderRunOptions = serde_json::from_value(json).unwrap();
    assert_eq!(back.images.as_ref().unwrap()[0].name, "photo.png");
    assert_eq!(back.images.as_ref().unwrap()[0].data, "abc==");
    assert_eq!(back.images.as_ref().unwrap()[0].media_type, "image/png");

    // Verify deserialization from old HashMap-style JSON still works
    let legacy_json = serde_json::json!({
        "thread_id": "t2",
        "message": "m",
        "images": [{"data": "xyz==", "media_type": "image/jpeg"}]
    });
    let from_legacy: ProviderRunOptions = serde_json::from_value(legacy_json).unwrap();
    assert_eq!(from_legacy.images.as_ref().unwrap()[0].name, "");
    assert_eq!(from_legacy.images.as_ref().unwrap()[0].data, "xyz==");
}

#[test]
fn build_prompt_message_with_attachments_appends_instructions() {
    let message = build_prompt_message_with_attachments(
        "Please summarize these.",
        &[PromptAttachment {
            kind: PromptAttachmentKind::File,
            path: "/tmp/report.md".to_owned(),
            name: "report.md".to_owned(),
            media_type: "text/markdown".to_owned(),
        }],
    );
    assert!(message.contains("Please summarize these."));
    assert!(message.contains("Read this file from disk: /tmp/report.md"));
}

#[test]
fn build_user_content_from_parts_prefers_structured_attachments() {
    let content = build_user_content_from_parts(
        "Check both",
        &[
            PromptAttachment {
                kind: PromptAttachmentKind::Image,
                path: "/tmp/shot.png".to_owned(),
                name: "shot.png".to_owned(),
                media_type: "image/png".to_owned(),
            },
            PromptAttachment {
                kind: PromptAttachmentKind::File,
                path: "/tmp/report.pdf".to_owned(),
                name: "report.pdf".to_owned(),
                media_type: "application/pdf".to_owned(),
            },
        ],
        &[ImagePayload {
            name: "ignored.png".to_owned(),
            data: "ignored".to_owned(),
            media_type: "image/png".to_owned(),
        }],
    );
    let blocks = content.as_array().unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].get("type").and_then(Value::as_str), Some("text"));
    assert_eq!(blocks[1].get("type").and_then(Value::as_str), Some("image"));
    assert_eq!(blocks[2].get("type").and_then(Value::as_str), Some("file"));
    assert_eq!(
        blocks[1].get("path").and_then(Value::as_str),
        Some("/tmp/shot.png")
    );
}

#[test]
fn stage_image_payloads_for_prompt_preserves_provided_name() {
    let attachments = stage_image_payloads_for_prompt(
        "garyx-models-test",
        &[ImagePayload {
            name: "screen-shot.final.png".to_owned(),
            data: BASE64.encode(b"png"),
            media_type: "image/png".to_owned(),
        }],
    );
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].name, "screen-shot.final.png");
    assert!(attachments[0].path.ends_with("screen-shot.final.png"));
    let _ = std::fs::remove_file(&attachments[0].path);
}
