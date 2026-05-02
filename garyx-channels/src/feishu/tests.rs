use super::ws::FeishuRuntimeContext;
use super::*;

async fn dispatch_im_message_event(
    account_id: &str,
    event: &ImMessageReceiveEvent,
    router: &Arc<Mutex<MessageRouter>>,
    bridge: &Arc<MultiProviderBridge>,
    client: &FeishuClient,
    account: &FeishuAccount,
    _public_url: &str,
    bot_open_id: &str,
) {
    let runtime =
        FeishuRuntimeContext::new(account_id, router, bridge, client, account, bot_open_id);
    super::ws::handle_im_message_event(event, runtime).await;
}

// -- Token refresh logic --

#[tokio::test]
async fn test_token_refresh_needed_when_empty() {
    let client = FeishuClient {
        app_id: "test_app".into(),
        app_secret: "test_secret".into(),
        domain: FeishuDomain::Feishu,
        http: HttpClient::new(),
        token_state: Arc::new(RwLock::new(None)),
        refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        api_base_override: None,
    };

    // With no token, get_access_token should fail (can't reach API in tests)
    let result = client.get_access_token().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_token_present_and_valid() {
    let client = FeishuClient {
        app_id: "test_app".into(),
        app_secret: "test_secret".into(),
        domain: FeishuDomain::Feishu,
        http: HttpClient::new(),
        token_state: Arc::new(RwLock::new(Some((
            "t-valid-token".into(),
            Instant::now() + Duration::from_secs(3600),
        )))),
        refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        api_base_override: None,
    };

    // Token is present and not near expiry — should return it
    let token = client.get_access_token().await.unwrap();
    assert_eq!(token, "t-valid-token");
}

#[tokio::test]
async fn test_token_near_expiry_triggers_refresh() {
    let client = FeishuClient {
        app_id: "test_app".into(),
        app_secret: "test_secret".into(),
        domain: FeishuDomain::Feishu,
        http: HttpClient::new(),
        token_state: Arc::new(RwLock::new(Some((
            "t-old-token".into(),
            // Token expires in 2 minutes, within the 5-minute margin
            Instant::now() + Duration::from_secs(120),
        )))),
        refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        api_base_override: None,
    };

    // Should try to refresh because token is within the margin.
    // Since we can't reach the API, this will fail with an HTTP error.
    let result = client.get_access_token().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_token_not_near_expiry_skips_refresh() {
    let client = FeishuClient {
        app_id: "test_app".into(),
        app_secret: "test_secret".into(),
        domain: FeishuDomain::Feishu,
        http: HttpClient::new(),
        token_state: Arc::new(RwLock::new(Some((
            "t-fresh-token".into(),
            // Token expires in 1 hour, well beyond the 5-minute margin
            Instant::now() + Duration::from_secs(3600),
        )))),
        refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        api_base_override: None,
    };

    let token = client.get_access_token().await.unwrap();
    assert_eq!(token, "t-fresh-token");
}

// -- Event parsing --

#[test]
fn test_parse_message_event() {
    let json = r#"{
            "schema": "2.0",
            "header": {
                "event_id": "ev_123",
                "event_type": "im.message.receive_v1",
                "create_time": "1234567890",
                "token": "tok",
                "app_id": "cli_xxx",
                "tenant_key": "tk_xxx"
            },
            "event": {
                "message": {
                    "chat_id": "oc_abc",
                    "chat_type": "group",
                    "message_id": "om_xyz",
                    "message_type": "text",
                    "content": "{\"text\":\"hello world\"}",
                    "mentions": [
                        {
                            "key": "@_user_1",
                            "name": "Bot",
                            "id": { "open_id": "ou_bot123" }
                        }
                    ]
                },
                "sender": {
                    "sender_id": { "open_id": "ou_sender456" },
                    "sender_type": "user"
                }
            }
        }"#;

    let envelope: FeishuEventEnvelope = serde_json::from_str(json).unwrap();
    assert_eq!(
        envelope.header.as_ref().unwrap().event_type,
        "im.message.receive_v1"
    );
    assert_eq!(envelope.header.as_ref().unwrap().event_id, "ev_123");

    let event: ImMessageReceiveEvent = serde_json::from_value(envelope.event.unwrap()).unwrap();
    let msg = event.message.as_ref().unwrap();
    assert_eq!(msg.chat_id, "oc_abc");
    assert_eq!(msg.chat_type, "group");
    assert_eq!(msg.message_id, "om_xyz");
    assert_eq!(msg.message_type, "text");
    assert_eq!(msg.mentions.len(), 1);
    assert_eq!(msg.mentions[0].key, "@_user_1");
    assert_eq!(msg.mentions[0].id.as_ref().unwrap().open_id, "ou_bot123");

    let sender = event.sender.as_ref().unwrap();
    assert_eq!(sender.sender_id.as_ref().unwrap().open_id, "ou_sender456");
    assert_eq!(sender.sender_type, "user");
}

#[test]
fn test_parse_event_with_minimal_fields() {
    let json = r#"{
            "header": {
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "message": {
                    "chat_id": "oc_test",
                    "message_type": "text",
                    "content": "{\"text\":\"hi\"}"
                },
                "sender": {
                    "sender_id": { "open_id": "ou_user" }
                }
            }
        }"#;

    let envelope: FeishuEventEnvelope = serde_json::from_str(json).unwrap();
    let event: ImMessageReceiveEvent = serde_json::from_value(envelope.event.unwrap()).unwrap();
    assert_eq!(event.message.as_ref().unwrap().chat_id, "oc_test");
}

// -- Card content building --

#[test]
fn test_build_card_content() {
    let content = build_card_content("Hello **world**");
    let parsed: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["schema"], "2.0");
    let elements = parsed["body"]["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 1);
    assert_eq!(elements[0]["tag"], "markdown");
    assert_eq!(elements[0]["content"], "Hello **world**");
}

#[test]
fn test_build_card_content_empty() {
    let content = build_card_content("");
    let parsed: Value = serde_json::from_str(&content).unwrap();
    let elements = parsed["body"]["elements"].as_array().unwrap();
    assert_eq!(elements[0]["content"], "");
}

#[test]
fn test_build_text_content() {
    let content = build_text_content("hello");
    let parsed: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["text"], "hello");
}

// -- Image key extraction --

#[test]
fn test_extract_image_keys_image_message() {
    use super::message::extract_image_keys;
    let keys = extract_image_keys("image", r#"{"image_key":"img_abc123"}"#);
    assert_eq!(keys, vec!["img_abc123"]);
}

#[test]
fn test_extract_image_keys_post_with_images() {
    use super::message::extract_image_keys;
    let content = r#"{"title":"test","content":[[{"tag":"text","text":"hello"},{"tag":"img","image_key":"img_001"}],[{"tag":"img","image_key":"img_002"}]]}"#;
    let keys = extract_image_keys("post", content);
    assert_eq!(keys, vec!["img_001", "img_002"]);
}

#[test]
fn test_extract_image_keys_text_message() {
    use super::message::extract_image_keys;
    let keys = extract_image_keys("text", r#"{"text":"hello"}"#);
    assert!(keys.is_empty());
}

// -- Mention detection --

#[test]
fn test_is_mentioned_with_bot_id() {
    let mentions = vec![ImMention {
        key: "@_user_1".into(),
        name: "Bot".into(),
        id: Some(ImMentionId {
            open_id: "ou_bot123".into(),
        }),
    }];
    assert!(is_mentioned(&mentions, "ou_bot123"));
}

#[test]
fn test_is_mentioned_different_user() {
    let mentions = vec![ImMention {
        key: "@_user_1".into(),
        name: "Other".into(),
        id: Some(ImMentionId {
            open_id: "ou_other".into(),
        }),
    }];
    assert!(!is_mentioned(&mentions, "ou_bot123"));
}

#[test]
fn test_is_mentioned_empty_bot_id() {
    let mentions = vec![ImMention {
        key: "@_user_1".into(),
        name: "Someone".into(),
        id: Some(ImMentionId {
            open_id: "ou_someone".into(),
        }),
    }];
    // Empty bot_open_id should return true (assume mentioned)
    assert!(is_mentioned(&mentions, ""));
}

#[test]
fn test_is_mentioned_empty_mentions() {
    assert!(!is_mentioned(&[], "ou_bot123"));
}

#[test]
fn test_is_mentioned_no_id() {
    let mentions = vec![ImMention {
        key: "@_user_1".into(),
        name: "NoId".into(),
        id: None,
    }];
    assert!(!is_mentioned(&mentions, "ou_bot123"));
}

// -- Text extraction --

#[test]
fn test_extract_text_message() {
    let text = extract_message_text("text", r#"{"text":"hello world"}"#);
    assert_eq!(text, "hello world");
}

#[test]
fn test_extract_text_message_empty_content() {
    let text = extract_message_text("text", "");
    assert_eq!(text, "");
}

#[test]
fn test_extract_text_message_invalid_json() {
    let text = extract_message_text("text", "not json");
    assert_eq!(text, "not json");
}

#[test]
fn test_extract_image_message() {
    let text = extract_message_text("image", "{}");
    assert_eq!(text, "<media:image>");
}

#[test]
fn test_extract_post_message() {
    let content = r#"{
            "title": "Post Title",
            "content": [
                [
                    {"tag": "text", "text": "Hello "},
                    {"tag": "a", "text": "link text"},
                    {"tag": "at", "user_name": "John"}
                ]
            ]
        }"#;
    let text = extract_message_text("post", content);
    assert!(text.contains("Post Title"));
    assert!(text.contains("Hello "));
    assert!(text.contains("link text"));
    assert!(text.contains("@John"));
}

#[test]
fn test_extract_post_message_empty() {
    let text = extract_message_text("post", r#"{"content":[]}"#);
    assert_eq!(text, "[rich post]");
}

#[test]
fn test_extract_unknown_type() {
    let text = extract_message_text("unknown", "{}");
    assert_eq!(text, "");
}

// -- Mention stripping --

#[test]
fn test_strip_mention_tokens() {
    let mentions = vec![ImMention {
        key: "@_user_1".into(),
        name: "Bot".into(),
        id: Some(ImMentionId {
            open_id: "ou_bot".into(),
        }),
    }];
    let result = strip_mention_tokens("@_user_1 hello @Bot world", &mentions);
    assert_eq!(result, "hello world");
}

#[test]
fn test_strip_mention_tokens_empty() {
    let result = strip_mention_tokens("", &[]);
    assert_eq!(result, "");
}

#[test]
fn test_strip_mention_tokens_no_mentions() {
    let result = strip_mention_tokens("hello world", &[]);
    assert_eq!(result, "hello world");
}

#[test]
fn test_strip_mention_tokens_skips_missing_open_id() {
    let mentions = vec![ImMention {
        key: "@_user_1".into(),
        name: "Bot".into(),
        id: None,
    }];
    let result = strip_mention_tokens("@_user_1 hello @Bot world", &mentions);
    assert_eq!(result, "@_user_1 hello @Bot world");
}

// -- API base URL --

#[test]
fn test_api_base_feishu() {
    let client = FeishuClient {
        app_id: String::new(),
        app_secret: String::new(),
        domain: FeishuDomain::Feishu,
        http: HttpClient::new(),
        token_state: Arc::new(RwLock::new(None)),
        refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        api_base_override: None,
    };
    assert_eq!(client.api_base(), FEISHU_API_BASE);
}

#[test]
fn test_api_base_lark() {
    let client = FeishuClient {
        app_id: String::new(),
        app_secret: String::new(),
        domain: FeishuDomain::Lark,
        http: HttpClient::new(),
        token_state: Arc::new(RwLock::new(None)),
        refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        api_base_override: None,
    };
    assert_eq!(client.api_base(), LARK_API_BASE);
}

// -- Error display --

#[test]
fn test_error_display() {
    let err = FeishuError::Http("connection refused".into());
    assert!(err.to_string().contains("connection refused"));

    let err = FeishuError::Api {
        code: 99991672,
        msg: "permission denied".into(),
    };
    assert!(err.to_string().contains("99991672"));
    assert!(err.to_string().contains("permission denied"));

    let err = FeishuError::WebSocket("closed unexpectedly".into());
    assert!(err.to_string().contains("closed unexpectedly"));
}

#[test]
fn test_parse_native_command() {
    assert!(garyx_router::is_native_command_text("/threads", "feishu"));
    assert!(garyx_router::is_native_command_text(
        "/threadnext@bot_name foo",
        "feishu"
    ));
    assert!(!garyx_router::is_native_command_text("hello", "feishu"));
    assert!(!garyx_router::is_native_command_text("/unknown", "feishu"));
    assert!(!garyx_router::is_native_command_text("/start", "feishu"));
}

#[test]
fn test_extract_permission_grant_url() {
    assert_eq!(
        extract_permission_grant_url(
            "Feishu API error (code=99991672): visit https://open.feishu.cn/appPermission?appId=cli_xxx"
        ),
        Some(Some(
            "https://open.feishu.cn/appPermission?appId=cli_xxx".to_owned()
        ))
    );
    assert_eq!(
        extract_permission_grant_url("Feishu API error (code=99991672): permission denied"),
        Some(None)
    );
    assert_eq!(
        extract_permission_grant_url("Feishu API error (code=123): permission denied"),
        None
    );
    assert_eq!(
        extract_permission_grant_url(
            "Feishu API error (code=99991672): see <https://open.feishu.cn/appPermission?appId=cli_xxx>"
        ),
        Some(Some(
            "https://open.feishu.cn/appPermission?appId=cli_xxx".to_owned()
        ))
    );
}

#[test]
fn test_permission_notice_cooldown() {
    reset_permission_error_notice_cache();
    assert!(should_emit_permission_notice("app1", "oc_1"));
    assert!(!should_emit_permission_notice("app1", "oc_1"));
    assert!(should_emit_permission_notice("app1", "oc_2"));
}

// -----------------------------------------------------------------------
// Router/Bridge dispatch tests
// -----------------------------------------------------------------------

mod dispatch_tests {
    use super::*;
    use garyx_bridge::provider_trait::StreamCallback;
    use garyx_bridge::{AgentLoopProvider, BridgeError, MultiProviderBridge};
    use garyx_models::config::GaryxConfig;
    use garyx_models::provider::{
        ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent,
    };
    use garyx_router::{InMemoryThreadStore, MessageRouter};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Mock provider that tracks calls.
    struct TestProvider {
        call_count: AtomicUsize,
        messages: std::sync::Mutex<Vec<String>>,
        metadata: std::sync::Mutex<Vec<HashMap<String, Value>>>,
        thread_ids: std::sync::Mutex<Vec<String>>,
    }

    impl TestProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
                messages: std::sync::Mutex::new(Vec::new()),
                metadata: std::sync::Mutex::new(Vec::new()),
                thread_ids: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl AgentLoopProvider for TestProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::ClaudeCode
        }

        fn is_ready(&self) -> bool {
            true
        }

        async fn initialize(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn run_streaming(
            &self,
            options: &ProviderRunOptions,
            on_chunk: StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            self.messages.lock().unwrap().push(options.message.clone());
            self.metadata.lock().unwrap().push(options.metadata.clone());
            self.thread_ids
                .lock()
                .unwrap()
                .push(options.thread_id.clone());
            let response = format!("echo: {}", options.message);
            on_chunk(StreamEvent::Delta {
                text: response.clone(),
            });
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "test-run".into(),
                thread_id: options.thread_id.clone(),
                response,
                session_messages: Vec::new(),
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 10,
                output_tokens: 5,
                cost: 0.001,
                duration_ms: 42,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    fn make_router() -> Arc<Mutex<MessageRouter>> {
        let store = Arc::new(InMemoryThreadStore::new());
        let config = GaryxConfig::default();
        let mut router = MessageRouter::new(store, config);
        router.set_message_ledger_store(Arc::new(garyx_router::MessageLedgerStore::memory()));
        Arc::new(Mutex::new(router))
    }

    async fn make_bridge() -> (Arc<MultiProviderBridge>, Arc<TestProvider>) {
        let bridge = Arc::new(MultiProviderBridge::new());
        let provider = Arc::new(TestProvider::new());
        bridge
            .register_provider("test-provider", provider.clone())
            .await;
        bridge.set_default_provider_key("test-provider").await;
        (bridge, provider)
    }

    #[tokio::test]
    async fn test_feishu_session_resolution_dm() {
        let router = make_router();
        let thread_id = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("feishu", "app1", "ou_user123", false, None)
        };
        assert!(thread_id.starts_with("thread::"), "{thread_id}");
    }

    #[tokio::test]
    async fn test_feishu_session_resolution_group() {
        let router = make_router();
        let thread_id = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("feishu", "app1", "ou_user123", true, Some("oc_group456"))
        };
        assert!(thread_id.starts_with("thread::"), "{thread_id}");
    }

    #[tokio::test]
    async fn test_feishu_bridge_dispatch() {
        let router = make_router();
        let (bridge, _provider) = make_bridge().await;

        let thread_id = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("feishu", "app1", "ou_user123", false, None)
        };
        assert!(thread_id.starts_with("thread::"), "{thread_id}");

        let result = bridge
            .start_agent_run(
                garyx_models::provider::AgentRunRequest::new(
                    &thread_id,
                    "hello from feishu",
                    "feishu-run-1",
                    "feishu",
                    "app1",
                    HashMap::new(),
                ),
                None,
            )
            .await;
        assert!(result.is_ok());

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!bridge.is_run_active("feishu-run-1").await);
    }

    #[tokio::test]
    async fn test_feishu_reply_routing() {
        let router = make_router();

        // Record outbound message
        {
            let mut r = router.lock().await;
            r.record_outbound_message("app1::main::ou_user123", "feishu", "app1", "om_reply_msg");
        }

        // Resolve via reply
        {
            let r = router.lock().await;
            let thread_id = r.resolve_reply_thread("feishu", "app1", "om_reply_msg");
            assert_eq!(thread_id, Some("app1::main::ou_user123"));
        }
    }

    #[tokio::test]
    async fn test_feishu_dispatch_with_callback() {
        let router = make_router();
        let (bridge, _provider) = make_bridge().await;

        let session_key = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("feishu", "app1", "ou_user", false, None)
        };

        let callback_called = Arc::new(AtomicBool::new(false));
        let cb_flag = callback_called.clone();
        let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
            cb_flag.store(true, Ordering::Relaxed);
            match event {
                StreamEvent::Delta { text } => assert!(text.contains("echo:")),
                StreamEvent::Done => {}
                StreamEvent::Boundary { .. } => {}
                StreamEvent::ToolUse { .. } | StreamEvent::ToolResult { .. } => {}
            }
        });

        let result = bridge
            .start_agent_run(
                garyx_models::provider::AgentRunRequest::new(
                    &session_key,
                    "test message",
                    "feishu-cb-1",
                    "feishu",
                    "app1",
                    HashMap::new(),
                ),
                Some(callback),
            )
            .await;
        assert!(result.is_ok());

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(callback_called.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn test_feishu_response_callback_sends_reply() {
        // P1-C: verify the callback accumulates response parts and
        // constructs the correct card content for sending.
        // We can't mock the HTTP server easily, but we verify the
        // callback structure: parts accumulate, and on Done the
        // full response is joined.
        let parts = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let final_response = Arc::new(std::sync::Mutex::new(Option::<String>::None));

        let parts_cb = parts.clone();
        let final_cb = final_response.clone();

        let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| match event {
            StreamEvent::Delta { text } => {
                if !text.is_empty() {
                    parts_cb.lock().unwrap().push(text);
                }
            }
            StreamEvent::Done => {
                let full = parts_cb.lock().unwrap().join("");
                *final_cb.lock().unwrap() = Some(full);
            }
            StreamEvent::Boundary { .. } => {}
            StreamEvent::ToolUse { .. } | StreamEvent::ToolResult { .. } => {}
        });

        // Simulate streaming events
        callback(StreamEvent::Delta {
            text: "Hello ".to_owned(),
        });
        callback(StreamEvent::Delta {
            text: "world!".to_owned(),
        });
        callback(StreamEvent::Done);

        let response = final_response.lock().unwrap().clone();
        assert_eq!(response, Some("Hello world!".to_owned()));

        // Verify card content format
        let card = build_card_content("Hello world!");
        let parsed: serde_json::Value = serde_json::from_str(&card).unwrap();
        assert_eq!(parsed["body"]["elements"][0]["tag"], "markdown");
        assert_eq!(parsed["body"]["elements"][0]["content"], "Hello world!");
    }

    #[tokio::test]
    async fn test_feishu_group_thread_routing() {
        let router = make_router();

        // Group with thread (root_id)
        let with_thread = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("feishu", "app1", "ou_user", true, Some("om_root_thread"))
        };
        assert!(with_thread.starts_with("thread::"));

        // Group with chat_id as fallback
        let with_chat = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("feishu", "app1", "ou_user", true, Some("oc_chat_id"))
        };
        assert!(with_chat.starts_with("thread::"));
        assert_ne!(with_chat, with_thread);
    }

    #[tokio::test]
    async fn test_feishu_session_switch_isolated_across_accounts() {
        let router = make_router();

        {
            let mut r = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("feishu", "app1", "ou_user123");
            r.switch_to_thread(&user_key, "app1_custom");
        }

        let app1_session = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("feishu", "app1", "ou_user123", false, None)
        };
        let app2_session = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("feishu", "app2", "ou_user123", false, None)
        };

        assert_eq!(app1_session, "app1_custom");
        assert!(app2_session.starts_with("thread::"));
    }

    #[tokio::test]
    async fn test_feishu_pending_history_replayed_when_mentioned() {
        let account_id = "app_pending_history_replayed";
        let chat_id = "oc_group_pending_history_replayed";
        clear_pending_history(account_id, chat_id);
        let router = make_router();
        let (bridge, provider) = make_bridge().await;
        let account = FeishuAccount {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            enabled: true,
            domain: FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: TopicSessionMode::Disabled,
        };

        let client = FeishuClient {
            app_id: "test_app".to_owned(),
            app_secret: "test_secret".to_owned(),
            domain: FeishuDomain::Feishu,
            http: HttpClient::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_base_override: Some("http://127.0.0.1:1".to_owned()),
        };

        let event1 =
            crate::test_helpers::FeishuEventBuilder::group("ou_user", chat_id, "first message")
                .build();
        dispatch_im_message_event(
            account_id,
            &event1,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        let event2 =
            crate::test_helpers::FeishuEventBuilder::group("ou_user", chat_id, "second message")
                .build();
        dispatch_im_message_event(
            account_id,
            &event2,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 0);

        let event3 = crate::test_helpers::FeishuEventBuilder::group(
            "ou_user",
            chat_id,
            "@_user_1 now respond",
        )
        .with_mention("@_user_1", "Bot", "test_app")
        .with_message_id("om_test_msg_002")
        .build();
        dispatch_im_message_event(
            account_id,
            &event3,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let messages = provider.messages.lock().unwrap();
        assert_eq!(
            messages[0],
            "ou_user: first message\nou_user: second message\nou_user: now respond"
        );

        let pending = get_pending_history(account_id, chat_id);
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn test_feishu_mention_forward_injects_system_hint() {
        let account_id = "app_mention_forward_hint";
        let chat_id = "oc_group_mention_forward_hint";
        clear_pending_history(account_id, chat_id);
        let router = make_router();
        let (bridge, provider) = make_bridge().await;
        let account = FeishuAccount {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            enabled: true,
            domain: FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: TopicSessionMode::Disabled,
        };

        let client = FeishuClient {
            app_id: "test_app".to_owned(),
            app_secret: "test_secret".to_owned(),
            domain: FeishuDomain::Feishu,
            http: HttpClient::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_base_override: Some("http://127.0.0.1:1".to_owned()),
        };

        let event = crate::test_helpers::FeishuEventBuilder::group(
            "ou_user",
            chat_id,
            "@_bot @_alice please handle this",
        )
        .with_mention("@_bot", "Bot", "test_app")
        .with_mention("@_alice", "Alice", "ou_alice")
        .build();
        dispatch_im_message_event(
            account_id,
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let messages = provider.messages.lock().unwrap();
        assert!(
                messages[0].contains(
                    "[System: Your reply will automatically @mention: Alice. Do not write @xxx yourself.]"
                ),
                "missing mention-forward system hint: {}",
                messages[0]
            );
    }

    #[tokio::test]
    async fn test_feishu_mention_detection_uses_bot_open_id_not_app_id() {
        let account_id = "app_mention_detection";
        let chat_id = "oc_group_mention_detection";
        clear_pending_history(account_id, chat_id);
        let router = make_router();
        let (bridge, provider) = make_bridge().await;
        let account = FeishuAccount {
            app_id: "cli_app_id".into(),
            app_secret: "test_secret".into(),
            enabled: true,
            domain: FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: TopicSessionMode::Disabled,
        };

        let client = FeishuClient {
            app_id: "cli_app_id".to_owned(),
            app_secret: "test_secret".to_owned(),
            domain: FeishuDomain::Feishu,
            http: HttpClient::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_base_override: Some("http://127.0.0.1:1".to_owned()),
        };

        let event = crate::test_helpers::FeishuEventBuilder::group(
            "ou_user",
            chat_id,
            "@_user_1 now respond",
        )
        .with_mention("@_user_1", "Bot", "ou_real_bot")
        .with_message_id("om_test_msg_bot_open_id")
        .build();
        dispatch_im_message_event(
            account_id,
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            "ou_real_bot",
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let pending = get_pending_history(account_id, chat_id);
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn test_feishu_dispatch_includes_python_parity_metadata_fields() {
        let router = make_router();
        let (bridge, provider) = make_bridge().await;
        let account = FeishuAccount {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            enabled: true,
            domain: FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            require_mention: false,
            topic_session_mode: TopicSessionMode::Disabled,
        };
        let client = FeishuClient {
            app_id: "test_app".to_owned(),
            app_secret: "test_secret".to_owned(),
            domain: FeishuDomain::Feishu,
            http: HttpClient::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_base_override: Some("http://127.0.0.1:1".to_owned()),
        };

        let event = crate::test_helpers::FeishuEventBuilder::dm("ou_user", "hello").build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            "ou_real_bot",
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let all_metadata = provider.metadata.lock().unwrap();
        let metadata = &all_metadata[0];
        assert_eq!(
            metadata.get("channel").and_then(Value::as_str),
            Some("feishu")
        );
        assert_eq!(
            metadata.get("account_id").and_then(Value::as_str),
            Some("app1")
        );
        assert_eq!(
            metadata.get("message_type").and_then(Value::as_str),
            Some("text")
        );
        assert_eq!(
            metadata.get("event_type").and_then(Value::as_str),
            Some("im.message.receive_v1")
        );
        assert_eq!(metadata.get("root_id").and_then(Value::as_str), Some(""));
        assert_eq!(
            metadata.get("mentioned_bot").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[tokio::test]
    async fn test_feishu_dm_falls_back_to_chat_id_when_sender_open_id_missing() {
        let router = make_router();
        let (bridge, provider) = make_bridge().await;
        let account = FeishuAccount {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            enabled: true,
            domain: FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            require_mention: false,
            topic_session_mode: TopicSessionMode::Disabled,
        };
        let client = FeishuClient {
            app_id: "test_app".to_owned(),
            app_secret: "test_secret".to_owned(),
            domain: FeishuDomain::Feishu,
            http: HttpClient::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_base_override: Some("http://127.0.0.1:1".to_owned()),
        };

        let event = crate::test_helpers::FeishuEventBuilder::dm("", "hello").build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            "ou_real_bot",
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let messages = provider.messages.lock().unwrap();
        assert_eq!(messages[0], "hello");
        let thread_ids = provider.thread_ids.lock().unwrap();
        assert!(
            thread_ids[0].starts_with("thread::"),
            "expected canonical thread id, got {}",
            thread_ids[0]
        );
    }

    #[tokio::test]
    async fn test_feishu_native_sessions_command_is_handled_locally() {
        let router = make_router();
        let (bridge, provider) = make_bridge().await;
        let account = FeishuAccount {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            enabled: true,
            domain: FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            require_mention: false,
            topic_session_mode: TopicSessionMode::Disabled,
        };
        let client = FeishuClient {
            app_id: "test_app".to_owned(),
            app_secret: "test_secret".to_owned(),
            domain: FeishuDomain::Feishu,
            http: HttpClient::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_base_override: Some("http://127.0.0.1:1".to_owned()),
        };

        let event = crate::test_helpers::FeishuEventBuilder::dm("ou_user", "/threads").build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            "ou_real_bot",
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            0,
            "native command should not route into provider"
        );
    }
}

// -----------------------------------------------------------------------
// End-to-end tests with mock HTTP server
// -----------------------------------------------------------------------

mod e2e_tests {
    use super::*;
    use crate::test_helpers::*;
    use async_trait::async_trait;
    use garyx_bridge::provider_trait::StreamCallback;
    use garyx_bridge::{AgentLoopProvider, BridgeError};
    use garyx_models::config::GaryxConfig;
    use garyx_models::provider::{
        ProviderMessage, ProviderRunOptions, ProviderRunResult, ProviderType, StreamBoundaryKind,
        StreamEvent,
    };
    use garyx_router::{
        ChannelBinding, InMemoryThreadStore, MessageRouter, ThreadEnsureOptions, ThreadStore,
        bind_endpoint_to_thread, bindings_from_value, create_thread_record,
        detach_endpoint_from_thread, is_thread_key,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{method, path, path_regex, query_param};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    struct FeishuUserAckBoundaryProvider;

    #[async_trait]
    impl AgentLoopProvider for FeishuUserAckBoundaryProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::ClaudeCode
        }

        fn is_ready(&self) -> bool {
            true
        }

        async fn initialize(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn run_streaming(
            &self,
            options: &ProviderRunOptions,
            on_chunk: StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            on_chunk(StreamEvent::Delta {
                text: "第一段".to_owned(),
            });
            on_chunk(StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: None,
            });
            on_chunk(StreamEvent::Delta {
                text: "第二段".to_owned(),
            });
            on_chunk(StreamEvent::Done);

            Ok(ProviderRunResult {
                run_id: "feishu-user-ack-boundary".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "第一段\n\n第二段".to_owned(),
                session_messages: Vec::new(),
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 1,
                output_tokens: 1,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    struct FeishuAssistantSegmentProvider;

    #[async_trait]
    impl AgentLoopProvider for FeishuAssistantSegmentProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::ClaudeCode
        }

        fn is_ready(&self) -> bool {
            true
        }

        async fn initialize(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn run_streaming(
            &self,
            options: &ProviderRunOptions,
            on_chunk: StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            on_chunk(StreamEvent::Delta {
                text: "第一段".to_owned(),
            });
            on_chunk(StreamEvent::Boundary {
                kind: StreamBoundaryKind::AssistantSegment,
                pending_input_id: None,
            });
            on_chunk(StreamEvent::Delta {
                text: "第二段".to_owned(),
            });
            on_chunk(StreamEvent::Done);

            Ok(ProviderRunResult {
                run_id: "feishu-assistant-segment".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "第一段\n\n第二段".to_owned(),
                session_messages: Vec::new(),
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 1,
                output_tokens: 1,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    struct FeishuToolThenAssistantProvider;

    #[async_trait]
    impl AgentLoopProvider for FeishuToolThenAssistantProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::CodexAppServer
        }

        fn is_ready(&self) -> bool {
            true
        }

        async fn initialize(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn run_streaming(
            &self,
            options: &ProviderRunOptions,
            on_chunk: StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            let tool_use = ProviderMessage::tool_use(
                serde_json::json!({
                    "type": "commandExecution",
                    "command": "pwd",
                    "status": "in_progress",
                }),
                Some("tool-feishu-1".to_owned()),
                Some("shell".to_owned()),
            );
            let tool_result = ProviderMessage::tool_result(
                serde_json::json!({
                    "type": "commandExecution",
                    "command": "pwd",
                    "status": "completed",
                    "output": "/tmp/workspace",
                    "exitCode": 0,
                }),
                Some("tool-feishu-1".to_owned()),
                Some("shell".to_owned()),
                Some(false),
            );

            on_chunk(StreamEvent::Delta {
                text: "先说一句".to_owned(),
            });
            on_chunk(StreamEvent::ToolUse { message: tool_use });
            on_chunk(StreamEvent::ToolResult {
                message: tool_result,
            });
            on_chunk(StreamEvent::Delta {
                text: "，再接一句".to_owned(),
            });
            on_chunk(StreamEvent::Done);

            Ok(ProviderRunResult {
                run_id: "feishu-tool-then-assistant".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "先说一句，再接一句".to_owned(),
                session_messages: Vec::new(),
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 1,
                output_tokens: 1,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    struct FeishuImageGenerationProvider;

    #[async_trait]
    impl AgentLoopProvider for FeishuImageGenerationProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::CodexAppServer
        }

        fn is_ready(&self) -> bool {
            true
        }

        async fn initialize(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn run_streaming(
            &self,
            options: &ProviderRunOptions,
            on_chunk: StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            let tool_use = ProviderMessage::tool_use(
                serde_json::json!({
                    "type": "imageGeneration",
                    "id": "ig-feishu-test",
                    "status": "in_progress",
                    "result": "",
                }),
                Some("ig-feishu-test".to_owned()),
                Some("imageGeneration".to_owned()),
            )
            .with_metadata_value("item_type", serde_json::json!("imageGeneration"));
            on_chunk(StreamEvent::ToolUse { message: tool_use });

            let tool_result = ProviderMessage::tool_result(
                serde_json::json!({
                    "type": "imageGeneration",
                    "id": "ig-feishu-test",
                    "status": "completed",
                    "result": "iVBORw0KGgo=",
                }),
                Some("ig-feishu-test".to_owned()),
                Some("imageGeneration".to_owned()),
                Some(false),
            )
            .with_metadata_value("item_type", serde_json::json!("imageGeneration"));
            on_chunk(StreamEvent::ToolResult {
                message: tool_result,
            });
            on_chunk(StreamEvent::Done);

            Ok(ProviderRunResult {
                run_id: "feishu-image-generation".to_owned(),
                thread_id: options.thread_id.clone(),
                response: String::new(),
                session_messages: Vec::new(),
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 1,
                output_tokens: 1,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    /// Create a default permissive FeishuAccount for testing (open DM + open group).
    fn make_default_account() -> FeishuAccount {
        FeishuAccount {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            enabled: true,
            domain: FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            require_mention: false,
            topic_session_mode: TopicSessionMode::Disabled,
        }
    }

    /// Start a mock server with Feishu token, send_message, and reply_message mocked.
    /// Returns (server, FeishuClient configured to use mock server).
    async fn setup_feishu_mock() -> (MockServer, FeishuClient) {
        let server = MockServer::start().await;

        // Mock token endpoint
        Mock::given(method("POST"))
            .and(path("/auth/v3/tenant_access_token/internal"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "tenant_access_token": "t-test-token-abc",
                "expire": 7200,
                "msg": "ok"
            })))
            .mount(&server)
            .await;

        // Mock send_message endpoint (DMs)
        Mock::given(method("POST"))
            .and(path_regex(r"^/im/v1/messages$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok",
                "data": {"message_id": "om_mock_reply_dm"}
            })))
            .mount(&server)
            .await;

        // Mock reply_message endpoint (groups)
        Mock::given(method("POST"))
            .and(path_regex(r"/im/v1/messages/.+/reply"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok",
                "data": {"message_id": "om_mock_reply_group"}
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/im/v1/images"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok",
                "data": {"image_key": "img_mock_generated"}
            })))
            .mount(&server)
            .await;

        // Mock Card Kit streaming endpoints used by Feishu streaming replies.
        Mock::given(method("POST"))
            .and(path("/cardkit/v1/cards"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok",
                "data": {"card_id": "cardkit_mock_card"}
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/cardkit/v1/cards/[^/]+/elements/content/content$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path_regex(r"^/cardkit/v1/cards/[^/]+/settings$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok"
            })))
            .mount(&server)
            .await;

        let client = FeishuClient {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            domain: FeishuDomain::Feishu,
            http: HttpClient::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_base_override: Some(server.uri()),
        };

        (server, client)
    }

    fn make_router_with_store(store: Arc<dyn ThreadStore>) -> Arc<Mutex<MessageRouter>> {
        let mut router = MessageRouter::new(store, GaryxConfig::default());
        router.set_message_ledger_store(Arc::new(garyx_router::MessageLedgerStore::memory()));
        Arc::new(Mutex::new(router))
    }

    async fn seed_bound_dm_thread(
        store: &Arc<dyn ThreadStore>,
        account_id: &str,
        binding_key: &str,
        label: &str,
    ) -> String {
        let options = ThreadEnsureOptions {
            label: Some(label.to_owned()),
            workspace_dir: None,
            agent_id: None,
            metadata: Default::default(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: Some("feishu".to_owned()),
            origin_account_id: Some(account_id.to_owned()),
            origin_from_id: Some(binding_key.to_owned()),
            is_group: Some(false),
        };
        let (thread_id, _) = create_thread_record(store, options).await.unwrap();
        bind_endpoint_to_thread(
            store,
            &thread_id,
            ChannelBinding {
                channel: "feishu".to_owned(),
                account_id: account_id.to_owned(),
                binding_key: binding_key.to_owned(),
                chat_id: format!("oc_dm_{binding_key}"),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: format!("oc_dm_{binding_key}"),
                display_label: label.to_owned(),
                last_inbound_at: None,
                last_delivery_at: None,
            },
        )
        .await
        .unwrap();
        thread_id
    }

    async fn wait_for_matching_requests<F>(
        server: &MockServer,
        timeout: std::time::Duration,
        expected_min: usize,
        mut predicate: F,
    ) -> Vec<Request>
    where
        F: FnMut(&Request) -> bool,
    {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let requests = server.received_requests().await.unwrap_or_default();
            let matching: Vec<_> = requests.into_iter().filter(|r| predicate(r)).collect();
            if matching.len() >= expected_min {
                return matching;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "timed out waiting for requests: expected >= {expected_min}, got {}",
                    matching.len()
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_request_quiet_window(
        server: &MockServer,
        quiet_for: std::time::Duration,
        timeout: std::time::Duration,
        expected_min: usize,
    ) -> Vec<Request> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut stable_since = tokio::time::Instant::now();
        let mut last_count = usize::MAX;
        loop {
            let requests = server.received_requests().await.unwrap_or_default();
            let count = requests.len();
            let now = tokio::time::Instant::now();
            if count != last_count {
                last_count = count;
                stable_since = now;
            } else if count >= expected_min && now.duration_since(stable_since) >= quiet_for {
                return requests;
            }
            if now >= deadline {
                panic!(
                    "timed out waiting for request quiet window: expected >= {expected_min}, got {count}"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_matching_requests_quiet_window<F>(
        server: &MockServer,
        quiet_for: std::time::Duration,
        timeout: std::time::Duration,
        expected_min: usize,
        mut predicate: F,
    ) -> Vec<Request>
    where
        F: FnMut(&Request) -> bool,
    {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut stable_since = tokio::time::Instant::now();
        let mut last_count = usize::MAX;
        loop {
            let requests = server.received_requests().await.unwrap_or_default();
            let matching: Vec<_> = requests.into_iter().filter(|r| predicate(r)).collect();
            let count = matching.len();
            let now = tokio::time::Instant::now();
            if count != last_count {
                last_count = count;
                stable_since = now;
            } else if count >= expected_min && now.duration_since(stable_since) >= quiet_for {
                return matching;
            }
            if now >= deadline {
                panic!(
                    "timed out waiting for matching request quiet window: expected >= {expected_min}, got {count}"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_counter_at_least(counter: &AtomicUsize, expected_min: usize) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if counter.load(Ordering::Relaxed) >= expected_min {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "timed out waiting for counter: expected >= {expected_min}, got {}",
                    counter.load(Ordering::Relaxed)
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn test_e2e_feishu_dm_full_chain() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let account = make_default_account();

        let event = FeishuEventBuilder::dm("ou_user123", "hello feishu").build();

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        wait_for_provider_calls(provider.as_ref(), 1).await;

        // Verify provider called with the correct thread and message
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let session_key = {
            let calls = provider.calls.lock().unwrap();
            assert_eq!(calls[0].message, "ou_user123: hello feishu");
            assert!(
                calls[0].thread_id.starts_with("thread::"),
                "expected canonical thread id, got {}",
                calls[0].thread_id
            );
            calls[0].thread_id.clone()
        };
        let current = {
            let mut router_guard = router.lock().await;
            router_guard
                .resolve_endpoint_thread_id("feishu", "app1", "ou_user123")
                .await
        };
        assert_eq!(current.as_deref(), Some(session_key.as_str()));
        let persisted = store.get(&session_key).await.expect("thread should exist");
        let bindings = bindings_from_value(&persisted);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].endpoint_key(), "feishu::app1::ou_user123");

        // Verify send_message was called (DM uses send, not reply)
        let token_calls =
            wait_for_matching_requests(&server, std::time::Duration::from_secs(5), 1, |r| {
                r.url.path().contains("tenant_access_token")
            })
            .await;
        assert!(!token_calls.is_empty(), "token endpoint should be called");

        // send_message (not reply) should be called for DMs
        let send_calls =
            wait_for_matching_requests(&server, std::time::Duration::from_secs(5), 1, |r| {
                r.method.as_str() == "POST" && r.url.path() == "/im/v1/messages"
            })
            .await;
        assert!(
            !send_calls.is_empty(),
            "send_message should be called for DM"
        );

        // Streaming DMs now send a Card Kit reference message and keep the
        // actual markdown body inside Card Kit create/update requests.
        let send_body: serde_json::Value = serde_json::from_slice(&send_calls[0].body).unwrap();
        let send_content: serde_json::Value =
            serde_json::from_str(send_body["content"].as_str().unwrap()).unwrap();
        assert_eq!(send_body["msg_type"], "interactive");
        assert_eq!(send_content["type"], "card");
        assert_eq!(send_content["data"]["card_id"], "cardkit_mock_card");

        let card_create_calls =
            wait_for_matching_requests(&server, std::time::Duration::from_secs(5), 1, |r| {
                r.method.as_str() == "POST" && r.url.path() == "/cardkit/v1/cards"
            })
            .await;
        let card_create_body: serde_json::Value =
            serde_json::from_slice(&card_create_calls[0].body).unwrap();
        let card_data: serde_json::Value =
            serde_json::from_str(card_create_body["data"].as_str().unwrap()).unwrap();
        assert_eq!(card_data["body"]["elements"][0]["tag"], "markdown");
        assert!(
            card_data["body"]["elements"][0]["content"]
                .as_str()
                .unwrap_or_default()
                .contains("echo:"),
            "card content should contain echo response"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_file_message_routes_local_path_into_agent_thread() {
        let (server, client) = setup_feishu_mock().await;
        Mock::given(method("GET"))
            .and(path(
                "/im/v1/messages/om_test_file_msg_001/resources/file_key_123",
            ))
            .and(query_param("type", "file"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"feishu file bytes".to_vec()))
            .mount(&server)
            .await;

        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let account = make_default_account();

        let event = FeishuEventBuilder::file_dm("ou_user123", "file_key_123", "brief.pdf").build();

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        wait_for_provider_calls(provider.as_ref(), 1).await;

        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].attachments.len(), 1);
        assert!(
            calls[0].attachments[0]
                .path
                .contains("garyx-feishu/inbound/")
        );
        assert!(calls[0].attachments[0].name.ends_with("brief.pdf"));
    }

    #[tokio::test]
    async fn test_e2e_feishu_user_ack_boundary_starts_new_reply_message() {
        let (server, client) = setup_feishu_mock().await;

        let provider = Arc::new(FeishuUserAckBoundaryProvider);
        let bridge = make_bridge_with(provider).await;
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let router = make_router_with_store(store.clone());
        let account = make_default_account();

        let event = FeishuEventBuilder::group("ou_user123", "oc_group456", "hi from group")
            .with_root_id("om_thread_root")
            .build();

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        let reply_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            2,
            |r| r.url.path().contains("/reply"),
        )
        .await;
        let card_create_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            2,
            |r| r.method.as_str() == "POST" && r.url.path() == "/cardkit/v1/cards",
        )
        .await;
        let update_bodies: Vec<Value> = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| {
                r.method.as_str() == "PUT"
                    && r.url.path().starts_with("/cardkit/v1/cards/")
                    && r.url.path().ends_with("/elements/content/content")
            },
        )
        .await
        .into_iter()
        .map(|r| serde_json::from_slice(&r.body).expect("valid cardkit update body"))
        .collect();
        let close_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.method.as_str() == "PATCH" && r.url.path().ends_with("/settings"),
        )
        .await;

        assert!(
            reply_calls.len() >= 2,
            "user ack boundary should start a fresh Feishu reply message"
        );
        assert!(
            card_create_calls.len() >= 2,
            "user ack boundary should start a fresh Feishu streaming card after the boundary"
        );
        assert!(
            update_bodies.iter().any(|body| body["content"]
                .as_str()
                .unwrap_or_default()
                .contains("第二段")),
            "second segment should close the new streaming card with the second segment text; update_bodies={update_bodies:?}"
        );
        assert!(
            !close_calls.is_empty(),
            "final segment should close the Feishu Card Kit stream"
        );

        let _ = store;
    }

    #[tokio::test]
    async fn test_e2e_feishu_assistant_segment_keeps_single_reply_message() {
        let (server, client) = setup_feishu_mock().await;

        let provider = Arc::new(FeishuAssistantSegmentProvider);
        let bridge = make_bridge_with(provider).await;
        let router = make_router();
        let account = make_default_account();

        let event = FeishuEventBuilder::group("ou_user123", "oc_group456", "hi from group")
            .with_root_id("om_thread_root")
            .build();

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        let reply_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("/reply"),
        )
        .await;
        let update_bodies: Vec<Value> = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| {
                r.method.as_str() == "PUT"
                    && r.url.path().starts_with("/cardkit/v1/cards/")
                    && r.url.path().ends_with("/elements/content/content")
            },
        )
        .await
        .into_iter()
        .map(|r| serde_json::from_slice(&r.body).expect("valid cardkit update body"))
        .collect();
        let close_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.method.as_str() == "PATCH" && r.url.path().ends_with("/settings"),
        )
        .await;

        assert_eq!(
            reply_calls.len(),
            1,
            "assistant segment must stay within a single Feishu reply message"
        );
        assert!(
            update_bodies.iter().any(|body| {
                body["content"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("第一段\n\n第二段")
            }),
            "assistant segment should keep updating the same Feishu Card Kit body with inline separation; update_bodies={update_bodies:?}"
        );
        assert!(
            !close_calls.is_empty(),
            "assistant segment should close the existing Feishu Card Kit stream in place"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_tool_trace_then_assistant_keeps_single_reply_message() {
        let (server, client) = setup_feishu_mock().await;

        let provider = Arc::new(FeishuToolThenAssistantProvider);
        let bridge = make_bridge_with(provider).await;
        let router = make_router();
        let account = make_default_account();

        let event = FeishuEventBuilder::group("ou_user123", "oc_group456", "hi from group")
            .with_root_id("om_thread_root")
            .build();

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        let reply_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("/reply"),
        )
        .await;
        let update_bodies: Vec<Value> = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| {
                r.method.as_str() == "PUT"
                    && r.url.path().starts_with("/cardkit/v1/cards/")
                    && r.url.path().ends_with("/elements/content/content")
            },
        )
        .await
        .into_iter()
        .map(|r| serde_json::from_slice(&r.body).expect("valid cardkit update body"))
        .collect();
        let close_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.method.as_str() == "PATCH" && r.url.path().ends_with("/settings"),
        )
        .await;

        assert_eq!(
            reply_calls.len(),
            1,
            "tool trace should not fan out assistant continuation into a new Feishu reply"
        );
        assert!(
            update_bodies.iter().any(|body| {
                body["content"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("先说一句，再接一句")
            }),
            "assistant continuation after tool trace should stay in the same Feishu Card Kit body; update_bodies={update_bodies:?}"
        );
        assert!(
            !close_calls.is_empty(),
            "assistant continuation after tool trace should close the existing Feishu Card Kit stream"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_image_generation_result_sends_image_only() {
        let (server, client) = setup_feishu_mock().await;

        let provider = Arc::new(FeishuImageGenerationProvider);
        let bridge = make_bridge_with(provider).await;
        let router = make_router();
        let account = make_default_account();

        let event = FeishuEventBuilder::group("ou_user123", "oc_group456", "make image")
            .with_root_id("om_thread_root")
            .build();

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        let upload_calls =
            wait_for_matching_requests(&server, std::time::Duration::from_secs(5), 1, |r| {
                r.method.as_str() == "POST" && r.url.path() == "/im/v1/images"
            })
            .await;
        assert_eq!(
            upload_calls.len(),
            1,
            "generated image should be uploaded once"
        );

        let reply_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| {
                r.url.path().contains("/reply")
                    && std::str::from_utf8(&r.body)
                        .map(|body| body.contains("\"msg_type\":\"image\""))
                        .unwrap_or(false)
            },
        )
        .await;
        assert_eq!(
            reply_calls.len(),
            1,
            "imageGeneration should produce one Feishu image reply"
        );
        let reply_body: Value = serde_json::from_slice(&reply_calls[0].body).unwrap();
        assert_eq!(reply_body["msg_type"], "image");
        let content: Value = serde_json::from_str(reply_body["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["image_key"], "img_mock_generated");

        let requests = server.received_requests().await.unwrap_or_default();
        let card_calls = requests
            .iter()
            .filter(|r| r.url.path().starts_with("/cardkit/v1/cards"))
            .count();
        assert_eq!(
            card_calls, 0,
            "image-only tool results should not render tool text or Card Kit messages"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_group_reply() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        let event = FeishuEventBuilder::group("ou_user123", "oc_group456", "hi from group")
            .with_root_id("om_thread_root")
            .build();

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        // Provider called with the correct group thread
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].thread_id.starts_with("thread::"));
        drop(calls);

        // reply_message should be called (not send_message) for groups
        let reply_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("/reply"),
        )
        .await;
        assert!(
            !reply_calls.is_empty(),
            "reply_message should be called for group messages"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_group_reply_does_not_enter_topic_when_topic_mode_disabled() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let mut account = make_default_account();
        account.topic_session_mode = TopicSessionMode::Disabled;

        let event = FeishuEventBuilder::group("ou_user123", "oc_group456", "hi from group")
            .with_root_id("om_thread_root")
            .build();

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;

        let reply_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("/reply"),
        )
        .await;
        let body: Value =
            serde_json::from_slice(&reply_calls[0].body).expect("valid Feishu reply body");
        assert!(
            body.get("reply_in_thread").is_none(),
            "topic mode disabled should keep replies in the main group chat: {body:?}"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_group_reply_stays_in_main_chat_when_topic_mode_enabled() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let mut account = make_default_account();
        account.topic_session_mode = TopicSessionMode::Enabled;

        let event = FeishuEventBuilder::group("ou_user123", "oc_group456", "hi from group")
            .with_root_id("om_thread_root")
            .build();

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;

        let reply_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("/reply"),
        )
        .await;
        let body: Value =
            serde_json::from_slice(&reply_calls[0].body).expect("valid Feishu reply body");
        assert!(
            body.get("reply_in_thread").is_none(),
            "ordinary group replies should stay in the main chat even when topic mode is enabled: {body:?}"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_group_root_changes_split_topic_scope_when_enabled() {
        let (_server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let mut account = make_default_account();
        account.topic_session_mode = TopicSessionMode::Enabled;

        let first = FeishuEventBuilder::group("ou_user123", "oc_group456", "first root")
            .with_message_id("om_group_scope_001")
            .with_root_id("om_root_a")
            .build();
        dispatch_im_message_event(
            "app1",
            &first,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        let second = FeishuEventBuilder::group("ou_user123", "oc_group456", "second root")
            .with_message_id("om_group_scope_002")
            .with_root_id("om_root_b")
            .build();
        dispatch_im_message_event(
            "app1",
            &second,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 2).await;

        let calls = provider.calls.lock().unwrap();
        assert_ne!(
            calls[0].thread_id, calls[1].thread_id,
            "different root ids in the same group should use different topic scopes when topic mode is enabled"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_reply_routing() {
        let (_server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        // Step 1: Send first DM message
        let event1 = FeishuEventBuilder::dm("ou_user123", "first msg").build();
        dispatch_im_message_event(
            "app1",
            &event1,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;
        let initial_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };
        let thread_from_reply = {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let resolved = {
                    let r = router.lock().await;
                    r.resolve_reply_thread_for_chat(
                        "feishu",
                        "app1",
                        Some("oc_dm_ou_user123"),
                        None,
                        "om_mock_reply_dm",
                    )
                    .map(|s| s.to_owned())
                };
                if resolved.is_some() {
                    break resolved;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "outbound message should be recorded for reply routing"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        };
        assert_eq!(
            thread_from_reply.as_deref(),
            Some(initial_thread.as_str()),
            "outbound message should be recorded for reply routing"
        );

        // Step 2: Send second message as a reply to the bot's message
        let event2 = FeishuEventBuilder::dm("ou_user123", "follow up")
            .with_message_id("om_test_msg_002")
            .with_parent_id("om_mock_reply_dm")
            .build();
        dispatch_im_message_event(
            "app1",
            &event2,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 2).await;

        // Both should route to the same thread
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, calls[1].thread_id);
        assert!(calls[0].thread_id.starts_with("thread::"));
    }

    #[tokio::test]
    async fn test_e2e_feishu_reply_routing_after_endpoint_rebind_keeps_old_thread_without_switching_current()
     {
        let (_server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let account = make_default_account();

        let event1 = FeishuEventBuilder::dm("ou_user123", "first msg")
            .with_message_id("om_test_msg_rebind_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &event1,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_provider_calls(provider.as_ref(), 1).await;

        let old_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };
        wait_for_thread_delivery_persistence(&store, &old_thread).await;

        let (new_thread, _) = create_thread_record(
            &store,
            ThreadEnsureOptions {
                label: Some("Rebound".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("thread should be created");
        bind_endpoint_to_thread(
            &store,
            &new_thread,
            ChannelBinding {
                channel: "feishu".to_owned(),
                account_id: "app1".to_owned(),
                binding_key: "ou_user123".to_owned(),
                chat_id: "oc_dm_ou_user123".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "oc_dm_ou_user123".to_owned(),
                display_label: "ou_user123".to_owned(),
                last_inbound_at: None,
                last_delivery_at: None,
            },
        )
        .await
        .expect("bind should succeed");
        {
            let mut router_guard = router.lock().await;
            router_guard.rebuild_thread_indexes().await;
        }

        let event2 = FeishuEventBuilder::dm("ou_user123", "follow old thread")
            .with_message_id("om_test_msg_rebind_002")
            .with_parent_id("om_mock_reply_dm")
            .build();
        dispatch_im_message_event(
            "app1",
            &event2,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_provider_calls(provider.as_ref(), 2).await;
        wait_for_provider_calls(provider.as_ref(), 2).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[1].thread_id, old_thread);
        drop(calls);

        let current = {
            let router_guard = router.lock().await;
            router_guard
                .get_current_thread_id_for_binding("feishu", "app1", "ou_user123")
                .map(str::to_owned)
        };
        assert_eq!(current, None);
        let rebound = {
            let mut router_guard = router.lock().await;
            router_guard
                .resolve_endpoint_thread_id("feishu", "app1", "ou_user123")
                .await
        };
        assert_eq!(rebound.as_deref(), Some(new_thread.as_str()));
    }

    #[tokio::test]
    async fn test_e2e_feishu_reply_routing_after_router_restart() {
        let (_server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let account = make_default_account();

        let event1 = FeishuEventBuilder::dm("ou_user123", "first msg")
            .with_message_id("om_test_msg_restart_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &event1,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_provider_calls(provider.as_ref(), 1).await;
        let first_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };
        wait_for_thread_delivery_persistence(&store, &first_thread).await;

        // Simulate router restart: rebuild reply index and delivery cache from persisted store.
        let restarted_router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        {
            let mut guard = restarted_router.lock().await;
            assert_eq!(guard.rebuild_routing_index("feishu").await, 1);
            assert!(guard.rebuild_last_delivery_cache().await >= 1);
        }

        let event2 = FeishuEventBuilder::dm("ou_user123", "follow up after restart")
            .with_message_id("om_test_msg_restart_002")
            .with_parent_id("om_mock_reply_dm")
            .build();
        dispatch_im_message_event(
            "app1",
            &event2,
            &restarted_router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_provider_calls(provider.as_ref(), 2).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].thread_id.starts_with("thread::"));
        assert_eq!(calls[0].thread_id, calls[1].thread_id);
    }

    #[tokio::test]
    async fn test_e2e_feishu_sessionprev_rebuilds_history_after_restart() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let account = make_default_account();

        store
            .set(
                "app1::main::ou_user123_a",
                serde_json::json!({
                    "from_id": "ou_user123",
                    "updated_at": "2026-03-01T10:00:00Z"
                }),
            )
            .await;
        store
            .set(
                "app1::main::ou_user123_b",
                serde_json::json!({
                    "from_id": "ou_user123",
                    "updated_at": "2026-03-01T11:00:00Z"
                }),
            )
            .await;
        store
            .set(
                "app1::main::ou_user123_c",
                serde_json::json!({
                    "from_id": "ou_user123",
                    "updated_at": "2026-03-01T12:00:00Z"
                }),
            )
            .await;

        let command = FeishuEventBuilder::dm("ou_user123", "/threadprev")
            .with_message_id("om_cmd_prev_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &command,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 0);
        let command_reqs = server.received_requests().await.unwrap();
        let switched_notice: Vec<_> = command_reqs
            .iter()
            .filter(|r| {
                r.url.path() == "/im/v1/messages/om_cmd_prev_001/reply"
                    && r.method.as_str() == "POST"
                    && std::str::from_utf8(&r.body)
                        .map(|body| {
                            body.contains("Switched to previous thread: app1::main::ou_user123_b")
                        })
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(switched_notice.len(), 1);

        let normal = FeishuEventBuilder::dm("ou_user123", "after restart command")
            .with_message_id("om_cmd_prev_002")
            .build();
        dispatch_im_message_event(
            "app1",
            &normal,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, "app1::main::ou_user123_b");
    }

    #[tokio::test]
    async fn test_e2e_feishu_sessions_lists_named_sessions_with_current_marker() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let account = make_default_account();

        {
            let mut router_guard = router.lock().await;
            router_guard
                .ensure_thread_entry(
                    "app1::main::ou_user123:thread-a",
                    "feishu",
                    "app1",
                    "ou_user123",
                    Some("thread-a"),
                )
                .await;
            router_guard
                .ensure_thread_entry(
                    "app1::main::ou_user123:thread-b",
                    "feishu",
                    "app1",
                    "ou_user123",
                    Some("thread-b"),
                )
                .await;
            let user_key = MessageRouter::build_binding_context_key("feishu", "app1", "ou_user123");
            router_guard.switch_to_thread(&user_key, "app1::main::ou_user123:thread-b");
        }

        let command = FeishuEventBuilder::dm("ou_user123", "/threads")
            .with_message_id("om_cmd_sessions_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &command,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 0);
        let requests = server.received_requests().await.unwrap();
        let sessions_reply: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.url.path() == "/im/v1/messages/om_cmd_sessions_001/reply"
                    && r.method.as_str() == "POST"
                    && std::str::from_utf8(&r.body)
                        .map(|body| {
                            body.contains("Your Threads:")
                                && body.contains("thread-a")
                                && body.contains("thread-b ⬅️")
                        })
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(sessions_reply.len(), 1);
    }

    #[tokio::test]
    async fn test_e2e_feishu_command_new_switches_to_named_session() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let account = make_default_account();

        let command = FeishuEventBuilder::dm("ou_user123", "/newthread")
            .with_message_id("om_cmd_new_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &command,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 0);
        let switched = {
            let router_guard = router.lock().await;
            router_guard
                .get_current_thread_id_for_binding("feishu", "app1", "ou_user123")
                .map(|s| s.to_owned())
        };
        assert!(
            switched
                .as_deref()
                .is_some_and(|s| s.starts_with("thread::")),
            "should switch to canonical thread, got {:?}",
            switched
        );
        let thread_data = store
            .get(switched.as_deref().expect("thread id should exist"))
            .await
            .expect("created thread should persist");
        assert!(
            thread_data["label"]
                .as_str()
                .unwrap_or_default()
                .starts_with("thread-"),
            "created thread should keep generated thread label"
        );

        let requests = server.received_requests().await.unwrap();
        let new_notice: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.url.path() == "/im/v1/messages/om_cmd_new_001/reply"
                    && r.method.as_str() == "POST"
                    && std::str::from_utf8(&r.body)
                        .map(|body| body.contains("Created and switched to new thread: thread-"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(new_notice.len(), 1);
    }

    #[tokio::test]
    async fn test_e2e_feishu_group_command_new_switches_to_group_topic_thread() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let mut account = make_default_account();
        account.topic_session_mode = TopicSessionMode::Enabled;

        let command = FeishuEventBuilder::group("ou_user123", "oc_group456", "/newthread")
            .with_message_id("om_cmd_new_group_001")
            .with_root_id("om_thread_root")
            .build();
        dispatch_im_message_event(
            "app1",
            &command,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 0);
        let group_id = "oc_group456:topic:om_thread_root";
        let switched = {
            let router_guard = router.lock().await;
            router_guard
                .get_current_thread_id_for_binding("feishu", "app1", group_id)
                .map(|s| s.to_owned())
        };
        assert!(
            switched
                .as_deref()
                .is_some_and(|s| s.starts_with("thread::")),
            "should switch to canonical thread, got {:?}",
            switched
        );
        let thread_data = store
            .get(switched.as_deref().expect("thread id should exist"))
            .await
            .expect("created thread should persist");
        assert!(
            thread_data["label"]
                .as_str()
                .unwrap_or_default()
                .starts_with("thread-"),
            "created thread should keep generated thread label"
        );

        let requests = server.received_requests().await.unwrap();
        let new_notice: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.url.path() == "/im/v1/messages/om_cmd_new_group_001/reply"
                    && r.method.as_str() == "POST"
                    && std::str::from_utf8(&r.body)
                        .map(|body| body.contains("Created and switched to new thread: thread-"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(new_notice.len(), 1);
    }

    #[tokio::test]
    async fn test_e2e_feishu_bind_detach_retargets_next_dm() {
        let (_server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let account = make_default_account();

        let first = FeishuEventBuilder::dm("ou_user123", "first bound thread")
            .with_message_id("om_bind_detach_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &first,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let first_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };
        assert!(first_thread.starts_with("thread::"));

        let detached = detach_endpoint_from_thread(&store, "feishu::app1::ou_user123")
            .await
            .expect("detach should succeed");
        assert_eq!(detached.as_deref(), Some(first_thread.as_str()));
        {
            let mut router_guard = router.lock().await;
            router_guard.rebuild_thread_indexes().await;
        }

        let second = FeishuEventBuilder::dm("ou_user123", "second rebound thread")
            .with_message_id("om_bind_detach_002")
            .build();
        dispatch_im_message_event(
            "app1",
            &second,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let second_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[1].thread_id.clone()
        };
        assert!(second_thread.starts_with("thread::"));
        assert_ne!(second_thread, first_thread);

        bind_endpoint_to_thread(
            &store,
            &first_thread,
            ChannelBinding {
                channel: "feishu".to_owned(),
                account_id: "app1".to_owned(),
                binding_key: "ou_user123".to_owned(),
                chat_id: "oc_dm_ou_user123".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "oc_dm_ou_user123".to_owned(),
                display_label: "ou_user123".to_owned(),
                last_inbound_at: None,
                last_delivery_at: None,
            },
        )
        .await
        .expect("bind should succeed");
        {
            let mut router_guard = router.lock().await;
            router_guard.rebuild_thread_indexes().await;
        }

        let third = FeishuEventBuilder::dm("ou_user123", "third back to first")
            .with_message_id("om_bind_detach_003")
            .build();
        dispatch_im_message_event(
            "app1",
            &third,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let third_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[2].thread_id.clone()
        };
        assert_eq!(third_thread, first_thread);

        let thread_keys: Vec<String> = store
            .list_keys(None)
            .await
            .into_iter()
            .filter(|key| is_thread_key(key))
            .collect();
        assert_eq!(thread_keys.len(), 2);
    }

    #[tokio::test]
    async fn test_e2e_feishu_detach_clears_explicit_session_override() {
        let (_server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let account = make_default_account();

        let first = FeishuEventBuilder::dm("ou_user123", "first bound thread")
            .with_message_id("om_bind_detach_override_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &first,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let first_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };

        {
            let mut router_guard = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("feishu", "app1", "ou_user123");
            router_guard.switch_to_thread(&user_key, &first_thread);
        }

        detach_endpoint_from_thread(&store, "feishu::app1::ou_user123")
            .await
            .expect("detach should succeed");
        {
            let mut router_guard = router.lock().await;
            router_guard.rebuild_thread_indexes().await;
        }

        let second = FeishuEventBuilder::dm("ou_user123", "after detach should not stick")
            .with_message_id("om_bind_detach_override_002")
            .build();
        dispatch_im_message_event(
            "app1",
            &second,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let second_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[1].thread_id.clone()
        };
        assert_ne!(second_thread, first_thread);
    }

    #[tokio::test]
    async fn test_e2e_feishu_reply_routing_switches_scheduled_thread() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        {
            let mut router_guard = router.lock().await;
            router_guard.record_outbound_message(
                "cron::daily_summary",
                "feishu",
                "app1",
                "om_cron_reply_001",
            );
        }

        let event = FeishuEventBuilder::dm("ou_user123", "follow scheduled context")
            .with_message_id("om_test_msg_switch_001")
            .with_parent_id("om_cron_reply_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, "cron::daily_summary");
        drop(calls);

        let switched_thread = {
            let router_guard = router.lock().await;
            router_guard
                .get_current_thread_id_for_binding("feishu", "app1", "ou_user123")
                .map(|s| s.to_owned())
        };
        assert_eq!(switched_thread.as_deref(), Some("cron::daily_summary"));

        let requests = server.received_requests().await.unwrap();
        let notice_calls: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.url.path() == "/im/v1/messages/om_test_msg_switch_001/reply"
                    && r.method.as_str() == "POST"
                    && std::str::from_utf8(&r.body)
                        .map(|body| body.contains("你已经切换到 thread:cron::daily_summary"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            notice_calls.len(),
            1,
            "should send thread switch notice once"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_group_reply_routing_scheduled_switch_scoped_to_group() {
        let (_server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        {
            let mut router_guard = router.lock().await;
            router_guard.record_outbound_message(
                "cron::daily_summary",
                "feishu",
                "app1",
                "om_cron_reply_group_001",
            );
        }

        let event = FeishuEventBuilder::group(
            "ou_user123",
            "oc_group456",
            "group follow scheduled context",
        )
        .with_message_id("om_test_msg_switch_group_001")
        .with_parent_id("om_cron_reply_group_001")
        .build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, "cron::daily_summary");
        drop(calls);

        let (group_switched, dm_switched) = {
            let router_guard = router.lock().await;
            let group_switched = router_guard
                .get_current_thread_id_for_binding("feishu", "app1", "oc_group456")
                .map(|s| s.to_owned());
            let dm_switched = router_guard
                .get_current_thread_id_for_binding("feishu", "app1", "ou_user123")
                .map(|s| s.to_owned());
            (group_switched, dm_switched)
        };

        assert_eq!(group_switched.as_deref(), Some("cron::daily_summary"));
        assert_eq!(dm_switched, None);
    }

    #[tokio::test]
    async fn test_e2e_feishu_reply_routing_switches_cron_session() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        {
            let mut router_guard = router.lock().await;
            router_guard.record_outbound_message(
                "cron::daily::ou_user123",
                "feishu",
                "app1",
                "om_cron_reply_001",
            );
        }

        let event = FeishuEventBuilder::dm("ou_user123", "follow scheduled context")
            .with_message_id("om_test_cron_switch_001")
            .with_parent_id("om_cron_reply_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, "cron::daily::ou_user123");
        drop(calls);

        let switched_thread = {
            let router_guard = router.lock().await;
            router_guard
                .get_current_thread_id_for_binding("feishu", "app1", "ou_user123")
                .map(|s| s.to_owned())
        };
        assert_eq!(switched_thread.as_deref(), Some("cron::daily::ou_user123"));

        let requests = server.received_requests().await.unwrap();
        let notice_calls: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.url.path() == "/im/v1/messages/om_test_cron_switch_001/reply"
                    && r.method.as_str() == "POST"
                    && std::str::from_utf8(&r.body)
                        .map(|body| body.contains("你已经切换到 thread:cron::daily::ou_user123"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(notice_calls.len(), 1);
    }

    #[tokio::test]
    async fn test_e2e_feishu_multi_account_switched_thread_isolated() {
        let (_server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let account = make_default_account();

        let seeded_app1_thread = seed_bound_dm_thread(&store, "app1", "ou_user123", "custom").await;
        {
            let mut router_guard = router.lock().await;
            router_guard.rebuild_thread_indexes().await;
        }

        let event_app2 = FeishuEventBuilder::dm("ou_user123", "hello from app2")
            .with_message_id("om_test_multi_acc_001")
            .build();
        dispatch_im_message_event(
            "app2",
            &event_app2,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        let event_app1 = FeishuEventBuilder::dm("ou_user123", "hello from app1")
            .with_message_id("om_test_multi_acc_002")
            .build();
        dispatch_im_message_event(
            "app1",
            &event_app1,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(800)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
        let calls = provider.calls.lock().unwrap();
        let app2_session = calls
            .iter()
            .find(|call| call.message == "ou_user123: hello from app2")
            .map(|call| call.thread_id.clone())
            .expect("app2 dispatch should exist");
        let app1_session = calls
            .iter()
            .find(|call| call.message == "ou_user123: hello from app1")
            .map(|call| call.thread_id.clone())
            .expect("app1 dispatch should exist");
        assert_eq!(app1_session, seeded_app1_thread);
        assert!(app2_session.starts_with("thread::"));
        assert_ne!(app2_session, seeded_app1_thread);
    }

    #[tokio::test]
    async fn test_e2e_feishu_sender_display_name_prefix() {
        let (server, client) = setup_feishu_mock().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/contact/v3/users/ou_named_user_001$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok",
                "data": {
                    "user": {
                        "name": "Alice"
                    }
                }
            })))
            .mount(&server)
            .await;

        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        let event = FeishuEventBuilder::dm("ou_named_user_001", "hello by name").build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].message, "Alice: hello by name");
    }

    #[tokio::test]
    async fn test_e2e_feishu_reply_quote_fallback() {
        let (server, client) = setup_feishu_mock().await;
        Mock::given(method("GET"))
            .and(path("/im/v1/messages/om_parent_quote_001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok",
                "data": {
                    "items": [{
                        "msg_type": "text",
                        "body": {
                            "content": "{\"text\":\"legacy context\"}"
                        }
                    }]
                }
            })))
            .mount(&server)
            .await;

        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        let event = FeishuEventBuilder::dm("ou_quote_user_001", "follow up")
            .with_parent_id("om_parent_quote_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(
            calls[0].message,
            "[Replying to: \"legacy context\"]\n\nou_quote_user_001: follow up"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_patch_message_fallback_to_update() {
        let (server, client) = setup_feishu_mock().await;
        Mock::given(method("PATCH"))
            .and(path("/im/v1/messages/om_patch_001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 99991672,
                "msg": "patch not allowed"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/im/v1/messages/om_patch_001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok"
            })))
            .mount(&server)
            .await;

        client
            .patch_message_text("om_patch_001", "stream reply")
            .await
            .expect("patch fallback should succeed");

        let requests = server.received_requests().await.unwrap();
        let patch_calls: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.method.as_str() == "PATCH" && r.url.path() == "/im/v1/messages/om_patch_001"
            })
            .collect();
        let put_calls: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.method.as_str() == "PUT" && r.url.path() == "/im/v1/messages/om_patch_001"
            })
            .collect();
        assert_eq!(patch_calls.len(), 1, "patch endpoint should be called once");
        assert_eq!(
            put_calls.len(),
            1,
            "update fallback endpoint should be called once"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_remove_reaction() {
        let (server, client) = setup_feishu_mock().await;
        Mock::given(method("POST"))
            .and(path("/im/v1/messages/om_react_001/reactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok",
                "data": {"reaction_id": "reaction_xyz"}
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/im/v1/messages/om_react_001/reactions/reaction_xyz"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok"
            })))
            .mount(&server)
            .await;

        let reaction_id = client
            .add_reaction("om_react_001", PROCESSING_REACTION_EMOJI)
            .await
            .expect("add reaction should succeed");
        assert_eq!(reaction_id.as_deref(), Some("reaction_xyz"));

        client
            .remove_reaction("om_react_001", reaction_id.as_deref().unwrap_or_default())
            .await
            .expect("remove reaction should succeed");

        let requests = server.received_requests().await.unwrap();
        let delete_calls: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.method.as_str() == "DELETE"
                    && r.url.path() == "/im/v1/messages/om_react_001/reactions/reaction_xyz"
            })
            .collect();
        assert_eq!(
            delete_calls.len(),
            1,
            "reaction delete endpoint should be called"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_permission_error_notifies_with_cooldown() {
        reset_permission_error_notice_cache();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/v3/tenant_access_token/internal"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "tenant_access_token": "t-test-token-abc",
                "expire": 7200,
                "msg": "ok"
            })))
            .mount(&server)
            .await;

        let client = FeishuClient {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            domain: FeishuDomain::Feishu,
            http: HttpClient::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_base_override: Some(server.uri()),
        };
        let permission_msg =
            "permission denied, visit https://open.feishu.cn/appPermission?appId=cli_xxx";
        Mock::given(method("POST"))
            .and(path("/im/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 99991672,
                "msg": permission_msg
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/im/v1/messages/om_perm_msg_001/reply"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0,
                "msg": "ok",
                "data": {"message_id": "om_perm_notice_reply"}
            })))
            .mount(&server)
            .await;

        let provider = Arc::new(ConfigurableTestProvider::with_response(|_| {
            "reply requiring permission".into()
        }));
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        let event = FeishuEventBuilder::dm("ou_user123", "permission flow")
            .with_message_id("om_perm_msg_001")
            .build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;
        wait_for_matching_requests(&server, std::time::Duration::from_secs(5), 1, |r| {
            r.url.path() == "/im/v1/messages/om_perm_msg_001/reply"
                && std::str::from_utf8(&r.body)
                    .map(|body| body.contains("Bot encountered a Feishu API permission error."))
                    .unwrap_or(false)
        })
        .await;

        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 2).await;
        wait_for_matching_requests(&server, std::time::Duration::from_secs(5), 2, |r| {
            r.method.as_str() == "POST" && r.url.path() == "/im/v1/messages"
        })
        .await;

        let requests = wait_for_request_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(2),
            3,
        )
        .await;
        let send_calls: Vec<_> = requests
            .iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/im/v1/messages")
            .collect();
        assert!(
            send_calls.len() >= 2,
            "send_message should be attempted for both events"
        );
        let notice_calls: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.url.path() == "/im/v1/messages/om_perm_msg_001/reply"
                    && std::str::from_utf8(&r.body)
                        .map(|body| body.contains("Bot encountered a Feishu API permission error."))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            notice_calls.len(),
            1,
            "permission notice should be cooldown-limited"
        );
        let notice_body = std::str::from_utf8(&notice_calls[0].body).unwrap_or_default();
        assert!(
            notice_body.contains("appPermission?appId=cli_xxx"),
            "permission notice should include grant URL when provided"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_card_format() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::with_response(|_| {
            "Hello **bold** text".to_string()
        }));
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        let event = FeishuEventBuilder::dm("ou_user123", "test card").build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;

        // Find the send_message request
        let send_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.method.as_str() == "POST" && r.url.path() == "/im/v1/messages",
        )
        .await;
        assert!(!send_calls.is_empty());

        let send_body: serde_json::Value = serde_json::from_slice(&send_calls[0].body).unwrap();
        let send_content: serde_json::Value =
            serde_json::from_str(send_body["content"].as_str().unwrap()).unwrap();
        assert_eq!(send_body["msg_type"], "interactive");
        assert_eq!(send_content["type"], "card");
        assert_eq!(send_content["data"]["card_id"], "cardkit_mock_card");

        let card_create_calls = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.method.as_str() == "POST" && r.url.path() == "/cardkit/v1/cards",
        )
        .await;
        let card_create_body: serde_json::Value =
            serde_json::from_slice(&card_create_calls[0].body).unwrap();
        let card_data: serde_json::Value =
            serde_json::from_str(card_create_body["data"].as_str().unwrap()).unwrap();
        assert_eq!(card_data["body"]["elements"][0]["tag"], "markdown");
        assert_eq!(
            card_data["body"]["elements"][0]["content"],
            "Hello **bold** text"
        );
    }

    #[tokio::test]
    async fn test_e2e_feishu_token_refresh() {
        let (server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let account = make_default_account();

        // Client has no token — should trigger refresh before sending
        assert!(client.token_state.read().await.is_none());

        let event = FeishuEventBuilder::dm("ou_user123", "test token").build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;

        let _token_calls =
            wait_for_matching_requests(&server, std::time::Duration::from_secs(5), 1, |r| {
                r.url.path().contains("tenant_access_token")
            })
            .await;
        let send_calls =
            wait_for_matching_requests(&server, std::time::Duration::from_secs(5), 1, |r| {
                r.method.as_str() == "POST" && r.url.path() == "/im/v1/messages"
            })
            .await;

        let auth_header = send_calls[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert_eq!(auth_header, "Bearer t-test-token-abc");
    }

    #[tokio::test]
    async fn test_e2e_feishu_session_persistence() {
        let (server, client) = setup_feishu_mock().await;
        let _server = server; // Keep server alive for the duration of the test
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> =
            Arc::new(garyx_router::InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let account = make_default_account();

        let event = FeishuEventBuilder::dm("ou_user123", "persist this").build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Verify thread store has user + assistant messages
        let thread_id = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };
        let data = store.get(&thread_id).await;
        assert!(data.is_some(), "thread data should be persisted");
        let data = data.unwrap();
        let messages = data["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2, "should have user + assistant messages");
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "ou_user123: persist this");
        assert_eq!(messages[1]["role"], "assistant");
        assert!(messages[1]["content"].as_str().unwrap().contains("echo:"));
        assert!(thread_id.starts_with("thread::"));
    }

    // ---------------------------------------------------------------
    // Feishu group behavior E2E tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_e2e_require_mention_blocks_no_mention() {
        let (_server, client) = setup_feishu_mock().await;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router_with_store(Arc::new(InMemoryThreadStore::new()));
        let mut account = make_default_account();
        account.require_mention = true;

        let event = FeishuEventBuilder::group("ou_user", "oc_group1", "hi everyone").build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            0,
            "no mention should be blocked"
        );
        let records = {
            let router = router.lock().await;
            router
                .list_message_ledger_records_for_bot("feishu:app1", 20)
                .await
        };
        let blocked = records.iter().find(|record| {
            record.text_excerpt.as_deref() == Some("hi everyone")
                && record.status == garyx_models::MessageLifecycleStatus::Filtered
                && record.terminal_reason
                    == Some(garyx_models::MessageTerminalReason::RoutingRejected)
                && record
                    .metadata
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("mention_required")
        });
        assert!(
            blocked.is_some(),
            "missing mention should produce a filtered routing-rejected ledger record: {records:#?}"
        );
    }

    #[tokio::test]
    async fn test_e2e_require_mention_allows_with_mention() {
        let (server, client) = setup_feishu_mock().await;
        let _server = server;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let mut account = make_default_account();
        account.require_mention = true;

        let event = FeishuEventBuilder::group("ou_user", "oc_group1", "@_user_1 hello")
            .with_mention("@_user_1", "Bot", "test_app")
            .build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            1,
            "message with bot mention should be allowed"
        );
    }

    #[tokio::test]
    async fn test_e2e_topic_session_mode_enabled_uses_topic_scope() {
        let (server, client) = setup_feishu_mock().await;
        let _server = server;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let mut account = make_default_account();
        account.topic_session_mode = TopicSessionMode::Enabled;

        let event = FeishuEventBuilder::group("ou_user", "oc_group1", "topic msg")
            .with_root_id("om_root_123")
            .build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].thread_id.starts_with("thread::"));
        assert_eq!(
            calls[0].metadata.get("thread_id").and_then(Value::as_str),
            Some("oc_group1:topic:om_root_123"),
        );
    }

    #[tokio::test]
    async fn test_e2e_topic_session_mode_disabled_keeps_group_scope() {
        let (server, client) = setup_feishu_mock().await;
        let _server = server;
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let mut account = make_default_account();
        account.topic_session_mode = TopicSessionMode::Disabled;

        let event = FeishuEventBuilder::group("ou_user", "oc_group1", "topic msg")
            .with_root_id("om_root_123")
            .build();
        dispatch_im_message_event(
            "app1",
            &event,
            &router,
            &bridge,
            &client,
            &account,
            "",
            &account.app_id,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].thread_id.starts_with("thread::"));
        assert_eq!(
            calls[0].metadata.get("thread_id").and_then(Value::as_str),
            Some("oc_group1"),
        );
    }
}

// -----------------------------------------------------------------------
// Unit tests for policy functions
// -----------------------------------------------------------------------

mod policy_tests {
    use super::*;

    fn make_account() -> FeishuAccount {
        FeishuAccount {
            app_id: "test_app".into(),
            app_secret: "test_secret".into(),
            enabled: true,
            domain: FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: TopicSessionMode::Disabled,
        }
    }

    #[test]
    fn test_dm_messages_are_allowed_by_default() {
        assert!(is_dm_message_allowed());
    }

    #[test]
    fn test_group_messages_are_allowed_by_default() {
        assert!(is_group_message_allowed());
    }

    #[test]
    fn test_requires_mention_uses_account_flag() {
        let mut account = make_account();
        account.require_mention = true;
        assert!(requires_group_mention(&account));

        account.require_mention = false;
        assert!(!requires_group_mention(&account));
    }

    #[test]
    fn test_topic_session_mode_uses_account_flag() {
        let mut account = make_account();
        account.topic_session_mode = TopicSessionMode::Disabled;
        assert_eq!(
            resolve_topic_session_mode(&account),
            TopicSessionMode::Disabled
        );

        account.topic_session_mode = TopicSessionMode::Enabled;
        assert_eq!(
            resolve_topic_session_mode(&account),
            TopicSessionMode::Enabled
        );
    }

    #[test]
    fn test_mention_context_limit_truncates() {
        let mut history: Vec<String> = (0..10).map(|i| format!("msg{}", i)).collect();
        apply_mention_context_limit(&mut history, 5);
        assert_eq!(history.len(), 5);
        assert_eq!(history[0], "msg5");
        assert_eq!(history[4], "msg9");
    }

    #[test]
    fn test_mention_context_limit_no_truncation_needed() {
        let mut history: Vec<String> = vec!["a".into(), "b".into()];
        apply_mention_context_limit(&mut history, 5);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_mention_context_limit_zero_no_op() {
        let mut history: Vec<String> = vec!["a".into(), "b".into()];
        apply_mention_context_limit(&mut history, 0);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_mention_context_limit_negative_no_op() {
        let mut history: Vec<String> = vec!["a".into(), "b".into()];
        apply_mention_context_limit(&mut history, -1);
        assert_eq!(history.len(), 2);
    }
}
