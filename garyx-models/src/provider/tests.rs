use super::*;

#[test]
fn test_provider_type_serde() {
    let pt = ProviderType::ClaudeCode;
    let json = serde_json::to_string(&pt).unwrap();
    assert_eq!(json, "\"claude_code\"");
    let back: ProviderType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProviderType::ClaudeCode);
    let legacy: ProviderType = serde_json::from_str("\"claude_tty\"").unwrap();
    assert_eq!(legacy, ProviderType::ClaudeCode);

    let pt = ProviderType::AntigravityCli;
    let json = serde_json::to_string(&pt).unwrap();
    assert_eq!(json, "\"antigravity\"");
    let back: ProviderType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ProviderType::AntigravityCli);
    let alias: ProviderType = serde_json::from_str("\"agy\"").unwrap();
    assert_eq!(alias, ProviderType::AntigravityCli);

    let grok = ProviderType::GrokBuild;
    let json = serde_json::to_string(&grok).unwrap();
    assert_eq!(json, "\"grok_build\"");
    let alias: ProviderType = serde_json::from_str("\"grok\"").unwrap();
    assert_eq!(alias, ProviderType::GrokBuild);
}

#[test]
fn test_provider_type_slug_round_trip() {
    for provider_type in [
        ProviderType::ClaudeCode,
        ProviderType::CodexAppServer,
        ProviderType::Traex,
        ProviderType::AntigravityCli,
        ProviderType::GrokBuild,
    ] {
        assert_eq!(
            ProviderType::from_slug(provider_type.as_slug()),
            Some(provider_type)
        );
    }

    assert_eq!(
        ProviderType::from_slug("claude"),
        Some(ProviderType::ClaudeCode)
    );
    assert_eq!(
        ProviderType::from_slug("claude-tty"),
        Some(ProviderType::ClaudeCode)
    );
    assert_eq!(
        ProviderType::from_slug(" claude_tty "),
        Some(ProviderType::ClaudeCode)
    );
    assert_eq!(ProviderType::from_slug("unknown-provider"), None);
    assert_eq!(
        ProviderType::from_slug("agy"),
        Some(ProviderType::AntigravityCli)
    );
    assert_eq!(
        ProviderType::from_slug("grok"),
        Some(ProviderType::GrokBuild)
    );
}

#[test]
fn merge_thread_model_cells_applies_stored_cells() {
    let thread_data = serde_json::json!({
        "metadata": {
            "model": "provider-default-v1",
            "model_reasoning_effort": "high",
            "model_service_tier": "flex",
        }
    });
    let mut run_metadata = std::collections::HashMap::new();

    merge_thread_model_cells(&thread_data, &mut run_metadata);

    assert_eq!(
        run_metadata.get("model"),
        Some(&serde_json::Value::String("provider-default-v1".to_owned()))
    );
    assert_eq!(
        run_metadata.get("model_reasoning_effort"),
        Some(&serde_json::Value::String("high".to_owned()))
    );
    assert_eq!(
        run_metadata.get("model_service_tier"),
        Some(&serde_json::Value::String("flex".to_owned()))
    );
}

#[test]
fn merge_thread_model_cells_keeps_request_metadata_priority() {
    let thread_data = serde_json::json!({
        "metadata": {
            "model": "provider-default-v1",
            "model_reasoning_effort": "high",
        }
    });
    let mut run_metadata = std::collections::HashMap::from([(
        "model".to_owned(),
        serde_json::Value::String("request-model".to_owned()),
    )]);

    merge_thread_model_cells(&thread_data, &mut run_metadata);

    assert_eq!(
        run_metadata.get("model"),
        Some(&serde_json::Value::String("request-model".to_owned()))
    );
    assert_eq!(
        run_metadata.get("model_reasoning_effort"),
        Some(&serde_json::Value::String("high".to_owned()))
    );
}

#[test]
fn merge_thread_model_cells_ignores_blank_and_missing_values() {
    let thread_data = serde_json::json!({
        "metadata": {
            "model": "   ",
        }
    });
    let mut run_metadata = std::collections::HashMap::new();

    merge_thread_model_cells(&thread_data, &mut run_metadata);
    assert!(run_metadata.is_empty());

    merge_thread_model_cells(&serde_json::json!({}), &mut run_metadata);
    assert!(run_metadata.is_empty());
}

#[test]
fn merge_thread_model_cells_coalesces_legacy_override_in_front_of_cell() {
    let thread_data = serde_json::json!({
        "metadata": {
            "model": "cell-model",
            "model_override": "legacy-override-model",
        }
    });
    let mut run_metadata = std::collections::HashMap::new();

    merge_thread_model_cells(&thread_data, &mut run_metadata);

    assert_eq!(
        run_metadata.get("model"),
        Some(&serde_json::Value::String(
            "legacy-override-model".to_owned()
        ))
    );
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
    let title = StreamEvent::ThreadTitleUpdated {
        title: "Provider Title".to_owned(),
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
    let title_json = serde_json::to_string(&title).unwrap();
    assert_eq!(
        title_json,
        "{\"type\":\"thread_title_updated\",\"title\":\"Provider Title\"}"
    );
    let done_json = serde_json::to_string(&done).unwrap();
    assert_eq!(done_json, "{\"type\":\"done\"}");

    let delta_back: StreamEvent = serde_json::from_str(&delta_json).unwrap();
    let tool_use_back: StreamEvent = serde_json::from_str(&tool_use_json).unwrap();
    let tool_result_back: StreamEvent = serde_json::from_str(&tool_result_json).unwrap();
    let boundary_back: StreamEvent = serde_json::from_str(&boundary_json).unwrap();
    let boundary_with_input_back: StreamEvent =
        serde_json::from_str(&boundary_with_input_json).unwrap();
    let title_back: StreamEvent = serde_json::from_str(&title_json).unwrap();
    let done_back: StreamEvent = serde_json::from_str(&done_json).unwrap();
    assert_eq!(delta_back, delta);
    assert_eq!(tool_use_back, tool_use);
    assert_eq!(tool_result_back, tool_result);
    assert_eq!(boundary_back, boundary);
    assert_eq!(boundary_with_input_back, boundary_with_input);
    assert_eq!(title_back, title);
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
    assert_eq!(cfg.claude_cli_mode, "native");
    assert_eq!(cfg.model_reasoning_effort, "");
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
fn test_grok_build_config_defaults() {
    let cfg = GrokBuildConfig::default();
    assert_eq!(cfg.provider_type, ProviderType::GrokBuild);
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
            attachment_id: None,
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
                attachment_id: None,
                kind: PromptAttachmentKind::Image,
                path: "/tmp/shot.png".to_owned(),
                name: "shot.png".to_owned(),
                media_type: "image/png".to_owned(),
            },
            PromptAttachment {
                attachment_id: None,
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
