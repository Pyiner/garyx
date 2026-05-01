use super::*;
use garyx_models::ProviderMessage;
use serde_json::json;

#[test]
fn initialize_params_round_trip() {
    let raw = json!({
        "protocol_version": 1,
        "host": {
            "version": "0.1.3",
            "public_url": "https://foo.example.com",
            "data_dir": "/data",
            "locale": "zh-CN"
        },
        "accounts": [
            { "id": "a1", "enabled": true, "config": { "token": "x" } }
        ]
    });
    let parsed: InitializeParams = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(parsed.protocol_version, 1);
    assert!(!parsed.dry_run);
    assert_eq!(parsed.accounts.len(), 1);
}

#[test]
fn stream_event_tagged_serialization() {
    let delta = StreamEventFrame::Delta { text: "hi".into() };
    let ser = serde_json::to_value(&delta).unwrap();
    assert_eq!(ser["type"], "delta");
    assert_eq!(ser["text"], "hi");

    let boundary = StreamEventFrame::Boundary {
        kind: "user_ack".into(),
        text: None,
    };
    let ser = serde_json::to_value(&boundary).unwrap();
    assert_eq!(ser["type"], "boundary");
    assert_eq!(ser["kind"], "user_ack");

    let tool_use = StreamEventFrame::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Bash"}),
            Some("tool-1".to_owned()),
            Some("Bash".to_owned()),
        ),
    };
    let ser = serde_json::to_value(&tool_use).unwrap();
    assert_eq!(ser["type"], "tool_use");
    assert_eq!(ser["message"]["tool_name"], "Bash");
}

#[test]
fn dispatch_outbound_content_is_structured() {
    let raw = json!({
        "account_id": "acct",
        "chat_id": "chat",
        "delivery_target_type": "chat_id",
        "delivery_target_id": "chat",
        "content": {
            "type": "tool_use",
            "message": {
                "role": "tool_use",
                "content": {"name": "Read"},
                "tool_name": "Read"
            }
        }
    });
    let parsed: DispatchOutbound = serde_json::from_value(raw).unwrap();
    assert_eq!(parsed.content.kind(), "tool_use");
}

#[test]
fn inbound_end_status_ok_or_error() {
    let ok = InboundEnd {
        stream_id: "str_x".into(),
        seq: 2,
        status: InboundEndStatus::ok(),
        thread_id: "thr".into(),
        final_text: "done".into(),
    };
    let j = serde_json::to_value(&ok).unwrap();
    assert_eq!(j["status"], "ok");

    let err = InboundEndStatus::error("host_shutting_down");
    let j = serde_json::to_value(&err).unwrap();
    assert_eq!(j["error"], "host_shutting_down");
}

#[test]
fn error_code_round_trips() {
    for code in [
        PluginErrorCode::NotInitialized,
        PluginErrorCode::ConfigRejected,
        PluginErrorCode::Busy,
        PluginErrorCode::ChannelConfigRejected,
        PluginErrorCode::PayloadTooLarge,
    ] {
        let n = code.as_i32();
        assert_eq!(PluginErrorCode::from_i32(n), Some(code));
    }
    assert_eq!(PluginErrorCode::from_i32(-99999), None);
    assert!(PluginErrorCode::Busy.is_retryable());
    assert!(!PluginErrorCode::PayloadTooLarge.is_retryable());
}

#[test]
fn auth_flow_display_item_tagged_kind() {
    let t = AuthFlowDisplayItem::Text { value: "hi".into() };
    let j = serde_json::to_value(&t).unwrap();
    assert_eq!(j, serde_json::json!({"kind": "text", "value": "hi"}));

    let q = AuthFlowDisplayItem::Qr {
        value: "opaque-nonce".into(),
    };
    let j = serde_json::to_value(&q).unwrap();
    assert_eq!(
        j,
        serde_json::json!({"kind": "qr", "value": "opaque-nonce"})
    );

    // forward-compat: an unknown `kind` decodes as `Unknown`
    // instead of failing, so a newer plugin can add display
    // kinds without breaking older hosts.
    let future: AuthFlowDisplayItem =
        serde_json::from_value(serde_json::json!({"kind": "image", "src": "x"})).unwrap();
    assert!(matches!(future, AuthFlowDisplayItem::Unknown));
}

#[test]
fn auth_flow_poll_response_tagged_status() {
    let p = AuthFlowPollResponse::Pending {
        display: None,
        next_interval_secs: None,
    };
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["status"], "pending");
    assert!(j.get("display").is_none());
    assert!(j.get("next_interval_secs").is_none());

    let c = AuthFlowPollResponse::Confirmed {
        values: std::collections::BTreeMap::from_iter([(
            "token".into(),
            serde_json::Value::String("secret".into()),
        )]),
    };
    let j = serde_json::to_value(&c).unwrap();
    assert_eq!(j["status"], "confirmed");
    assert_eq!(j["values"]["token"], "secret");

    let f = AuthFlowPollResponse::Failed {
        reason: "expired".into(),
    };
    let j = serde_json::to_value(&f).unwrap();
    assert_eq!(j["status"], "failed");
    assert_eq!(j["reason"], "expired");
}

#[test]
fn reload_accounts_params_round_trip() {
    // Reuses the `AccountDescriptor` deserializer — same shape
    // as `initialize.params.accounts`, so any plugin that
    // already parses one can reuse that code for the reload
    // entry point.
    let p: ReloadAccountsParams = serde_json::from_value(json!({
        "accounts": [
            { "id": "a1", "enabled": true, "config": { "token": "x" } },
            { "id": "a2", "config": { "token": "y" } }
        ]
    }))
    .unwrap();
    assert_eq!(p.accounts.len(), 2);
    assert!(p.accounts[1].enabled, "enabled defaults to true");

    // Empty account list is the canonical "drop everything"
    // shape — omitted field decodes as `vec![]`.
    let empty: ReloadAccountsParams = serde_json::from_value(json!({})).unwrap();
    assert!(empty.accounts.is_empty());
}

#[test]
fn attachment_ref_untagged_distinguishes_shape() {
    let inline: AttachmentRef = serde_json::from_value(json!({
        "data": "abc",
        "media_type": "image/png"
    }))
    .unwrap();
    assert!(matches!(inline, AttachmentRef::Inline { .. }));

    let path: AttachmentRef = serde_json::from_value(json!({
        "path": "/tmp/x.png",
        "media_type": "image/png"
    }))
    .unwrap();
    assert!(matches!(path, AttachmentRef::Path { .. }));
}
