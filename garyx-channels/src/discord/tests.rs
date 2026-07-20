use super::*;
use async_trait::async_trait;
use garyx_models::AgentBindingError;
use garyx_models::config::DiscordAccount;
use garyx_models::provider::StreamEvent;
use garyx_router::{ThreadCreationError, ThreadCreator, ThreadEnsureOptions, ThreadStore};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct NoEnabledThreadCreator;

#[async_trait]
impl ThreadCreator for NoEnabledThreadCreator {
    async fn create_thread(
        &self,
        _thread_store: Arc<dyn ThreadStore>,
        _options: ThreadEnsureOptions,
    ) -> Result<(String, Value), ThreadCreationError> {
        Err(AgentBindingError::NoEnabledAgent.into())
    }
}

fn account(require_mention: bool) -> DiscordAccount {
    DiscordAccount {
        token: "discord-token".to_owned(),
        enabled: true,
        name: None,
        agent_id: Some("claude".to_owned()),
        workspace_dir: None,
        owner_target: None,
        require_mention,
        api_base: "https://discord.com/api/v10".to_owned(),
        gateway_url: "wss://gateway.discord.gg/?v=10&encoding=json".to_owned(),
    }
}

#[tokio::test]
async fn inbound_no_enabled_agent_sends_visible_error_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-error-reply"
        })))
        .mount(&server)
        .await;

    let router = crate::test_helpers::make_router();
    router
        .lock()
        .await
        .set_thread_creator(Arc::new(NoEnabledThreadCreator));
    let bridge = crate::test_helpers::make_bridge_with(Arc::new(
        crate::test_helpers::ConfigurableTestProvider::echo(),
    ))
    .await;
    let mut configured_account = account(false);
    configured_account.api_base = server.uri();
    let runtime = DiscordInboundRuntime {
        http: Client::new(),
        account_id: "main".to_owned(),
        account: configured_account,
        router,
        bridge,
        dispatcher: Arc::new(crate::dispatcher::ChannelDispatcherImpl::new()),
    };
    let bot = DiscordCurrentUser {
        id: "bot-999".to_owned(),
        username: Some("Garyx".to_owned()),
    };

    DiscordChannel::handle_message_create(
        &runtime,
        &bot,
        DiscordMessageCreateEvent {
            id: "message-001".to_owned(),
            channel_id: "dm-channel-123".to_owned(),
            guild_id: None,
            content: "hello".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: Vec::new(),
            message_reference: None,
            attachments: Vec::new(),
        },
    )
    .await;

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["content"],
        "Error: no enabled standalone agent is available"
    );
    assert_eq!(body["message_reference"]["message_id"], "message-001");
}

#[test]
fn dm_message_does_not_require_mention() {
    let event = DiscordMessageCreateEvent {
        id: "message-001".to_owned(),
        channel_id: "dm-channel-123".to_owned(),
        guild_id: None,
        content: "hello from dm".to_owned(),
        author: DiscordUser {
            id: "user-123".to_owned(),
            username: Some("Test User".to_owned()),
            bot: false,
        },
        mentions: Vec::new(),
        message_reference: None,
        attachments: Vec::new(),
    };

    let request = build_inbound_request("main", &account(true), "bot-999", event)
        .expect("dm should route without mention");

    assert_eq!(request.channel, "discord");
    assert_eq!(request.account_id, "main");
    assert_eq!(request.from_id, "user-123");
    assert!(!request.is_group);
    assert_eq!(request.thread_binding_key, "user-123");
    assert_eq!(request.message, "hello from dm");
    assert_eq!(request.extra_metadata["chat_id"], "dm-channel-123");
    assert_eq!(request.extra_metadata["display_label"], "Test User");
}

#[test]
fn guild_message_requires_mention_by_default() {
    let event = DiscordMessageCreateEvent {
        id: "message-002".to_owned(),
        channel_id: "guild-channel-123".to_owned(),
        guild_id: Some("guild-456".to_owned()),
        content: "not for the bot".to_owned(),
        author: DiscordUser {
            id: "user-123".to_owned(),
            username: Some("Test User".to_owned()),
            bot: false,
        },
        mentions: Vec::new(),
        message_reference: None,
        attachments: Vec::new(),
    };

    assert!(build_inbound_request("main", &account(true), "bot-999", event).is_none());
}

#[test]
fn guild_mention_is_stripped_without_adding_reference_text() {
    let event = DiscordMessageCreateEvent {
        id: "message-003".to_owned(),
        channel_id: "guild-channel-123".to_owned(),
        guild_id: Some("guild-456".to_owned()),
        content: "<@bot-999> please help".to_owned(),
        author: DiscordUser {
            id: "user-123".to_owned(),
            username: Some("Test User".to_owned()),
            bot: false,
        },
        mentions: vec![DiscordUser {
            id: "bot-999".to_owned(),
            username: Some("Garyx".to_owned()),
            bot: true,
        }],
        message_reference: Some(DiscordMessageReference {
            message_id: Some("reply-001".to_owned()),
        }),
        attachments: Vec::new(),
    };

    let request = build_inbound_request("main", &account(true), "bot-999", event)
        .expect("mentioned guild message should route");

    assert!(request.is_group);
    assert_eq!(request.thread_binding_key, "guild-channel-123");
    assert_eq!(request.message, "please help");
    assert_eq!(request.extra_metadata["guild_id"], "guild-456");
    assert_eq!(
        request.extra_metadata["delivery_thread_id"],
        "guild-channel-123"
    );
}

#[tokio::test]
async fn referenced_guild_message_uses_current_binding() {
    let event = DiscordMessageCreateEvent {
        id: "message-current-binding".to_owned(),
        channel_id: "guild-channel-123".to_owned(),
        guild_id: Some("guild-456".to_owned()),
        content: "<@bot-999> follow up".to_owned(),
        author: DiscordUser {
            id: "user-123".to_owned(),
            username: Some("Test User".to_owned()),
            bot: false,
        },
        mentions: vec![DiscordUser {
            id: "bot-999".to_owned(),
            username: Some("Garyx".to_owned()),
            bot: true,
        }],
        message_reference: Some(DiscordMessageReference {
            message_id: Some("message-from-another-thread".to_owned()),
        }),
        attachments: Vec::new(),
    };
    let request = build_inbound_request("main", &account(true), "bot-999", event)
        .expect("referenced guild message should route");
    assert_eq!(request.message, "follow up");

    let router = crate::test_helpers::make_router();
    {
        let mut router_guard = router.lock().await;
        router_guard
            .ensure_thread_entry(
                "thread::discord-current",
                "discord",
                "main",
                "guild-channel-123",
                Some("Current"),
            )
            .await;
        let binding_key =
            MessageRouter::build_binding_context_key("discord", "main", "guild-channel-123");
        router_guard.switch_to_thread(&binding_key, "thread::discord-current");
    }
    let bridge = crate::test_helpers::make_bridge_with(Arc::new(
        crate::test_helpers::ConfigurableTestProvider::echo(),
    ))
    .await;
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(|_| {});
    let result = router
        .lock()
        .await
        .route_and_dispatch(request, bridge.as_ref(), Some(callback))
        .await
        .expect("referenced message should dispatch");

    assert_eq!(result.thread_id, "thread::discord-current");
}

#[test]
fn mention_only_message_without_attachments_is_ignored() {
    let event = DiscordMessageCreateEvent {
        id: "message-004".to_owned(),
        channel_id: "guild-channel-123".to_owned(),
        guild_id: Some("guild-456".to_owned()),
        content: "<@bot-999>".to_owned(),
        author: DiscordUser {
            id: "user-123".to_owned(),
            username: Some("Test User".to_owned()),
            bot: false,
        },
        mentions: vec![DiscordUser {
            id: "bot-999".to_owned(),
            username: Some("Garyx".to_owned()),
            bot: true,
        }],
        message_reference: None,
        attachments: Vec::new(),
    };

    assert!(build_inbound_request("main", &account(true), "bot-999", event).is_none());
}

#[test]
fn empty_text_message_with_attachment_routes_without_fallback_text() {
    let event = DiscordMessageCreateEvent {
        id: "message-005".to_owned(),
        channel_id: "dm-channel-123".to_owned(),
        guild_id: None,
        content: String::new(),
        author: DiscordUser {
            id: "user-123".to_owned(),
            username: Some("Test User".to_owned()),
            bot: false,
        },
        mentions: Vec::new(),
        message_reference: None,
        attachments: vec![DiscordAttachment {
            id: "attachment-file".to_owned(),
            filename: "report.txt".to_owned(),
            content_type: Some("text/plain".to_owned()),
            size: Some(12),
            url: "https://example.invalid/files/report.txt".to_owned(),
        }],
    };

    let request = build_inbound_request("main", &account(true), "bot-999", event)
        .expect("attachment-only dm should route");

    assert_eq!(request.message, "");
    assert_eq!(request.extra_metadata[NATIVE_COMMAND_TEXT_METADATA_KEY], "");
}

#[test]
fn bot_authored_messages_are_ignored() {
    let event = DiscordMessageCreateEvent {
        id: "message-006".to_owned(),
        channel_id: "dm-channel-123".to_owned(),
        guild_id: None,
        content: "ignore me".to_owned(),
        author: DiscordUser {
            id: "bot-999".to_owned(),
            username: Some("Garyx".to_owned()),
            bot: true,
        },
        mentions: Vec::new(),
        message_reference: None,
        attachments: Vec::new(),
    };

    assert!(build_inbound_request("main", &account(true), "bot-999", event).is_none());
}

#[test]
fn discord_gateway_resume_payload_preserves_session_cursor() {
    let payload = discord_resume_payload("discord-token", "session-123", 42);

    assert_eq!(payload["op"], 6);
    assert_eq!(payload["d"]["token"], "discord-token");
    assert_eq!(payload["d"]["session_id"], "session-123");
    assert_eq!(payload["d"]["seq"], 42);
}

#[test]
fn discord_gateway_url_query_keeps_root_path() {
    assert_eq!(
        discord_gateway_url_with_query("wss://gateway.discord.gg"),
        "wss://gateway.discord.gg/?v=10&encoding=json"
    );
    assert_eq!(
        discord_gateway_url_with_query("wss://gateway.discord.gg/"),
        "wss://gateway.discord.gg/?v=10&encoding=json"
    );
    assert_eq!(
        discord_gateway_url_with_query("wss://gateway.discord.gg/?v=10&encoding=json"),
        "wss://gateway.discord.gg/?v=10&encoding=json"
    );
}

#[tokio::test]
async fn inbound_downloads_discord_images_and_files() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files/plot.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(b"fake png".to_vec()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/files/report.txt"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_bytes(b"report bytes".to_vec()),
        )
        .mount(&server)
        .await;

    let event = DiscordMessageCreateEvent {
        id: "message-005".to_owned(),
        channel_id: "dm-channel-123".to_owned(),
        guild_id: None,
        content: "see attached".to_owned(),
        author: DiscordUser {
            id: "user-123".to_owned(),
            username: Some("Test User".to_owned()),
            bot: false,
        },
        mentions: Vec::new(),
        message_reference: None,
        attachments: vec![
            DiscordAttachment {
                id: "attachment-image".to_owned(),
                filename: "plot.png".to_owned(),
                content_type: Some("image/png".to_owned()),
                size: Some(8),
                url: format!("{}/files/plot.png", server.uri()),
            },
            DiscordAttachment {
                id: "attachment-file".to_owned(),
                filename: "report.txt".to_owned(),
                content_type: Some("text/plain".to_owned()),
                size: Some(12),
                url: format!("{}/files/report.txt", server.uri()),
            },
        ],
    };
    let mut request = build_inbound_request("main", &account(true), "bot-999", event.clone())
        .expect("discord message should route");
    let runtime = DiscordInboundRuntime {
        http: Client::new(),
        account_id: "main".to_owned(),
        account: account(true),
        router: crate::test_helpers::make_router(),
        bridge: crate::test_helpers::make_bridge_with(Arc::new(
            crate::test_helpers::ConfigurableTestProvider::echo(),
        ))
        .await,
        dispatcher: Arc::new(crate::dispatcher::ChannelDispatcherImpl::new()),
    };

    enrich_inbound_request_with_discord_attachments(&runtime, &event, &mut request).await;

    let prompt_attachments =
        garyx_models::provider::attachments_from_metadata(&request.extra_metadata);
    assert_eq!(prompt_attachments.len(), 1);
    assert_eq!(prompt_attachments[0].kind, PromptAttachmentKind::Image);
    assert_eq!(prompt_attachments[0].name, "plot.png");
    assert_eq!(prompt_attachments[0].media_type, "image/png");
    assert!(Path::new(&prompt_attachments[0].path).is_file());
    assert_eq!(request.file_paths.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&request.file_paths[0]).expect("downloaded file"),
        "report bytes"
    );
    assert_eq!(request.extra_metadata["image_count"], 1);
    assert_eq!(request.extra_metadata["file_count"], 1);
    assert_eq!(request.extra_metadata["attachment_count"], 2);

    let _ = std::fs::remove_file(&prompt_attachments[0].path);
    let _ = std::fs::remove_file(&request.file_paths[0]);
}

#[tokio::test]
async fn response_callback_sends_final_text_with_wire_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-reply-001"
        })))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::Delta {
        text: "在".to_owned(),
    });
    callback(StreamEvent::Delta {
        text: "。".to_owned(),
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        server
            .received_requests()
            .await
            .expect("received requests")
            .is_empty(),
        "Discord text deltas should buffer until a tool call or final Done"
    );

    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if !requests.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 1);
    let create_body: Value =
        serde_json::from_slice(&requests[0].body).expect("discord create body");
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(create_body["content"], "在。");
    assert_eq!(
        create_body["message_reference"]["message_id"],
        "message-001"
    );
}

#[tokio::test]
async fn response_callback_flushes_buffered_text_when_tool_starts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-buffered-tool-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::Delta {
        text: "before ".to_owned(),
    });
    callback(StreamEvent::Delta {
        text: "tool".to_owned(),
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        server
            .received_requests()
            .await
            .expect("received requests")
            .is_empty(),
        "Discord should not send buffered text before a tool call"
    );

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Bash"}),
            Some("tool-bash-1".to_owned()),
            None,
        ),
    });
    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 2);
    let create_body: Value =
        serde_json::from_slice(&requests[0].body).expect("discord create body");
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(create_body["content"], "before tool\n\n🔧 #1 Bash");
    let edit_body: Value = serde_json::from_slice(&requests[1].body).expect("discord edit body");
    assert_eq!(requests[1].method.as_str(), "PATCH");
    assert_eq!(edit_body["content"], "before tool");
}

#[tokio::test]
async fn response_callback_replaces_tool_placeholder_with_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-tool-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Read"}),
            Some("tool-read-1".to_owned()),
            None,
        ),
    });
    callback(StreamEvent::Delta {
        text: "done".to_owned(),
    });
    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 2);
    let create_body: Value =
        serde_json::from_slice(&requests[0].body).expect("discord create body");
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(create_body["content"], "🔧 #1 Read");
    let edit_body: Value = serde_json::from_slice(&requests[1].body).expect("discord edit body");
    assert_eq!(requests[1].method.as_str(), "PATCH");
    assert_eq!(edit_body["content"], "done");
}

#[tokio::test]
async fn response_callback_falls_back_to_new_message_when_final_edit_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-placeholder-001"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(
            "/channels/dm-channel-123/messages/discord-placeholder-001",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "code": 10008,
            "message": "Unknown Message"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-final-001"
        })))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-edit-fallback-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Bash"}),
            Some("tool-bash-1".to_owned()),
            None,
        ),
    });
    callback(StreamEvent::Delta {
        text: "final text".to_owned(),
    });
    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..30 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(requests[1].method.as_str(), "PATCH");
    assert_eq!(requests[2].method.as_str(), "POST");
    let fallback_body: Value =
        serde_json::from_slice(&requests[2].body).expect("discord fallback body");
    assert_eq!(fallback_body["content"], "final text");
}

#[tokio::test]
async fn response_callback_done_resets_state_before_later_user_ack() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-final-001"
        })))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-done-reset-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::Delta {
        text: "old final".to_owned(),
    });
    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(requests.len(), 1);

    callback(StreamEvent::Boundary {
        kind: StreamBoundaryKind::UserAck,
        pending_input_id: None,
    });
    tokio::time::sleep(Duration::from_millis(150)).await;

    requests = server.received_requests().await.expect("received requests");
    assert_eq!(
        requests.len(),
        1,
        "a user ack after Done must not resend stale accumulated text"
    );
}

#[tokio::test]
async fn discord_sender_retries_transient_create_message_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "message": "temporary upstream failure"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-retry-001"
        })))
        .mount(&server)
        .await;

    let sender = DiscordSender {
        account_id: "main".to_owned(),
        token: "discord-token".to_owned(),
        http: Client::new(),
        api_base: server.uri(),
        is_running: true,
    };

    let message_ids = sender
        .send_text("dm-channel-123", "retry me", Some("message-001"))
        .await
        .expect("transient Discord create failure should be retried");

    assert_eq!(message_ids, vec!["discord-retry-001".to_owned()]);
    let requests = server.received_requests().await.expect("received requests");
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn response_callback_deletes_runtime_only_tool_placeholder_on_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-tool-only-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Bash"}),
            Some("tool-bash-1".to_owned()),
            None,
        ),
    });
    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(requests[1].method.as_str(), "DELETE");
}

#[tokio::test]
async fn response_callback_user_ack_boundary_splits_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-boundary-001"
        })))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-boundary-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::Delta {
        text: "第一段".to_owned(),
    });
    callback(StreamEvent::Boundary {
        kind: StreamBoundaryKind::UserAck,
        pending_input_id: None,
    });
    callback(StreamEvent::Delta {
        text: "第二段".to_owned(),
    });
    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 2);
    let first_body: Value = serde_json::from_slice(&requests[0].body).expect("discord first body");
    let second_body: Value =
        serde_json::from_slice(&requests[1].body).expect("discord second body");
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(requests[1].method.as_str(), "POST");
    assert_eq!(first_body["content"], "第一段");
    assert_eq!(second_body["content"], "第二段");
}

#[tokio::test]
async fn response_callback_user_ack_deletes_runtime_only_tool_placeholder() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-tool-boundary-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Bash"}),
            Some("tool-bash-1".to_owned()),
            None,
        ),
    });
    callback(StreamEvent::Boundary {
        kind: StreamBoundaryKind::UserAck,
        pending_input_id: None,
    });
    callback(StreamEvent::Delta {
        text: "after".to_owned(),
    });
    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 3);
    let tool_body: Value = serde_json::from_slice(&requests[0].body).expect("discord tool body");
    let after_body: Value = serde_json::from_slice(&requests[2].body).expect("discord after body");
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(tool_body["content"], "🔧 #1 Bash");
    assert_eq!(requests[1].method.as_str(), "DELETE");
    assert_eq!(requests[2].method.as_str(), "POST");
    assert_eq!(after_body["content"], "after");
}

#[tokio::test]
async fn response_callback_user_ack_cancels_scheduled_tool_placeholder_update() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-tool-boundary-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Bash"}),
            Some("tool-bash-1".to_owned()),
            None,
        ),
    });
    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if !requests.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(requests.len(), 1);

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Read"}),
            Some("tool-read-1".to_owned()),
            None,
        ),
    });
    callback(StreamEvent::Boundary {
        kind: StreamBoundaryKind::UserAck,
        pending_input_id: None,
    });

    tokio::time::sleep(DISCORD_TOOL_PLACEHOLDER_UPDATE_INTERVAL + Duration::from_millis(150)).await;
    requests = server.received_requests().await.expect("received requests");

    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(requests[1].method.as_str(), "DELETE");
}

#[tokio::test]
async fn response_callback_coalesces_rapid_tool_placeholder_updates() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/channels/dm-channel-123/messages/discord-tool-001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-tool-001"
        })))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-tool-coalesce-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Bash"}),
            Some("tool-bash-1".to_owned()),
            None,
        ),
    });
    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(requests.len(), 1);

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Read"}),
            Some("tool-read-1".to_owned()),
            None,
        ),
    });
    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Write"}),
            Some("tool-write-1".to_owned()),
            None,
        ),
    });

    tokio::time::sleep(Duration::from_millis(200)).await;
    requests = server.received_requests().await.expect("received requests");
    assert_eq!(
        requests.len(),
        1,
        "rapid Discord tool placeholders should wait for the coalesced update"
    );

    for _ in 0..30 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 2);
    let create_body: Value =
        serde_json::from_slice(&requests[0].body).expect("discord create body");
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(create_body["content"], "🔧 #1 Bash");
    let edit_body: Value = serde_json::from_slice(&requests[1].body).expect("discord edit body");
    assert_eq!(requests[1].method.as_str(), "PATCH");
    assert_eq!(edit_body["content"], "🔧 #3 Write");
}

#[tokio::test]
async fn response_callback_suppresses_child_agent_tool_placeholder() {
    let server = MockServer::start().await;
    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-child-tool-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"name": "Bash"}),
            Some("tool-child-1".to_owned()),
            None,
        )
        .with_metadata_value("parent_tool_use_id", json!("tool-parent")),
    });
    callback(StreamEvent::Done);
    tokio::time::sleep(Duration::from_millis(150)).await;

    let requests = server.received_requests().await.expect("received requests");
    assert!(requests.is_empty());
}

#[tokio::test]
async fn response_callback_sends_local_markdown_images_as_attachments() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-text-001"
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let image_path = tmp.path().join("plot.png");
    std::fs::write(&image_path, b"fake png").unwrap();

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-image-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::Delta {
        text: format!("结果如下\n![plot]({})", image_path.display()),
    });
    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 2);
    let text_body: Value = serde_json::from_slice(&requests[0].body).expect("discord text body");
    assert_eq!(text_body["content"], "结果如下");
    assert_eq!(requests[1].method.as_str(), "POST");
    assert!(
        String::from_utf8_lossy(&requests[1].body).contains("plot.png"),
        "multipart body should include the local image filename"
    );
}

#[tokio::test]
async fn response_callback_sends_remote_markdown_images_as_attachments() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/images/plot.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(b"fake png".to_vec()),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/channels/dm-channel-123/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "discord-text-001"
        })))
        .mount(&server)
        .await;

    let (callback, thread_id_tx) =
        build_discord_response_callback(DiscordStreamingCallbackConfig {
            sender: DiscordSender {
                account_id: "main".to_owned(),
                token: "discord-token".to_owned(),
                http: Client::new(),
                api_base: server.uri(),
                is_running: true,
            },
            chat_id: "dm-channel-123".to_owned(),
            reply_to_message_id: Some("message-001".to_owned()),
        });
    thread_id_tx
        .send("thread::discord-remote-image-test".to_owned())
        .expect("thread id receiver should still be alive");

    callback(StreamEvent::Delta {
        text: format!("结果如下\n![plot]({}/images/plot.png)", server.uri()),
    });
    callback(StreamEvent::Done);

    let mut requests = Vec::new();
    for _ in 0..20 {
        requests = server.received_requests().await.expect("received requests");
        if requests.len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(requests.len(), 3);
    assert!(requests.iter().any(
        |request| request.method.as_str() == "GET" && request.url.path() == "/images/plot.png"
    ));
    let post_bodies = requests
        .iter()
        .filter(|request| request.method.as_str() == "POST")
        .map(|request| String::from_utf8_lossy(&request.body).to_string())
        .collect::<Vec<_>>();
    assert_eq!(post_bodies.len(), 2);
    assert!(
        post_bodies.iter().any(|body| body.contains("结果如下")),
        "text message should strip the markdown image"
    );
    assert!(
        post_bodies.iter().any(|body| body.contains("plot.png")),
        "multipart body should include the downloaded remote image filename"
    );
}
