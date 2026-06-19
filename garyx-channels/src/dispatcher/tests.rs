use super::*;
use garyx_models::ChannelOutboundContent;
use garyx_models::config::{DiscordAccount, discord_account_to_plugin_entry};
use garyx_models::provider::{ProviderMessage, StreamBoundaryKind};
use garyx_models::routing::DELIVERY_TARGET_TYPE_CHAT_ID;
use garyx_router::{InMemoryThreadStore, MessageRouter};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct RecordingStreamDispatcher {
    calls: std::sync::Mutex<Vec<OutboundMessage>>,
}

impl RecordingStreamDispatcher {
    fn calls(&self) -> Vec<OutboundMessage> {
        self.calls
            .lock()
            .expect("recording stream dispatcher lock poisoned")
            .clone()
    }
}

#[async_trait::async_trait]
impl ChannelDispatcher for RecordingStreamDispatcher {
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        self.calls
            .lock()
            .expect("recording stream dispatcher lock poisoned")
            .push(request);
        Ok(SendMessageResult {
            message_ids: vec!["msg-1".to_owned()],
        })
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        Vec::new()
    }
}

fn test_stream_target() -> StreamingDispatchTarget {
    StreamingDispatchTarget {
        target_thread_id: "thread::bound".to_owned(),
        channel: "test-channel".to_owned(),
        account_id: "bot1".to_owned(),
        chat_id: "chat1".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "chat1".to_owned(),
        thread_id: Some("topic1".to_owned()),
    }
}

fn test_message_router() -> Arc<Mutex<MessageRouter>> {
    let store = Arc::new(InMemoryThreadStore::new());
    Arc::new(Mutex::new(MessageRouter::new(
        store,
        garyx_models::config::GaryxConfig::default(),
    )))
}

async fn wait_for_stream_calls(
    dispatcher: &RecordingStreamDispatcher,
    expected: usize,
) -> Vec<OutboundMessage> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let calls = dispatcher.calls();
            if calls.len() >= expected {
                break calls;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("stream callback should send outbound messages")
}

#[tokio::test]
async fn outbound_stream_callback_flushes_assistant_segments() {
    let dispatcher = Arc::new(RecordingStreamDispatcher::default());
    let callback = build_outbound_stream_callback(
        dispatcher.clone(),
        test_stream_target(),
        test_message_router(),
    );

    callback(StreamEvent::Delta {
        text: "first".to_owned(),
    });
    callback(StreamEvent::Boundary {
        kind: StreamBoundaryKind::AssistantSegment,
        pending_input_id: None,
    });
    callback(StreamEvent::Delta {
        text: "second".to_owned(),
    });
    callback(StreamEvent::Done);

    let calls = wait_for_stream_calls(&dispatcher, 2).await;

    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].content.as_text(), Some("first"));
    assert_eq!(calls[1].content.as_text(), Some("second"));
    assert_eq!(calls[0].thread_id.as_deref(), Some("topic1"));
}

#[tokio::test]
async fn outbound_stream_callback_flushes_text_before_structured_events() {
    let dispatcher = Arc::new(RecordingStreamDispatcher::default());
    let callback = build_outbound_stream_callback(
        dispatcher.clone(),
        test_stream_target(),
        test_message_router(),
    );
    let message = ProviderMessage::tool_use(
        serde_json::json!({"name": "shell"}),
        Some("tool-1".to_owned()),
        Some("shell".to_owned()),
    );

    callback(StreamEvent::Delta {
        text: "before tool".to_owned(),
    });
    callback(StreamEvent::ToolUse {
        message: message.clone(),
    });

    let calls = wait_for_stream_calls(&dispatcher, 2).await;

    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].content.as_text(), Some("before tool"));
    assert_eq!(
        calls[1].content,
        ChannelOutboundContent::ToolUse { message }
    );
}

#[test]
fn test_empty_dispatcher_has_no_channels() {
    let dispatcher = ChannelDispatcherImpl::new();
    assert!(dispatcher.available_channels().is_empty());
}

#[test]
fn test_register_telegram_sender() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(TelegramSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: "https://api.telegram.org".to_string(),
        is_running: true,
    });

    let channels = dispatcher.available_channels();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].channel, "telegram");
    assert_eq!(channels[0].account_id, "main");
    assert!(channels[0].is_running);
}

#[test]
fn test_register_feishu_sender() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_feishu(FeishuSender::new(
        "bot1".to_string(),
        "app123".to_string(),
        "secret".to_string(),
        "https://open.feishu.cn/open-apis".to_string(),
        true,
    ));

    let channels = dispatcher.available_channels();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].channel, "feishu");
    assert_eq!(channels[0].account_id, "bot1");
}

#[test]
fn test_register_weixin_sender() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_weixin(WeixinSender {
        account_id: "wx-main".to_string(),
        account: garyx_models::config::WeixinAccount {
            token: "token".to_string(),
            uin: "MTIz".to_string(),
            enabled: true,
            base_url: "https://ilinkai.weixin.qq.com".to_string(),
            name: None,
            agent_id: "claude".to_string(),
            workspace_dir: None,
            streaming_update: true,
        },
        http: Client::new(),
        is_running: true,
    });

    let channels = dispatcher.available_channels();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].channel, "weixin");
    assert_eq!(channels[0].account_id, "wx-main");
}

#[test]
fn test_register_discord_sender() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_discord(DiscordSender {
        account_id: "discord-main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: "https://discord.com/api/v10".to_string(),
        is_running: true,
    });

    let channels = dispatcher.available_channels();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].channel, "discord");
    assert_eq!(channels[0].account_id, "discord-main");
    assert!(channels[0].is_running);
}

#[test]
fn test_from_config_registers_weixin_account() {
    let mut channels = ChannelsConfig::default();
    channels.plugin_channel_mut("weixin").accounts.insert(
        "wx-main".to_string(),
        garyx_models::config::weixin_account_to_plugin_entry(
            &garyx_models::config::WeixinAccount {
                token: "token".to_string(),
                uin: "MTIz".to_string(),
                enabled: true,
                base_url: "https://ilinkai.weixin.qq.com".to_string(),
                name: None,
                agent_id: "claude".to_string(),
                workspace_dir: None,
                streaming_update: true,
            },
        ),
    );

    let dispatcher = ChannelDispatcherImpl::from_config(&channels);
    let available = dispatcher.available_channels();
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].channel, "weixin");
    assert_eq!(available[0].account_id, "wx-main");
}

#[test]
fn test_from_config_registers_discord_account() {
    let mut channels = ChannelsConfig::default();
    channels.plugin_channel_mut("discord").accounts.insert(
        "discord-main".to_string(),
        discord_account_to_plugin_entry(&DiscordAccount {
            token: "test-token".to_string(),
            enabled: true,
            name: None,
            agent_id: "claude".to_string(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            api_base: "https://discord.com/api/v10".to_string(),
            gateway_url: "wss://gateway.discord.gg/?v=10&encoding=json".to_string(),
        }),
    );

    let dispatcher = ChannelDispatcherImpl::from_config(&channels);
    let available = dispatcher.available_channels();
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].channel, "discord");
    assert_eq!(available[0].account_id, "discord-main");
}

#[test]
fn test_multiple_channels_sorted() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(TelegramSender {
        account_id: "bot2".to_string(),
        token: "t2".to_string(),
        http: Client::new(),
        api_base: "https://api.telegram.org".to_string(),
        is_running: true,
    });
    dispatcher.register_feishu(FeishuSender::new(
        "bot1".to_string(),
        "a".to_string(),
        "s".to_string(),
        "https://open.feishu.cn/open-apis".to_string(),
        false,
    ));
    dispatcher.register_telegram(TelegramSender {
        account_id: "bot1".to_string(),
        token: "t1".to_string(),
        http: Client::new(),
        api_base: "https://api.telegram.org".to_string(),
        is_running: true,
    });

    let channels = dispatcher.available_channels();
    assert_eq!(channels.len(), 3);
    // Should be sorted: feishu/bot1, telegram/bot1, telegram/bot2
    assert_eq!(channels[0].channel, "feishu");
    assert_eq!(channels[1].channel, "telegram");
    assert_eq!(channels[1].account_id, "bot1");
    assert_eq!(channels[2].channel, "telegram");
    assert_eq!(channels[2].account_id, "bot2");
}

#[tokio::test]
async fn test_send_discord_text_uses_thread_and_safe_allowed_mentions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/thread-456/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "message-001"
        })))
        .mount(&server)
        .await;

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_discord(DiscordSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: server.uri(),
        is_running: true,
    });

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "discord".to_string(),
            account_id: "main".to_string(),
            chat_id: "channel-123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "channel-123".to_string(),
            content: ChannelOutboundContent::text("hello <@1000000001> @everyone"),
            reply_to: Some("reply-789".to_string()),
            thread_id: Some("thread-456".to_string()),
        })
        .await
        .expect("send discord text");

    assert_eq!(result.message_ids, vec!["message-001".to_string()]);
    let requests = server.received_requests().await.expect("received requests");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bot test-token")
    );
    let body: Value = serde_json::from_slice(&request.body).expect("discord request json");
    assert_eq!(body["content"], "hello <@1000000001> @everyone");
    assert_eq!(
        body["allowed_mentions"]["parse"],
        serde_json::json!(["users"])
    );
    assert_eq!(body["allowed_mentions"]["replied_user"], true);
    assert_eq!(body["message_reference"]["message_id"], "reply-789");
    assert_eq!(body["message_reference"]["channel_id"], "thread-456");
    assert_eq!(body["message_reference"]["fail_if_not_exists"], false);
}

#[tokio::test]
async fn test_send_discord_retries_without_reply_reference_when_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/channel-123/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "code": 10008,
            "message": "Unknown Message"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/channels/channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "message-002"
        })))
        .mount(&server)
        .await;

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_discord(DiscordSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: server.uri(),
        is_running: true,
    });

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "discord".to_string(),
            account_id: "main".to_string(),
            chat_id: "channel-123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "channel-123".to_string(),
            content: ChannelOutboundContent::text("hello"),
            reply_to: Some("missing-message".to_string()),
            thread_id: None,
        })
        .await
        .expect("retry without rejected reply reference");

    assert_eq!(result.message_ids, vec!["message-002".to_string()]);
    let requests = server.received_requests().await.expect("received requests");
    assert_eq!(requests.len(), 2);
    let first: Value = serde_json::from_slice(&requests[0].body).expect("first json");
    let second: Value = serde_json::from_slice(&requests[1].body).expect("second json");
    assert!(first.get("message_reference").is_some());
    assert!(second.get("message_reference").is_none());
}

#[tokio::test]
async fn test_send_discord_text_retries_after_rate_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/channel-123/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "0.01")
                .insert_header("x-ratelimit-scope", "user")
                .set_body_json(serde_json::json!({
                    "message": "You are being rate limited.",
                    "retry_after": 0.01,
                    "global": false
                })),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/channels/channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "message-003"
        })))
        .mount(&server)
        .await;

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_discord(DiscordSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: server.uri(),
        is_running: true,
    });

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "discord".to_string(),
            account_id: "main".to_string(),
            chat_id: "channel-123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "channel-123".to_string(),
            content: ChannelOutboundContent::text("hello"),
            reply_to: None,
            thread_id: None,
        })
        .await
        .expect("retry after rate limit");

    assert_eq!(result.message_ids, vec!["message-003".to_string()]);
    let requests = server.received_requests().await.expect("received requests");
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn test_edit_discord_text_retries_after_rate_limit() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/channels/channel-123/messages/message-001"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "0.01")
                .set_body_json(serde_json::json!({
                    "message": "You are being rate limited.",
                    "retry_after": 0.01,
                    "global": false
                })),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/channels/channel-123/messages/message-001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "message-001"
        })))
        .mount(&server)
        .await;

    let sender = DiscordSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: server.uri(),
        is_running: true,
    };

    let result = sender
        .edit_text("channel-123", "message-001", "updated")
        .await
        .expect("retry edit after rate limit");

    assert_eq!(result, "message-001");
    let requests = server.received_requests().await.expect("received requests");
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn test_send_unknown_channel() {
    let dispatcher = ChannelDispatcherImpl::new();
    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "unknown-chat".to_string(),
            account_id: "x".to_string(),
            chat_id: "123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "123".to_string(),
            content: ChannelOutboundContent::text("hello"),
            reply_to: None,
            thread_id: None,
        })
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Unknown channel type"), "got: {err}");
}

#[tokio::test]
async fn test_send_unregistered_telegram_account() {
    let dispatcher = ChannelDispatcherImpl::new();
    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "telegram".to_string(),
            account_id: "nonexistent".to_string(),
            chat_id: "123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "123".to_string(),
            content: ChannelOutboundContent::text("hello"),
            reply_to: None,
            thread_id: None,
        })
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not registered"), "got: {err}");
}

#[tokio::test]
async fn test_send_invalid_telegram_chat_id() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(TelegramSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: "https://api.telegram.org".to_string(),
        is_running: true,
    });

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "telegram".to_string(),
            account_id: "main".to_string(),
            chat_id: "not-a-number".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "not-a-number".to_string(),
            content: ChannelOutboundContent::text("hello"),
            reply_to: None,
            thread_id: None,
        })
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Invalid Telegram chat_id"), "got: {err}");
}

#[tokio::test]
async fn test_send_invalid_telegram_reply_to() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(TelegramSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: "https://api.telegram.org".to_string(),
        is_running: true,
    });

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "telegram".to_string(),
            account_id: "main".to_string(),
            chat_id: "123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "123".to_string(),
            content: ChannelOutboundContent::text("hello"),
            reply_to: Some("bad-reply-id".to_string()),
            thread_id: None,
        })
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Invalid Telegram reply_to"), "got: {err}");
}

#[tokio::test]
async fn test_send_invalid_telegram_thread_id() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(TelegramSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: "https://api.telegram.org".to_string(),
        is_running: true,
    });

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "telegram".to_string(),
            account_id: "main".to_string(),
            chat_id: "123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "123".to_string(),
            content: ChannelOutboundContent::text("hello"),
            reply_to: None,
            thread_id: Some("bad-thread-id".to_string()),
        })
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("send failed")
            || err.contains("Invalid Telegram thread_id")
            || err.contains("Not Found"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_send_telegram_image_content_uses_send_photo() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/sendPhoto"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 42,
                "chat": { "id": 123, "type": "private" },
                "date": 1
            }
        })))
        .mount(&server)
        .await;

    let temp = tempfile::tempdir().expect("temp dir");
    let image_path = temp.path().join("preview.png");
    std::fs::write(&image_path, b"png").expect("image");

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(TelegramSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: server.uri(),
        is_running: true,
    });

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "telegram".to_string(),
            account_id: "main".to_string(),
            chat_id: "123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "123".to_string(),
            content: ChannelOutboundContent::image(
                image_path.to_string_lossy().to_string(),
                Some("preview".to_string()),
            ),
            reply_to: None,
            thread_id: None,
        })
        .await
        .expect("send image");

    assert_eq!(result.message_ids, vec!["42".to_string()]);
    let requests = server.received_requests().await.expect("received requests");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/bottest-token/sendPhoto")
            .count(),
        1
    );
}

#[test]
fn test_extract_telegram_markdown_image_refs_supports_local_image_links() {
    let (text, refs) = extract_telegram_markdown_image_refs(
        "改好了。\n[局部截图](/tmp/garyx-preview.png)\n![细节](file:///tmp/garyx-detail.jpg)",
    );

    assert!(!text.contains("/tmp/garyx-preview.png"));
    assert!(!text.contains("/tmp/garyx-detail.jpg"));
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].path, "/tmp/garyx-preview.png");
    assert_eq!(refs[0].caption.as_deref(), Some("局部截图"));
    assert_eq!(refs[1].path, "/tmp/garyx-detail.jpg");
    assert_eq!(refs[1].caption.as_deref(), Some("细节"));
}

#[tokio::test]
async fn test_send_telegram_text_markdown_image_link_sends_photo() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 41,
                "chat": { "id": 123, "type": "private" },
                "date": 1
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/sendPhoto"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 42,
                "chat": { "id": 123, "type": "private" },
                "date": 1
            }
        })))
        .mount(&server)
        .await;

    let temp = tempfile::tempdir().expect("temp dir");
    let image_path = temp.path().join("preview.png");
    std::fs::write(&image_path, b"png").expect("image");

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(TelegramSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: server.uri(),
        is_running: true,
    });

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "telegram".to_string(),
            account_id: "main".to_string(),
            chat_id: "123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "123".to_string(),
            content: ChannelOutboundContent::text(format!(
                "改好了。\n[局部截图]({})",
                image_path.display()
            )),
            reply_to: None,
            thread_id: None,
        })
        .await
        .expect("send markdown image link");

    assert_eq!(result.message_ids, vec!["41".to_string(), "42".to_string()]);
    let requests = server.received_requests().await.expect("received requests");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/bottest-token/sendPhoto")
            .count(),
        1
    );
    let text_request = requests
        .iter()
        .find(|request| request.url.path() == "/bottest-token/sendMessage")
        .expect("sendMessage request");
    let body: Value = serde_json::from_slice(&text_request.body).expect("sendMessage body");
    assert!(
        !body["text"]
            .as_str()
            .unwrap_or_default()
            .contains("preview.png")
    );
}

#[tokio::test]
async fn test_send_telegram_file_content_uses_send_document() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/sendDocument"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 43,
                "chat": { "id": 123, "type": "private" },
                "date": 1
            }
        })))
        .mount(&server)
        .await;

    let temp = tempfile::tempdir().expect("temp dir");
    let file_path = temp.path().join("note.md");
    std::fs::write(&file_path, b"# Test\n").expect("file");

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(TelegramSender {
        account_id: "main".to_string(),
        token: "test-token".to_string(),
        http: Client::new(),
        api_base: server.uri(),
        is_running: true,
    });

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "telegram".to_string(),
            account_id: "main".to_string(),
            chat_id: "123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "123".to_string(),
            content: ChannelOutboundContent::file(
                file_path.to_string_lossy().to_string(),
                Some("note".to_string()),
            ),
            reply_to: None,
            thread_id: None,
        })
        .await
        .expect("send file");

    assert_eq!(result.message_ids, vec!["43".to_string()]);
    let requests = server.received_requests().await.expect("received requests");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/bottest-token/sendDocument")
            .count(),
        1
    );
}

#[test]
fn test_normalize_telegram_thread_id_ignores_private_chat_binding_key() {
    assert_eq!(normalize_telegram_thread_id(42, Some("42")), None);
    assert_eq!(normalize_telegram_thread_id(42, Some("555")), Some(555));
    assert_eq!(
        normalize_telegram_thread_id(42, Some("thread::internal")),
        None
    );
    assert_eq!(normalize_telegram_thread_id(42, None), None);
}

#[tokio::test]
async fn test_feishu_sender_refreshes_token_once_under_concurrency() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "tenant_access_token": "tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;

    let sender = FeishuSender::new(
        "bot1".to_string(),
        "app123".to_string(),
        "secret".to_string(),
        server.uri(),
        true,
    );

    let (first, second, third, fourth) = tokio::join!(
        sender.get_access_token(),
        sender.get_access_token(),
        sender.get_access_token(),
        sender.get_access_token()
    );

    for token in [first, second, third, fourth] {
        assert_eq!(token.expect("token refresh"), "tenant-token");
    }

    let requests = server.received_requests().await.expect("received requests");
    let refresh_calls = requests
        .iter()
        .filter(|request| request.url.path() == "/auth/v3/tenant_access_token/internal")
        .count();
    assert_eq!(refresh_calls, 1);
}

#[tokio::test]
async fn test_send_feishu_topic_thread_replies_to_root_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "tenant_access_token": "tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/im/v1/messages/om_root_123/reply"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": { "message_id": "om_reply_001" }
        })))
        .mount(&server)
        .await;

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_feishu(FeishuSender::new(
        "bot1".to_string(),
        "app123".to_string(),
        "secret".to_string(),
        server.uri(),
        true,
    ));

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "feishu".to_string(),
            account_id: "bot1".to_string(),
            chat_id: "oc_group_123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "oc_group_123".to_string(),
            content: ChannelOutboundContent::text("hello thread"),
            reply_to: None,
            thread_id: Some("oc_group_123:topic:om_root_123".to_string()),
        })
        .await
        .expect("send thread reply");

    assert_eq!(result.message_ids, vec!["om_reply_001".to_string()]);

    let requests = server.received_requests().await.expect("received requests");
    let reply_calls = requests
        .iter()
        .filter(|request| request.url.path() == "/im/v1/messages/om_root_123/reply")
        .count();
    assert_eq!(reply_calls, 1);
}

#[tokio::test]
async fn test_feishu_sender_fetches_app_owner_open_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "tenant_access_token": "tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/application/v6/applications/app123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "app": {
                    "owner": {
                        "owner_id": "ou_owner_123"
                    }
                }
            }
        })))
        .mount(&server)
        .await;

    let sender = FeishuSender::new(
        "bot1".to_string(),
        "app123".to_string(),
        "secret".to_string(),
        server.uri(),
        true,
    );

    let owner_open_id = sender
        .fetch_app_owner_open_id()
        .await
        .expect("fetch app owner should succeed");
    assert_eq!(owner_open_id.as_deref(), Some("ou_owner_123"));

    let requests = server.received_requests().await.expect("received requests");
    let owner_request = requests
        .iter()
        .find(|request| request.url.path() == "/application/v6/applications/app123")
        .expect("owner request should be sent");
    assert_eq!(owner_request.method.as_str(), "GET");
    assert_eq!(owner_request.url.query(), Some("lang=zh_cn"));
}

#[tokio::test]
async fn test_feishu_sender_fetches_chat_summary() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "tenant_access_token": "tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/im/v1/chats/oc_group_123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": {
                "name": "bot 测试",
                "chat_mode": "group",
                "chat_type": "private"
            }
        })))
        .mount(&server)
        .await;

    let sender = FeishuSender::new(
        "bot1".to_string(),
        "app123".to_string(),
        "secret".to_string(),
        server.uri(),
        true,
    );

    let summary = sender
        .fetch_chat_summary("oc_group_123")
        .await
        .expect("fetch chat summary should succeed");
    assert_eq!(
        summary,
        Some(FeishuChatSummary {
            name: Some("bot 测试".to_string()),
            chat_mode: Some("group".to_string()),
            chat_type: Some("private".to_string()),
        })
    );
}

#[tokio::test]
async fn test_send_feishu_open_id_target_uses_open_id_receive_type() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "tenant_access_token": "tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/im/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": { "message_id": "om_open_id_001" }
        })))
        .mount(&server)
        .await;

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_feishu(FeishuSender::new(
        "bot1".to_string(),
        "app123".to_string(),
        "secret".to_string(),
        server.uri(),
        true,
    ));

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "feishu".to_string(),
            account_id: "bot1".to_string(),
            chat_id: "ou_owner_123".to_string(),
            delivery_target_type: "open_id".to_string(),
            delivery_target_id: "ou_owner_123".to_string(),
            content: ChannelOutboundContent::text("hello owner"),
            reply_to: None,
            thread_id: None,
        })
        .await
        .expect("send open-id message");

    assert_eq!(result.message_ids, vec!["om_open_id_001".to_string()]);

    let requests = server.received_requests().await.expect("received requests");
    let message_request = requests
        .iter()
        .find(|request| request.url.path() == "/im/v1/messages")
        .expect("message request should be sent");
    assert_eq!(message_request.url.query(), Some("receive_id_type=open_id"));
    let body: Value = serde_json::from_slice(&message_request.body).expect("message request json");
    assert_eq!(body["receive_id"], "ou_owner_123");
}

#[tokio::test]
async fn test_send_feishu_image_content_uploads_and_sends_image() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "tenant_access_token": "tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/im/v1/images"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": { "image_key": "img_v2_123" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/im/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": { "message_id": "om_image_001" }
        })))
        .mount(&server)
        .await;

    let temp = tempfile::tempdir().expect("temp dir");
    let image_path = temp.path().join("preview.png");
    std::fs::write(&image_path, b"png").expect("image");

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_feishu(FeishuSender::new(
        "bot1".to_string(),
        "app123".to_string(),
        "secret".to_string(),
        server.uri(),
        true,
    ));

    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "feishu".to_string(),
            account_id: "bot1".to_string(),
            chat_id: "oc_group_123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "oc_group_123".to_string(),
            content: ChannelOutboundContent::image(image_path.to_string_lossy().to_string(), None),
            reply_to: None,
            thread_id: None,
        })
        .await
        .expect("send image");

    assert_eq!(result.message_ids, vec!["om_image_001".to_string()]);
    let requests = server.received_requests().await.expect("received requests");
    assert!(
        requests
            .iter()
            .any(|request| request.url.path() == "/im/v1/images"),
        "image upload request missing"
    );
    let message_request = requests
        .iter()
        .find(|request| request.url.path() == "/im/v1/messages")
        .expect("image message request should be sent");
    let body: Value = serde_json::from_slice(&message_request.body).expect("message request json");
    assert_eq!(body["msg_type"], "image");
    assert_eq!(body["receive_id"], "oc_group_123");
}

// -----------------------------------------------------------------
// Plugin-backed routing + SwappableDispatcher
// -----------------------------------------------------------------

mod plugin_routing {
    use super::*;
    use crate::plugin_host::protocol::CapabilitiesResponse;
    use crate::plugin_host::sender::PluginSenderHandle;
    use crate::plugin_host::transport::{
        InboundHandler, PluginRpcClient, Transport, TransportConfig,
    };
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::duplex;

    fn caps_outbound() -> CapabilitiesResponse {
        CapabilitiesResponse {
            outbound: true,
            inbound: false,
            streaming: false,
            images: false,
            files: false,
            survives_respawn: false,
        }
    }

    /// Build a PluginSenderHandle whose remote `dispatch_outbound`
    /// echoes a single message id. The plugin-side client is
    /// returned alongside so the caller can keep it in scope for
    /// the duration of the test — dropping it would cause the
    /// plugin writer to close and subsequent RPCs to fail with
    /// Disconnected.
    fn stub_plugin_sender(
        plugin_id: &str,
        fixed_message_id: &'static str,
    ) -> (PluginSenderHandle, PluginRpcClient) {
        struct HostDrop;
        #[async_trait]
        impl InboundHandler for HostDrop {
            async fn on_request(
                &self,
                _method: String,
                _params: Value,
            ) -> Result<Value, (i32, String)> {
                Err((-32601, "none".into()))
            }
            async fn on_notification(&self, _: String, _: Value) {}
        }

        struct StubPlugin {
            id: &'static str,
        }
        #[async_trait]
        impl InboundHandler for StubPlugin {
            async fn on_request(
                &self,
                method: String,
                _params: Value,
            ) -> Result<Value, (i32, String)> {
                match method.as_str() {
                    "dispatch_outbound" => Ok(json!({"message_ids": [self.id]})),
                    other => Err((-32601, format!("no {other}"))),
                }
            }
            async fn on_notification(&self, _: String, _: Value) {}
        }

        let (host_rw, plugin_rw) = duplex(64 * 1024);
        let (host_r, host_w) = tokio::io::split(host_rw);
        let (plugin_r, plugin_w) = tokio::io::split(plugin_rw);

        let (host_rpc, _host_handles) = Transport::spawn(
            host_r,
            host_w,
            TransportConfig {
                plugin_id: plugin_id.to_owned(),
                default_rpc_timeout: Duration::from_secs(10),
                ..Default::default()
            },
            Arc::new(HostDrop),
        );
        let stub_id: &'static str = Box::leak(fixed_message_id.to_owned().into_boxed_str());
        let (plugin_keep, _plugin_handles) = Transport::spawn(
            plugin_r,
            plugin_w,
            TransportConfig {
                plugin_id: format!("{plugin_id}-peer"),
                ..Default::default()
            },
            Arc::new(StubPlugin { id: stub_id }),
        );
        let handle = PluginSenderHandle::new(plugin_id.to_owned(), host_rpc, caps_outbound());
        (handle, plugin_keep)
    }

    fn outbound(channel: &str) -> OutboundMessage {
        OutboundMessage {
            channel: channel.to_owned(),
            account_id: "acct".to_owned(),
            chat_id: "chat-1".to_owned(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
            delivery_target_id: "chat-1".to_owned(),
            content: ChannelOutboundContent::text("hi"),
            reply_to: None,
            thread_id: None,
        }
    }

    #[tokio::test]
    async fn register_plugin_routes_by_plugin_id() {
        let (handle, _keep) = stub_plugin_sender("mino", "plug-msg-1");
        let mut dispatcher = ChannelDispatcherImpl::new();
        dispatcher.register_plugin(handle).expect("register");
        let result = dispatcher
            .send_message(outbound("mino"))
            .await
            .expect("plugin send");
        assert_eq!(result.message_ids, vec!["plug-msg-1".to_string()]);
    }

    #[tokio::test]
    async fn register_plugin_rejects_reserved_builtin_names() {
        // A plugin id that collides with a built-in match arm
        // would be shadowed forever, so registration must reject.
        // Iterate the single source of truth so drift fails here.
        for reserved in super::super::RESERVED_CHANNEL_NAMES {
            let (handle, _keep) = stub_plugin_sender(reserved, "x");
            let mut dispatcher = ChannelDispatcherImpl::new();
            let err = dispatcher
                .register_plugin(handle)
                .expect_err("reserved must reject");
            match err {
                ChannelError::Config(msg) => {
                    assert!(
                        msg.contains(reserved) && msg.contains("reserved"),
                        "expected collision message for {reserved}, got {msg}"
                    );
                }
                other => panic!("expected Config for {reserved}, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn unknown_channel_still_falls_through_to_config_error() {
        // Plugin channel 'mino' exists, but the caller asked for
        // 'nope'. Must surface the existing Config error — the
        // plugin fallback MUST NOT swallow unknown names into
        // SendFailed.
        let (handle, _keep) = stub_plugin_sender("mino", "irrelevant");
        let mut dispatcher = ChannelDispatcherImpl::new();
        dispatcher.register_plugin(handle).expect("register");
        let err = dispatcher
            .send_message(outbound("nope"))
            .await
            .expect_err("unknown");
        match err {
            ChannelError::Config(msg) => assert!(msg.contains("Unknown channel type")),
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn available_channels_includes_plugin_id() {
        let (handle, _keep) = stub_plugin_sender("mino", "x");
        let mut dispatcher = ChannelDispatcherImpl::new();
        dispatcher.register_plugin(handle).expect("register");
        let channels = dispatcher.available_channels();
        assert!(
            channels.iter().any(|c| c.channel == "mino"),
            "plugin id should appear in available_channels: {channels:?}"
        );
    }

    #[tokio::test]
    async fn unregister_plugin_drops_it_from_routing() {
        let (handle, _keep) = stub_plugin_sender("mino", "x");
        let mut dispatcher = ChannelDispatcherImpl::new();
        dispatcher.register_plugin(handle).expect("register");
        assert!(dispatcher.unregister_plugin("mino").is_some());
        let err = dispatcher
            .send_message(outbound("mino"))
            .await
            .expect_err("should be gone");
        assert!(matches!(err, ChannelError::Config(_)));
    }

    #[tokio::test]
    async fn register_plugin_overwrites_existing_handle_for_respawn() {
        // §9.4 respawn contract: re-registering the same plugin id
        // replaces the handle. This is what `respawn_plugin` will
        // lean on.
        let (old, _old_keep) = stub_plugin_sender("mino", "OLD");
        let (new, _new_keep) = stub_plugin_sender("mino", "NEW");
        let mut dispatcher = ChannelDispatcherImpl::new();
        dispatcher.register_plugin(old).expect("register old");
        dispatcher.register_plugin(new).expect("register new");
        let result = dispatcher
            .send_message(outbound("mino"))
            .await
            .expect("plugin send");
        assert_eq!(result.message_ids, vec!["NEW".to_string()]);
    }

    #[tokio::test]
    async fn swappable_dispatcher_routes_through_current_snapshot() {
        let (first, _first_keep) = stub_plugin_sender("mino", "V1");
        let (second, _second_keep) = stub_plugin_sender("mino", "V2");

        let mut initial = ChannelDispatcherImpl::new();
        initial.register_plugin(first).expect("register first");
        let swap = Arc::new(SwappableDispatcher::new(initial));

        let r1 = swap.send_message(outbound("mino")).await.expect("V1 send");
        assert_eq!(r1.message_ids, vec!["V1".to_string()]);

        // Publish a new dispatcher; subsequent sends should see V2.
        let mut next = ChannelDispatcherImpl::new();
        next.register_plugin(second).expect("register second");
        swap.store(Arc::new(next));

        let r2 = swap.send_message(outbound("mino")).await.expect("V2 send");
        assert_eq!(r2.message_ids, vec!["V2".to_string()]);
    }

    #[tokio::test]
    async fn swappable_dispatcher_in_flight_send_completes_against_old_snapshot() {
        // Gate the OLD plugin so it cannot respond until we fire
        // the release signal. Then start the send, swap in the NEW
        // dispatcher mid-flight, and only then release the gate.
        // The in-flight send must complete against the OLD handle
        // it captured at dispatch time — §9.4's drain window.
        use tokio::sync::Mutex as TokioMutex;
        use tokio::sync::oneshot;

        struct GatedPlugin {
            // Fires the instant the plugin-side handler enters
            // `dispatch_outbound` — the test uses this as a
            // handshake so the swap demonstrably happens *after*
            // the request is truly in flight inside the OLD
            // plugin, not merely after an arbitrary sleep.
            entered_tx: Arc<TokioMutex<Option<oneshot::Sender<()>>>>,
            gate_rx: Arc<TokioMutex<Option<oneshot::Receiver<()>>>>,
            id: &'static str,
        }
        #[async_trait]
        impl InboundHandler for GatedPlugin {
            async fn on_request(
                &self,
                method: String,
                _params: Value,
            ) -> Result<Value, (i32, String)> {
                match method.as_str() {
                    "dispatch_outbound" => {
                        if let Some(tx) = self.entered_tx.lock().await.take() {
                            let _ = tx.send(());
                        }
                        if let Some(rx) = self.gate_rx.lock().await.take() {
                            let _ = rx.await;
                        }
                        Ok(json!({"message_ids": [self.id]}))
                    }
                    other => Err((-32601, format!("no {other}"))),
                }
            }
            async fn on_notification(&self, _: String, _: Value) {}
        }

        struct HostDrop;
        #[async_trait]
        impl InboundHandler for HostDrop {
            async fn on_request(
                &self,
                _method: String,
                _params: Value,
            ) -> Result<Value, (i32, String)> {
                Err((-32601, "none".into()))
            }
            async fn on_notification(&self, _: String, _: Value) {}
        }

        // Wire the OLD gated plugin.
        let (gate_tx, gate_rx) = oneshot::channel::<()>();
        let (entered_tx, entered_rx) = oneshot::channel::<()>();
        let (host_rw, plugin_rw) = duplex(64 * 1024);
        let (host_r, host_w) = tokio::io::split(host_rw);
        let (plugin_r, plugin_w) = tokio::io::split(plugin_rw);
        let (host_rpc_old, _h) = Transport::spawn(
            host_r,
            host_w,
            TransportConfig {
                plugin_id: "mino-old".into(),
                default_rpc_timeout: Duration::from_secs(30),
                ..Default::default()
            },
            Arc::new(HostDrop),
        );
        let (plugin_keep_old, _p) = Transport::spawn(
            plugin_r,
            plugin_w,
            TransportConfig {
                plugin_id: "mino-old-peer".into(),
                ..Default::default()
            },
            Arc::new(GatedPlugin {
                entered_tx: Arc::new(TokioMutex::new(Some(entered_tx))),
                gate_rx: Arc::new(TokioMutex::new(Some(gate_rx))),
                id: "OLD",
            }),
        );
        let old_handle = PluginSenderHandle::new("mino".into(), host_rpc_old, caps_outbound());

        // NEW plugin: an ungated stub that responds with "NEW".
        let (new_handle, _new_keep) = stub_plugin_sender("mino", "NEW");

        let mut initial = ChannelDispatcherImpl::new();
        initial.register_plugin(old_handle).expect("register old");
        let swap = Arc::new(SwappableDispatcher::new(initial));

        // Start a send that will stall inside the OLD plugin's
        // handler until we fire `gate_tx`.
        let send_fut = {
            let swap = swap.clone();
            tokio::spawn(async move { swap.send_message(outbound("mino")).await })
        };
        // Handshake: wait until the OLD plugin is demonstrably
        // inside `dispatch_outbound`. Without this the swap could
        // race the RPC write and the test would only prove "a
        // completed send can return OLD", not "an in-flight send
        // survives a swap".
        entered_rx.await.expect("OLD plugin entered dispatch");

        // Swap in the NEW dispatcher while the OLD send is still
        // stalled. If snapshot capture is correct, the in-flight
        // send will still resolve against OLD.
        let mut next = ChannelDispatcherImpl::new();
        next.register_plugin(new_handle).expect("register new");
        swap.store(Arc::new(next));

        // A fresh send started AFTER the swap must see NEW.
        let fresh = swap
            .send_message(outbound("mino"))
            .await
            .expect("fresh send");
        assert_eq!(fresh.message_ids, vec!["NEW".to_string()]);

        // Release the OLD plugin's gate; the in-flight send
        // completes against OLD's handler.
        gate_tx.send(()).expect("release gate");
        let old_result = send_fut.await.expect("join").expect("OLD send completes");
        assert_eq!(
            old_result.message_ids,
            vec!["OLD".to_string()],
            "in-flight send MUST land on the pre-swap snapshot"
        );
        drop(plugin_keep_old);
    }

    #[tokio::test]
    async fn swappable_dispatcher_delegates_available_channels() {
        let (handle, _keep) = stub_plugin_sender("mino", "x");
        let mut impl_ = ChannelDispatcherImpl::new();
        impl_.register_plugin(handle).expect("register");
        let swap = SwappableDispatcher::new(impl_);
        let channels = swap.available_channels();
        assert!(channels.iter().any(|c| c.channel == "mino"));
    }
}
