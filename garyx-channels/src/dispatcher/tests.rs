use super::*;
use garyx_models::routing::DELIVERY_TARGET_TYPE_CHAT_ID;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
async fn test_send_unknown_channel() {
    let dispatcher = ChannelDispatcherImpl::new();
    let result = dispatcher
        .send_message(OutboundMessage {
            channel: "discord".to_string(),
            account_id: "x".to_string(),
            chat_id: "123".to_string(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_string(),
            delivery_target_id: "123".to_string(),
            text: "hello".to_string(),
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
            text: "hello".to_string(),
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
            text: "hello".to_string(),
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
            text: "hello".to_string(),
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
            text: "hello".to_string(),
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
            text: "hello thread".to_string(),
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
            text: "hello owner".to_string(),
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
            text: "hi".to_owned(),
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
