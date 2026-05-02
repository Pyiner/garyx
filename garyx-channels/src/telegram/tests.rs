use super::*;
use garyx_models::command_catalog::{CommandCatalogOptions, CommandSurface};
use garyx_models::config::SlashCommand;
use garyx_router::is_native_command_text;
use garyx_router::is_thread_key;

async fn dispatch_update(
    http: &reqwest::Client,
    account_id: &str,
    token: &str,
    bot_username: &str,
    bot_id: i64,
    account: &TelegramAccount,
    update: &TgUpdate,
    router: &Arc<Mutex<MessageRouter>>,
    bridge: &Arc<MultiProviderBridge>,
    api_base: &str,
) {
    let context = handlers::TelegramUpdateContext::new(
        handlers::TelegramChannelResources {
            http,
            router,
            bridge,
            api_base,
        },
        handlers::TelegramBotRuntime {
            account_id,
            token,
            bot_username,
            bot_id,
            account,
        },
    );
    TelegramChannel::handle_update(&context, update).await;
}

#[test]
fn test_split_message_short() {
    let chunks = split_message("hello world", 4096);
    assert_eq!(chunks, vec!["hello world"]);
}

#[test]
fn command_menu_sync_interval_is_ten_minutes() {
    assert_eq!(
        TELEGRAM_COMMAND_MENU_SYNC_INTERVAL,
        std::time::Duration::from_secs(10 * 60)
    );
}

#[test]
fn telegram_bot_commands_are_projected_from_command_list() {
    let mut config = garyx_models::config::GaryxConfig::default();
    config.commands.push(SlashCommand {
        name: "summary".to_owned(),
        description: "Summarize the active thread".to_owned(),
        prompt: Some("Please summarize the active thread.".to_owned()),
        skill_id: None,
    });
    let catalog = garyx_router::command_catalog_for_config(
        &config,
        CommandCatalogOptions {
            surface: Some(CommandSurface::Telegram),
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            include_hidden: false,
        },
    );

    let commands = TelegramChannel::telegram_bot_commands_from_catalog(&catalog);
    assert!(catalog.commands.iter().any(|entry| {
        entry.name == "newthread"
            && entry.kind == garyx_models::command_catalog::CommandKind::ChannelNative
    }));
    assert!(catalog.commands.iter().any(|entry| {
        entry.name == "summary"
            && entry.kind == garyx_models::command_catalog::CommandKind::Shortcut
    }));
    let names = commands
        .iter()
        .map(|command| command.command.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        &names[..5],
        &["newthread", "threads", "threadprev", "threadnext", "loop",]
    );
    assert!(names.contains(&"summary"));
}

#[test]
fn test_split_message_exact_boundary() {
    let text = "a".repeat(4096);
    let chunks = split_message(&text, 4096);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].len(), 4096);
}

#[test]
fn test_split_message_long() {
    let text = "a".repeat(5000);
    let chunks = split_message(&text, 4096);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 4096);
    assert_eq!(chunks[1].len(), 904);
}

#[test]
fn test_split_message_prefers_newlines() {
    let mut text = String::new();
    for i in 0..100 {
        text.push_str(&format!("Line {}\n", i));
    }
    let chunks = split_message(&text, 200);
    // Every chunk (except possibly the last) should end at a newline boundary
    for chunk in &chunks[..chunks.len().saturating_sub(1)] {
        assert!(
            chunk.ends_with('\n') || chunk.len() <= 200,
            "chunk should end at newline boundary: {:?}",
            &chunk[chunk.len().saturating_sub(20)..]
        );
    }
    // Recombined text should equal original (minus trailing whitespace between splits)
    let recombined: String = chunks.join("");
    // Verify no content is lost
    assert!(
        recombined.len() >= text.len() - chunks.len(),
        "content should not be lost in splitting"
    );
}

#[test]
fn test_split_message_empty() {
    let chunks = split_message("", 4096);
    assert_eq!(chunks, vec![""]);
}

#[test]
fn test_safe_log_preview_utf8_boundary() {
    let text = "这是一段包含中文和 English 的混合文本，用来验证 safe_log_preview 的 UTF-8 边界处理";
    let preview = safe_log_preview(text, 50);
    assert!(text.starts_with(preview));
    assert!(text.is_char_boundary(preview.len()));
    assert!(preview.len() <= 50);
}

fn make_test_message() -> TgMessage {
    TgMessage {
        message_id: 1,
        chat: TgChat {
            id: -100123,
            chat_type: "supergroup".to_string(),
            title: Some("Test Group".to_string()),
            is_forum: None,
        },
        from: Some(TgUser {
            id: 42,
            is_bot: false,
            first_name: "Test".to_string(),
            last_name: None,
            username: Some("testuser".to_string()),
        }),
        text: None,
        caption: None,
        date: 0,
        message_thread_id: None,
        media_group_id: None,
        reply_to_message: None,
        entities: None,
        photo: None,
        voice: None,
        audio: None,
        document: None,
        video: None,
        animation: None,
        sticker: None,
    }
}

#[test]
fn test_is_mentioned_at_username() {
    let mut msg = make_test_message();
    msg.text = Some("@garyx hello".to_string());
    assert!(is_mentioned("@garyx hello", "garyx", 999, &msg));
}

#[test]
fn test_is_mentioned_case_insensitive() {
    let mut msg = make_test_message();
    msg.text = Some("@Garyx hello".to_string());
    assert!(is_mentioned("@Garyx hello", "garyx", 999, &msg));
}

#[test]
fn test_is_mentioned_no_mention() {
    let mut msg = make_test_message();
    msg.text = Some("hello world".to_string());
    assert!(!is_mentioned("hello world", "garyx", 999, &msg));
}

#[test]
fn test_is_mentioned_reply_to_bot() {
    let bot_msg = TgMessage {
        message_id: 0,
        chat: TgChat {
            id: -100123,
            chat_type: "supergroup".to_string(),
            title: None,
            is_forum: None,
        },
        from: Some(TgUser {
            id: 999,
            is_bot: true,
            first_name: "Gary".to_string(),
            last_name: None,
            username: Some("garyx".to_string()),
        }),
        text: Some("previous message".to_string()),
        caption: None,
        date: 0,
        message_thread_id: None,
        media_group_id: None,
        reply_to_message: None,
        entities: None,
        photo: None,
        voice: None,
        audio: None,
        document: None,
        video: None,
        animation: None,
        sticker: None,
    };

    let mut msg = make_test_message();
    msg.text = Some("hello".to_string());
    msg.reply_to_message = Some(Box::new(bot_msg));

    assert!(is_mentioned("hello", "garyx", 999, &msg));
}

#[test]
fn test_is_mentioned_entity() {
    let mut msg = make_test_message();
    msg.text = Some("@garyx test".to_string());
    msg.entities = Some(vec![TgMessageEntity {
        entity_type: "mention".to_string(),
        offset: 0,
        length: 8,
    }]);
    assert!(is_mentioned("@garyx test", "garyx", 999, &msg));
}

#[test]
fn test_strip_mention() {
    assert_eq!(strip_mention("@garyx hello", "garyx"), "hello");
    assert_eq!(strip_mention("hello @Garyx", "garyx"), "hello");
    assert_eq!(
        strip_mention("hey @garyx what's up", "garyx"),
        "hey  what's up"
    );
    assert_eq!(strip_mention("hello world", "garyx"), "hello world");
}

#[test]
fn test_strip_mention_empty_username() {
    assert_eq!(strip_mention("@garyx hello", ""), "@garyx hello");
}

#[test]
fn test_native_router_commands_do_not_interrupt_inflight_streams() {
    assert!(is_native_command_text("/loop", "telegram"));
    assert!(is_native_command_text("/threads", "telegram"));
    assert!(is_native_command_text("/newthread", "telegram"));
    assert!(is_native_command_text("/threadprev", "telegram"));
    assert!(is_native_command_text("/threadnext", "telegram"));
    assert!(is_native_command_text("/loop@test_gary_bot", "telegram"));
    assert!(!is_native_command_text("hello", "telegram"));
    assert!(!is_native_command_text("/unknown", "telegram"));
    assert!(!is_native_command_text("/start", "telegram"));
    assert!(!is_native_command_text("/start", "feishu"));
}

#[test]
fn test_update_parsing() {
    let json = r#"{
        "update_id": 12345,
        "message": {
            "message_id": 1,
            "chat": {"id": -100123, "type": "supergroup", "title": "Test"},
            "from": {"id": 42, "is_bot": false, "first_name": "User"},
            "text": "hello @garyx",
            "date": 1700000000,
            "entities": [{"type": "mention", "offset": 6, "length": 8}]
        }
    }"#;
    let update: TgUpdate = serde_json::from_str(json).unwrap();
    assert_eq!(update.update_id, 12345);
    let msg = update.message.unwrap();
    assert_eq!(msg.message_id, 1);
    assert_eq!(msg.chat.id, -100123);
    assert_eq!(msg.chat.chat_type, "supergroup");
    assert_eq!(msg.text.as_deref(), Some("hello @garyx"));
    assert_eq!(msg.from.as_ref().unwrap().id, 42);
    assert_eq!(msg.entities.as_ref().unwrap().len(), 1);
    assert_eq!(msg.entities.as_ref().unwrap()[0].entity_type, "mention");
}

#[test]
fn test_update_parsing_minimal() {
    let json = r#"{"update_id": 1}"#;
    let update: TgUpdate = serde_json::from_str(json).unwrap();
    assert_eq!(update.update_id, 1);
    assert!(update.message.is_none());
}

#[test]
fn test_tg_response_parsing() {
    let json = r#"{"ok": true, "result": {"id": 123, "is_bot": true, "first_name": "Gary", "username": "garyx"}}"#;
    let resp: TgResponse<TgUser> = serde_json::from_str(json).unwrap();
    assert!(resp.ok);
    let user = resp.result.unwrap();
    assert_eq!(user.id, 123);
    assert!(user.is_bot);
    assert_eq!(user.username.as_deref(), Some("garyx"));
}

#[test]
fn test_tg_response_error() {
    let json = r#"{"ok": false, "description": "Unauthorized"}"#;
    let resp: TgResponse<TgUser> = serde_json::from_str(json).unwrap();
    assert!(!resp.ok);
    assert!(resp.result.is_none());
    assert_eq!(resp.description.as_deref(), Some("Unauthorized"));
}

#[test]
fn test_tg_response_retry_after_parameters() {
    let json =
        r#"{"ok": false, "description": "Too Many Requests", "parameters": {"retry_after": 3}}"#;
    let resp: TgResponse<TgUser> = serde_json::from_str(json).unwrap();
    assert!(!resp.ok);
    assert_eq!(
        resp.parameters
            .as_ref()
            .and_then(|parameters| parameters.retry_after),
        Some(3)
    );
}

#[cfg(test)]
mod proptest_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn split_message_no_content_loss(text in ".{0,10000}", max_len in 100usize..5000) {
            let chunks = split_message(&text, max_len);
            // Every chunk should be within limits
            for chunk in &chunks {
                prop_assert!(chunk.len() <= max_len, "chunk too long: {} > {}", chunk.len(), max_len);
            }
            // Recombined content should account for all original content
            // (some newlines may be trimmed at split boundaries)
            let total_len: usize = chunks.iter().map(|c| c.len()).sum();
            prop_assert!(total_len >= text.len().saturating_sub(chunks.len()), "lost too much content");
        }
    }
}

// -----------------------------------------------------------------------
// extract_message_content tests
// -----------------------------------------------------------------------

mod content_extraction_tests {
    use super::*;

    fn base_msg() -> TgMessage {
        make_test_message()
    }

    #[test]
    fn text_message_returns_text() {
        let mut msg = base_msg();
        msg.text = Some("hello world".to_string());
        let (text, media) = extract_message_content(&msg);
        assert_eq!(text.as_deref(), Some("hello world"));
        assert_eq!(media, None);
    }

    #[test]
    fn photo_with_caption() {
        let mut msg = base_msg();
        msg.photo = Some(vec![TgPhotoSize {
            file_id: "p1".into(),
            width: 100,
            height: 100,
        }]);
        msg.caption = Some("look at this".to_string());
        let (text, media) = extract_message_content(&msg);
        assert_eq!(text.as_deref(), Some("look at this"));
        assert_eq!(media.as_deref(), Some("photo"));
    }

    #[test]
    fn photo_without_caption() {
        let mut msg = base_msg();
        msg.photo = Some(vec![TgPhotoSize {
            file_id: "p1".into(),
            width: 100,
            height: 100,
        }]);
        let (text, media) = extract_message_content(&msg);
        assert_eq!(text.as_deref(), Some("[photo]"));
        assert_eq!(media.as_deref(), Some("photo"));
    }

    #[test]
    fn voice_message() {
        let mut msg = base_msg();
        msg.voice = Some(TgVoice {
            file_id: "v1".into(),
            duration: 10,
        });
        let (text, media) = extract_message_content(&msg);
        assert_eq!(text.as_deref(), Some("[voice message, 10s]"));
        assert_eq!(media.as_deref(), Some("voice"));
    }

    #[test]
    fn audio_message() {
        let mut msg = base_msg();
        msg.audio = Some(TgAudio {
            file_id: "a1".into(),
            duration: 180,
            title: Some("My Song".into()),
        });
        let (text, media) = extract_message_content(&msg);
        assert_eq!(text.as_deref(), Some("[My Song, 180s]"));
        assert_eq!(media.as_deref(), Some("audio"));
    }

    #[test]
    fn document_message() {
        let mut msg = base_msg();
        msg.document = Some(TgDocument {
            file_id: "d1".into(),
            file_name: Some("report.pdf".into()),
            mime_type: None,
        });
        let (text, media) = extract_message_content(&msg);
        assert_eq!(text.as_deref(), Some("[document: report.pdf]"));
        assert_eq!(media.as_deref(), Some("document"));
    }

    #[test]
    fn video_message() {
        let mut msg = base_msg();
        msg.video = Some(TgVideo {
            file_id: "vid1".into(),
            duration: 30,
            width: 1920,
            height: 1080,
        });
        let (text, media) = extract_message_content(&msg);
        assert_eq!(text.as_deref(), Some("[video, 30s]"));
        assert_eq!(media.as_deref(), Some("video"));
    }

    #[test]
    fn sticker_message() {
        let mut msg = base_msg();
        msg.sticker = Some(TgSticker {
            file_id: "s1".into(),
            emoji: Some("\u{1f600}".into()),
            is_animated: false,
            is_video: false,
        });
        let (text, media) = extract_message_content(&msg);
        assert!(text.as_deref().unwrap().contains("sticker"));
        assert_eq!(media.as_deref(), Some("sticker"));
    }

    #[test]
    fn animation_message() {
        let mut msg = base_msg();
        msg.animation = Some(TgAnimation {
            file_id: "anim1".into(),
            duration: 3,
        });
        let (text, media) = extract_message_content(&msg);
        assert_eq!(text.as_deref(), Some("[animation/GIF]"));
        assert_eq!(media.as_deref(), Some("animation"));
    }

    #[test]
    fn no_content_returns_none() {
        let msg = base_msg();
        let (text, media) = extract_message_content(&msg);
        assert!(text.is_none());
        assert!(media.is_none());
    }

    #[test]
    fn document_image_media_type_from_mime() {
        let doc = TgDocument {
            file_id: "d1".into(),
            file_name: Some("photo.bin".into()),
            mime_type: Some("image/png".into()),
        };
        assert_eq!(
            resolve_document_image_media_type(&doc).as_deref(),
            Some("image/png")
        );
    }

    #[test]
    fn document_image_media_type_from_extension() {
        let doc = TgDocument {
            file_id: "d1".into(),
            file_name: Some("photo.webp".into()),
            mime_type: Some("application/octet-stream".into()),
        };
        assert_eq!(
            resolve_document_image_media_type(&doc).as_deref(),
            Some("image/webp")
        );
    }

    #[test]
    fn document_non_image_media_type_rejected() {
        let doc = TgDocument {
            file_id: "d1".into(),
            file_name: Some("report.pdf".into()),
            mime_type: Some("application/pdf".into()),
        };
        assert!(resolve_document_image_media_type(&doc).is_none());
    }
}

// -----------------------------------------------------------------------
// reply_to_mode tests
// -----------------------------------------------------------------------

mod reply_to_mode_tests {
    use super::*;
    use garyx_models::config::ReplyToMode;

    #[test]
    fn off_never_replies() {
        assert_eq!(resolve_reply_to(&ReplyToMode::Off, 42, true), None);
        assert_eq!(resolve_reply_to(&ReplyToMode::Off, 42, false), None);
    }

    #[test]
    fn first_only_replies_first_time() {
        assert_eq!(resolve_reply_to(&ReplyToMode::First, 42, true), Some(42));
        assert_eq!(resolve_reply_to(&ReplyToMode::First, 42, false), None);
    }

    #[test]
    fn all_always_replies() {
        assert_eq!(resolve_reply_to(&ReplyToMode::All, 42, true), Some(42));
        assert_eq!(resolve_reply_to(&ReplyToMode::All, 42, false), Some(42));
    }
}

mod forum_thread_tests {
    use super::*;

    fn base_msg() -> TgMessage {
        super::make_test_message()
    }

    #[test]
    fn resolve_forum_thread_id_non_forum_ignores_thread() {
        let mut msg = base_msg();
        msg.chat.is_forum = Some(false);
        msg.message_thread_id = Some(555);
        assert_eq!(resolve_forum_thread_id(&msg), None);
    }

    #[test]
    fn resolve_forum_thread_id_forum_defaults_general() {
        let mut msg = base_msg();
        msg.chat.is_forum = Some(true);
        msg.message_thread_id = None;
        assert_eq!(
            resolve_forum_thread_id(&msg),
            Some(TELEGRAM_GENERAL_TOPIC_ID)
        );
    }

    #[test]
    fn build_group_thread_key_for_forum_topic() {
        assert_eq!(
            build_group_thread_key(-100123, true, Some(555)),
            "-100123_t555"
        );
    }

    #[test]
    fn build_group_thread_key_general_and_non_forum() {
        assert_eq!(build_group_thread_key(-100123, true, Some(1)), "-100123");
        assert_eq!(build_group_thread_key(-100123, true, None), "-100123");
        assert_eq!(build_group_thread_key(-100123, false, Some(555)), "-100123");
    }

    #[test]
    fn thread_id_resolution_for_send_and_typing() {
        assert_eq!(resolve_outbound_thread_id(true, Some(1)), None);
        assert_eq!(resolve_outbound_thread_id(true, Some(555)), Some(555));
        assert_eq!(resolve_outbound_thread_id(false, Some(555)), None);

        assert_eq!(resolve_typing_thread_id(true, Some(1)), Some(1));
        assert_eq!(resolve_typing_thread_id(true, Some(555)), Some(555));
        assert_eq!(resolve_typing_thread_id(false, Some(555)), None);
    }
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
    }

    impl TestProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
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

    async fn make_bridge() -> Arc<MultiProviderBridge> {
        let bridge = Arc::new(MultiProviderBridge::new());
        let provider = Arc::new(TestProvider::new());
        bridge.register_provider("test-provider", provider).await;
        bridge.set_default_provider_key("test-provider").await;
        bridge
    }

    #[tokio::test]
    async fn test_thread_resolution_dm() {
        let router = make_router();

        // Simulate DM thread resolution
        let thread_id = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", false, None)
        };

        assert!(is_thread_key(&thread_id));
    }

    #[tokio::test]
    async fn test_thread_resolution_group() {
        let router = make_router();

        // Simulate group thread resolution with thread id
        let thread_id = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", true, Some("-100123"))
        };

        assert!(is_thread_key(&thread_id));
    }

    #[tokio::test]
    async fn test_command_new_switches_to_named_thread() {
        let router = make_router();

        // Switch user to a custom thread first
        {
            let mut r = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "42");
            r.switch_to_thread(&user_key, "custom_session");
            assert_eq!(
                r.get_current_thread_id_for_binding("telegram", "bot1", "42"),
                Some("custom_session")
            );
        }

        // Simulate /new command behavior: create named thread and switch to it
        {
            let mut r = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "42");
            let thread_id = "thread::12345";
            r.switch_to_thread(&user_key, thread_id);
        }

        // Current thread should be switched to the named one, not reset to default
        {
            let r = router.lock().await;
            let current = r.get_current_thread_id_for_binding("telegram", "bot1", "42");
            assert_eq!(current, Some("thread::12345"));
        }
    }

    #[tokio::test]
    async fn test_command_session_navigation() {
        let router = make_router();

        // Set up thread history
        {
            let mut r = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "42");
            r.switch_to_thread(&user_key, "session_a");
            r.switch_to_thread(&user_key, "session_b");
            r.switch_to_thread(&user_key, "session_c");
        }

        // Navigate backwards (/threadprev)
        {
            let mut r = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "42");
            let prev = r.navigate_thread(&user_key, -1);
            assert_eq!(prev.as_deref(), Some("session_b"));
        }

        // Navigate forwards (/threadnext)
        {
            let mut r = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "42");
            let next = r.navigate_thread(&user_key, 1);
            assert_eq!(next.as_deref(), Some("session_c"));
        }
    }

    #[tokio::test]
    async fn test_bridge_dispatch() {
        let router = make_router();
        let bridge = make_bridge().await;

        // Resolve thread
        let thread_id = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", false, None)
        };

        assert!(is_thread_key(&thread_id));

        // Dispatch to bridge
        let result = bridge
            .start_agent_run(
                garyx_models::provider::AgentRunRequest::new(
                    &thread_id,
                    "hello world",
                    "run-test-1",
                    "telegram",
                    "bot1",
                    HashMap::new(),
                ),
                None,
            )
            .await;
        assert!(result.is_ok());

        // Give the task time to complete
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!bridge.is_run_active("run-test-1").await);
    }

    #[tokio::test]
    async fn test_bridge_dispatch_with_callback() {
        let router = make_router();
        let bridge = make_bridge().await;

        let session_key = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", false, None)
        };

        // Track callback invocations
        let callback_called = Arc::new(AtomicBool::new(false));
        let cb_flag = callback_called.clone();
        let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
            if let StreamEvent::Delta { text } = event {
                cb_flag.store(true, Ordering::Relaxed);
                assert!(text.contains("echo:"));
            }
        });

        let result = bridge
            .start_agent_run(
                garyx_models::provider::AgentRunRequest::new(
                    &session_key,
                    "hello",
                    "run-cb-1",
                    "telegram",
                    "bot1",
                    HashMap::new(),
                ),
                Some(callback),
            )
            .await;
        assert!(result.is_ok());

        // Give the task time to complete
        tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
        assert!(callback_called.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn test_reply_routing_records_and_resolves() {
        let router = make_router();

        // Record an outbound message
        {
            let mut r = router.lock().await;
            r.record_outbound_message("session_x", "telegram", "bot1", "msg100");
        }

        // Resolve via reply routing
        {
            let r = router.lock().await;
            let thread_id = r.resolve_reply_thread("telegram", "bot1", "msg100");
            assert_eq!(thread_id, Some("session_x"));
        }

        // Non-existent reply should return None
        {
            let r = router.lock().await;
            let thread_id = r.resolve_reply_thread("telegram", "bot1", "msg999");
            assert_eq!(thread_id, None);
        }
    }

    #[tokio::test]
    async fn test_thread_resolution_uses_switched_thread() {
        let router = make_router();

        // Default resolution
        let default_thread = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", false, None)
        };
        assert!(is_thread_key(&default_thread));

        // Switch to custom thread
        {
            let mut r = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "user42");
            r.switch_to_thread(&user_key, "custom_session");
        }

        // Should now resolve to the custom thread
        let switched_thread = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", false, None)
        };
        assert_eq!(switched_thread, "custom_session");
    }

    #[tokio::test]
    async fn test_thread_switch_isolated_across_accounts() {
        let router = make_router();

        {
            let mut r = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "user42");
            r.switch_to_thread(&user_key, "bot1_custom");
        }

        let bot1_thread = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", false, None)
        };
        let bot2_thread = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot2", "user42", false, None)
        };

        assert_eq!(bot1_thread, "bot1_custom");
        assert!(is_thread_key(&bot2_thread));
    }

    #[tokio::test]
    async fn test_topic_thread_switch_isolation() {
        let router = make_router();

        {
            let mut r = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "topic-1");
            r.switch_to_thread(&user_key, "topic_1_custom");
        }

        let topic_1 = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", true, Some("topic-1"))
        };
        let topic_2 = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", true, Some("topic-2"))
        };

        assert_eq!(topic_1, "topic_1_custom");
        assert!(is_thread_key(&topic_2));
    }

    #[tokio::test]
    async fn test_group_thread_isolation() {
        let router = make_router();

        // DM thread for user
        let dm_thread = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", false, None)
        };

        // Group thread for same user
        let group_thread = {
            let mut r = router.lock().await;
            r.resolve_inbound_thread("telegram", "bot1", "user42", true, Some("-100999"))
        };

        // Threads should be different
        assert_ne!(dm_thread, group_thread);
        assert!(is_thread_key(&dm_thread));
        assert!(is_thread_key(&group_thread));
    }
}

// -----------------------------------------------------------------------
// End-to-end tests with mock HTTP server
// -----------------------------------------------------------------------

mod e2e_tests {
    use super::*;
    use crate::test_helpers::*;
    use garyx_bridge::{AgentLoopProvider, BridgeError};
    use garyx_models::config::{GaryxConfig, TelegramAccount};
    use garyx_models::provider::{
        ProviderRunOptions, ProviderRunResult, ProviderType, StreamBoundaryKind, StreamEvent,
    };
    use garyx_router::{
        ChannelBinding, InMemoryThreadStore, MessageRouter, ThreadEnsureOptions, ThreadStore,
        bind_endpoint_to_thread, bindings_from_value, create_thread_record,
        detach_endpoint_from_thread, is_thread_key,
    };
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex as StdMutex, OnceLock};
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};
    type StreamCallback = garyx_bridge::provider_trait::StreamCallback;

    fn default_account() -> TelegramAccount {
        TelegramAccount {
            token: "fake-token".into(),
            enabled: true,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            groups: HashMap::new(),
        }
    }

    fn tg_mock_api_prefixes() -> &'static StdMutex<HashMap<String, String>> {
        static PREFIXES: OnceLock<StdMutex<HashMap<String, String>>> = OnceLock::new();
        PREFIXES.get_or_init(|| StdMutex::new(HashMap::new()))
    }

    fn register_unique_api_prefix(server: &MockServer) -> String {
        let prefix = format!("/__tg_mock_{}", uuid::Uuid::new_v4());
        tg_mock_api_prefixes()
            .lock()
            .unwrap()
            .insert(server.uri(), prefix.clone());
        prefix
    }

    fn unique_api_prefix(server: &MockServer) -> String {
        tg_mock_api_prefixes()
            .lock()
            .unwrap()
            .get(&server.uri())
            .cloned()
            .expect("mock API prefix should be registered")
    }

    fn unique_api_base(server: &MockServer) -> String {
        format!("{}{}", server.uri(), unique_api_prefix(server))
    }

    fn request_matches_registered_prefix(server: &MockServer, req: &wiremock::Request) -> bool {
        tg_mock_api_prefixes()
            .lock()
            .unwrap()
            .get(&server.uri())
            .is_none_or(|prefix| req.url.path().starts_with(prefix))
    }

    /// Start a mock server with Telegram sendMessage and sendChatAction mocked.
    async fn setup_tg_mock() -> MockServer {
        let server = MockServer::start().await;
        let api_prefix = register_unique_api_prefix(&server);

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"{api_prefix}/bot.+/sendMessage")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {
                    "message_id": 999,
                    "chat": {"id": 42, "type": "private"},
                    "date": 1700000000,
                    "text": "response"
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"{api_prefix}/bot.+/sendChatAction")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": true})),
            )
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"{api_prefix}/bot.+/sendPhoto")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {
                    "message_id": 1001,
                    "chat": {"id": 42, "type": "private"},
                    "date": 1700000000
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"{api_prefix}/bot.+/editMessageText")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {
                    "message_id": 999,
                    "chat": {"id": 42, "type": "private"},
                    "date": 1700000000,
                    "text": "edited response"
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex(&format!(r"{api_prefix}/bot.+/getFile")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {
                    "file_id": "photo_file_id",
                    "file_path": "photos/test_image.png",
                    "file_size": 4
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex(&format!(
                r"{api_prefix}/file/bot.+/photos/test_image\.png"
            )))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(vec![1_u8, 2_u8, 3_u8, 4_u8], "application/octet-stream"),
            )
            .mount(&server)
            .await;

        server
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
            origin_channel: Some("telegram".to_owned()),
            origin_account_id: Some(account_id.to_owned()),
            origin_from_id: Some(binding_key.to_owned()),
            is_group: Some(false),
        };
        let (thread_id, _) = create_thread_record(store, options).await.unwrap();
        bind_endpoint_to_thread(
            store,
            &thread_id,
            ChannelBinding {
                channel: "telegram".to_owned(),
                account_id: account_id.to_owned(),
                binding_key: binding_key.to_owned(),
                chat_id: binding_key.to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: binding_key.to_owned(),
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
    ) -> Vec<wiremock::Request>
    where
        F: FnMut(&wiremock::Request) -> bool,
    {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let requests = server.received_requests().await.unwrap_or_default();
            let matching: Vec<_> = requests
                .into_iter()
                .filter(|r| request_matches_registered_prefix(server, r))
                .filter(|r| predicate(r))
                .collect();
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
    ) -> Vec<wiremock::Request> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut stable_since = tokio::time::Instant::now();
        let mut last_count = usize::MAX;
        loop {
            let requests: Vec<_> = server
                .received_requests()
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|r| request_matches_registered_prefix(server, r))
                .collect();
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
    ) -> Vec<wiremock::Request>
    where
        F: FnMut(&wiremock::Request) -> bool,
    {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut stable_since = tokio::time::Instant::now();
        let mut last_count = usize::MAX;
        loop {
            let requests = server.received_requests().await.unwrap_or_default();
            let matching: Vec<_> = requests
                .into_iter()
                .filter(|r| request_matches_registered_prefix(server, r))
                .filter(|r| predicate(r))
                .collect();
            let count = matching.len();
            let now = tokio::time::Instant::now();
            if count != last_count {
                last_count = count;
                stable_since = now;
            } else if count >= expected_min && now.duration_since(stable_since) >= quiet_for {
                return matching;
            }
            if now >= deadline {
                let all_requests: Vec<_> = server
                    .received_requests()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|r| request_matches_registered_prefix(server, r))
                    .collect();
                let all_summaries: Vec<String> = all_requests
                    .iter()
                    .map(|req| {
                        let body = std::str::from_utf8(&req.body).unwrap_or("<non-utf8>");
                        format!("{} {} {}", req.method, req.url.path(), body)
                    })
                    .collect();
                let summaries: Vec<String> = matching
                    .iter()
                    .map(|req| {
                        let body = std::str::from_utf8(&req.body).unwrap_or("<non-utf8>");
                        format!("{} {} {}", req.method, req.url.path(), body)
                    })
                    .collect();
                panic!(
                    "timed out waiting for matching request quiet window: expected >= {expected_min}, got {count}; matching={summaries:?}; all={all_summaries:?}"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[derive(Clone, Default)]
    struct TelegramRequestCapture {
        send_messages: Arc<StdMutex<Vec<Value>>>,
        edit_messages: Arc<StdMutex<Vec<Value>>>,
        delete_messages: Arc<StdMutex<Vec<Value>>>,
        chat_actions: Arc<StdMutex<Vec<Value>>>,
    }

    impl TelegramRequestCapture {
        fn push_send(&self, req: &Request) {
            let body = serde_json::from_slice(&req.body).expect("valid sendMessage body");
            self.send_messages.lock().unwrap().push(body);
        }

        fn push_edit(&self, req: &Request) {
            let body = serde_json::from_slice(&req.body).expect("valid editMessageText body");
            self.edit_messages.lock().unwrap().push(body);
        }

        fn push_delete(&self, req: &Request) {
            let body = serde_json::from_slice(&req.body).expect("valid deleteMessage body");
            self.delete_messages.lock().unwrap().push(body);
        }

        fn push_action(&self, req: &Request) {
            let body = serde_json::from_slice(&req.body).expect("valid sendChatAction body");
            self.chat_actions.lock().unwrap().push(body);
        }

        fn send_bodies(&self) -> Vec<Value> {
            self.send_messages.lock().unwrap().clone()
        }

        fn edit_bodies(&self) -> Vec<Value> {
            self.edit_messages.lock().unwrap().clone()
        }

        fn delete_bodies(&self) -> Vec<Value> {
            self.delete_messages.lock().unwrap().clone()
        }
    }

    async fn wait_for_json_capture_len(
        capture: &Arc<StdMutex<Vec<Value>>>,
        expected_min: usize,
        timeout: std::time::Duration,
    ) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if capture.lock().unwrap().len() >= expected_min {
                return;
            }
            let captured_snapshot = capture.lock().unwrap().clone();
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for captured requests: expected >= {expected_min}, got {}; captured={:?}",
                captured_snapshot.len(),
                captured_snapshot
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_json_capture_quiet_window(
        capture: &Arc<StdMutex<Vec<Value>>>,
        quiet_for: std::time::Duration,
        timeout: std::time::Duration,
        expected_min: usize,
    ) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut stable_since: Option<tokio::time::Instant> = None;
        let mut last_len = 0usize;

        loop {
            let snapshot = capture.lock().unwrap().clone();
            let len = snapshot.len();
            let now = tokio::time::Instant::now();
            if len != last_len {
                last_len = len;
                stable_since = Some(now);
            } else if len >= expected_min
                && stable_since.is_some_and(|since| now.duration_since(since) >= quiet_for)
            {
                return snapshot;
            }

            assert!(
                now < deadline,
                "timed out waiting for captured quiet window: expected >= {expected_min}, got {len}; captured={snapshot:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_json_capture_match(
        capture: &Arc<StdMutex<Vec<Value>>>,
        timeout: std::time::Duration,
        predicate: impl Fn(&Value) -> bool,
    ) -> Value {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let snapshot = capture.lock().unwrap().clone();
            if let Some(body) = snapshot.iter().find(|body| predicate(body)) {
                return body.clone();
            }

            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for captured JSON body match; captured={snapshot:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_telegram_render_body(
        capture: &TelegramRequestCapture,
        timeout: std::time::Duration,
        predicate: impl Fn(&Value) -> bool,
    ) -> Value {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let send_bodies = capture.send_bodies();
            if let Some(body) = send_bodies.iter().find(|body| predicate(body)) {
                return body.clone();
            }

            let edit_bodies = capture.edit_bodies();
            if let Some(body) = edit_bodies.iter().find(|body| predicate(body)) {
                return body.clone();
            }

            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for rendered Telegram body; send={send_bodies:?}; edit={edit_bodies:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    fn body_text(body: &Value) -> String {
        body["text"].as_str().unwrap_or_default().to_owned()
    }

    fn looks_like_loading_indicator(text: &str) -> bool {
        matches!(text, "." | ".." | "...")
            || text.ends_with("\n\n...")
            || text.ends_with("\n\n..")
            || text.ends_with("\n\n.")
    }

    fn assert_no_loading_indicator(texts: &[String]) {
        assert!(
            texts.iter().all(|text| !looks_like_loading_indicator(text)),
            "Telegram streaming should not render legacy loading indicators: {texts:?}"
        );
    }

    fn has_legacy_loading_suffix(text: &str) -> bool {
        text.ends_with("\n\n.") || text.ends_with("\n\n..") || text.ends_with("\n\n...")
    }

    async fn setup_tg_capture_mock(
        guard_edit_length: bool,
    ) -> (MockServer, TelegramRequestCapture) {
        let server = MockServer::start().await;
        let api_prefix = register_unique_api_prefix(&server);
        let capture = TelegramRequestCapture::default();
        let next_message_id = Arc::new(AtomicUsize::new(1000));

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"{api_prefix}/bot.+/sendMessage")))
            .respond_with({
                let capture = capture.clone();
                let next_message_id = next_message_id.clone();
                move |req: &Request| {
                    capture.push_send(req);
                    let message_id = next_message_id.fetch_add(1, Ordering::Relaxed) as i64;
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "ok": true,
                        "result": {
                            "message_id": message_id,
                            "chat": {"id": 42, "type": "private"},
                            "date": 1700000000,
                            "text": "response"
                        }
                    }))
                }
            })
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"{api_prefix}/bot.+/sendChatAction")))
            .respond_with({
                let capture = capture.clone();
                move |req: &Request| {
                    capture.push_action(req);
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"ok": true, "result": true}))
                }
            })
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"{api_prefix}/bot.+/editMessageText")))
            .respond_with({
                let capture = capture.clone();
                move |req: &Request| {
                    capture.push_edit(req);
                    let body: Value =
                        serde_json::from_slice(&req.body).expect("valid editMessageText body");
                    let text = body["text"].as_str().unwrap_or_default();
                    if guard_edit_length && text.len() > MAX_MESSAGE_LENGTH {
                        ResponseTemplate::new(200).set_body_json(serde_json::json!({
                            "ok": false,
                            "description": "Bad Request: message is too long"
                        }))
                    } else {
                        ResponseTemplate::new(200).set_body_json(serde_json::json!({
                            "ok": true,
                            "result": {
                                "message_id": body["message_id"].as_i64().unwrap_or(999),
                                "chat": {"id": 42, "type": "private"},
                                "date": 1700000000,
                                "text": text
                            }
                        }))
                    }
                }
            })
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"{api_prefix}/bot.+/deleteMessage")))
            .respond_with({
                let capture = capture.clone();
                move |req: &Request| {
                    capture.push_delete(req);
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"ok": true, "result": true}))
                }
            })
            .mount(&server)
            .await;

        (server, capture)
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

    struct StreamingFirstChunkProvider {
        call_count: AtomicUsize,
    }

    impl StreamingFirstChunkProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AgentLoopProvider for StreamingFirstChunkProvider {
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
            on_chunk(StreamEvent::Delta {
                text: "你好".to_owned(),
            });
            on_chunk(StreamEvent::Delta {
                text: "！👋".to_owned(),
            });
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "stream-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "你好！👋".to_owned(),
                session_messages: vec![],
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 10,
                output_tokens: 5,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    struct StreamingToolPlaceholderProvider {
        call_count: AtomicUsize,
    }

    impl StreamingToolPlaceholderProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AgentLoopProvider for StreamingToolPlaceholderProvider {
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
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Bash"}),
                    Some("tool-bash".to_owned()),
                    Some("Bash".to_owned()),
                ),
            });
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Read"}),
                    Some("tool-read".to_owned()),
                    Some("Read".to_owned()),
                ),
            });
            on_chunk(StreamEvent::Delta {
                text: "done".to_owned(),
            });
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Write"}),
                    Some("tool-write".to_owned()),
                    Some("Write".to_owned()),
                ),
            });
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Search"}),
                    Some("tool-search".to_owned()),
                    Some("Search".to_owned()),
                ),
            });
            on_chunk(StreamEvent::Delta {
                text: "\nnext".to_owned(),
            });
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "stream-tools".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "done\nnext".to_owned(),
                session_messages: vec![],
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 10,
                output_tokens: 5,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    struct StreamingChildToolPlaceholderProvider {
        call_count: AtomicUsize,
    }

    impl StreamingChildToolPlaceholderProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AgentLoopProvider for StreamingChildToolPlaceholderProvider {
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
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Bash"}),
                    Some("tool-child-bash".to_owned()),
                    Some("Bash".to_owned()),
                )
                .with_metadata_value("parent_tool_use_id", serde_json::json!("tool-parent")),
            });
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Read"}),
                    Some("tool-agent-read".to_owned()),
                    Some("Read".to_owned()),
                )
                .with_metadata_value("agent_id", serde_json::json!("coder"))
                .with_metadata_value("agent_display_name", serde_json::json!("Coder")),
            });
            on_chunk(StreamEvent::Delta {
                text: "done".to_owned(),
            });
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "stream-child-tools".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "done".to_owned(),
                session_messages: vec![],
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 10,
                output_tokens: 5,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    struct StreamingToolOnlyProvider {
        call_count: AtomicUsize,
    }

    impl StreamingToolOnlyProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AgentLoopProvider for StreamingToolOnlyProvider {
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
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Read"}),
                    Some("tool-read-1".to_owned()),
                    Some("Read".to_owned()),
                ),
            });
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Read"}),
                    Some("tool-read-2".to_owned()),
                    Some("Read".to_owned()),
                ),
            });
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Bash"}),
                    Some("tool-bash".to_owned()),
                    Some("Bash".to_owned()),
                ),
            });
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "stream-tools-only".to_owned(),
                thread_id: options.thread_id.clone(),
                response: String::new(),
                session_messages: vec![],
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 10,
                output_tokens: 0,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    struct StreamingToolBoundaryProvider {
        call_count: AtomicUsize,
    }

    impl StreamingToolBoundaryProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AgentLoopProvider for StreamingToolBoundaryProvider {
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
            on_chunk(StreamEvent::ToolUse {
                message: garyx_models::ProviderMessage::tool_use(
                    serde_json::json!({"name": "Bash"}),
                    Some("tool-bash".to_owned()),
                    Some("Bash".to_owned()),
                ),
            });
            on_chunk(StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: None,
            });
            on_chunk(StreamEvent::Delta {
                text: "after".to_owned(),
            });
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "stream-tool-boundary".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "after".to_owned(),
                session_messages: vec![],
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 10,
                output_tokens: 5,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    struct StreamingSegmentBoundaryProvider {
        call_count: AtomicUsize,
    }

    impl StreamingSegmentBoundaryProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    struct StreamingAssistantSegmentProvider {
        call_count: AtomicUsize,
    }

    impl StreamingAssistantSegmentProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AgentLoopProvider for StreamingSegmentBoundaryProvider {
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
                run_id: "segment-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "第一段第二段".to_owned(),
                session_messages: vec![],
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

    #[async_trait]
    impl AgentLoopProvider for StreamingAssistantSegmentProvider {
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
                run_id: "assistant-segment-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "第一段\n\n第二段".to_owned(),
                session_messages: vec![],
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

    struct StreamingDelayedFlushProvider {
        call_count: AtomicUsize,
    }

    impl StreamingDelayedFlushProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    struct StreamingSlowProvider {
        call_count: AtomicUsize,
    }

    impl StreamingSlowProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    struct StreamingOverflowProvider {
        call_count: AtomicUsize,
    }

    impl StreamingOverflowProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    struct StreamingOverflowBoundaryProvider {
        call_count: AtomicUsize,
    }

    impl StreamingOverflowBoundaryProvider {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AgentLoopProvider for StreamingOverflowProvider {
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
            on_chunk(StreamEvent::Delta {
                text: "a".repeat(4000),
            });
            on_chunk(StreamEvent::Delta {
                text: "b".repeat(200),
            });
            on_chunk(StreamEvent::Delta {
                text: "c".repeat(200),
            });
            on_chunk(StreamEvent::Done);

            Ok(ProviderRunResult {
                run_id: "overflow-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: format!("{}{}{}", "a".repeat(4000), "b".repeat(200), "c".repeat(200)),
                session_messages: vec![],
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

    #[async_trait]
    impl AgentLoopProvider for StreamingOverflowBoundaryProvider {
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
            on_chunk(StreamEvent::Delta {
                text: "a".repeat(4000),
            });
            on_chunk(StreamEvent::Delta {
                text: "b".repeat(200),
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
                run_id: "overflow-boundary-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: format!("{}{}第二段", "a".repeat(4000), "b".repeat(200)),
                session_messages: vec![],
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

    #[async_trait]
    impl AgentLoopProvider for StreamingSlowProvider {
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
            on_chunk(StreamEvent::Delta {
                text: "thinking".to_owned(),
            });
            tokio::time::sleep(std::time::Duration::from_millis(1600)).await;
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "slow-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "thinking".to_owned(),
                session_messages: vec![],
                sdk_session_id: None,
                actual_model: None,
                success: true,
                error: None,
                input_tokens: 1,
                output_tokens: 1,
                cost: 0.0,
                duration_ms: 1600,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    #[async_trait]
    impl AgentLoopProvider for StreamingDelayedFlushProvider {
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
            on_chunk(StreamEvent::Delta {
                text: "p".to_owned(),
            });
            on_chunk(StreamEvent::Delta {
                text: "rovider".to_owned(),
            });

            tokio::time::sleep(std::time::Duration::from_millis(400)).await;

            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "delayed-flush-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "provider".to_owned(),
                session_messages: vec![],
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

    struct StreamingInterruptAwareProvider {
        run_count: AtomicUsize,
        pwd_call_count: AtomicUsize,
        active_session: tokio::sync::Mutex<Option<String>>,
        queued_inputs: tokio::sync::Mutex<Vec<String>>,
    }

    impl StreamingInterruptAwareProvider {
        fn new() -> Self {
            Self {
                run_count: AtomicUsize::new(0),
                pwd_call_count: AtomicUsize::new(0),
                active_session: tokio::sync::Mutex::new(None),
                queued_inputs: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl AgentLoopProvider for StreamingInterruptAwareProvider {
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
            self.run_count.fetch_add(1, Ordering::Relaxed);
            {
                let mut active = self.active_session.lock().await;
                *active = Some(options.thread_id.clone());
            }

            let mut lines = Vec::new();
            for idx in 0..10 {
                if idx > 0 {
                    let queued = {
                        let mut inputs = self.queued_inputs.lock().await;
                        std::mem::take(&mut *inputs)
                    };
                    if queued.iter().any(|m| m.contains("停止")) {
                        break;
                    }
                }

                let call_idx = self.pwd_call_count.fetch_add(1, Ordering::Relaxed) + 1;
                let line = format!("pwd call #{call_idx}: /tmp/test-workspace");
                lines.push(line.clone());
                on_chunk(StreamEvent::Delta {
                    text: format!("{line}\n"),
                });

                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }

            {
                let mut active = self.active_session.lock().await;
                *active = None;
            }
            on_chunk(StreamEvent::Done);

            Ok(ProviderRunResult {
                run_id: "interrupt-aware-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: lines.join("\n"),
                session_messages: vec![],
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

        async fn add_streaming_input(
            &self,
            session_key: &str,
            input: garyx_models::provider::QueuedUserInput,
        ) -> bool {
            let active = { self.active_session.lock().await.clone() };
            if active.as_deref() != Some(session_key) {
                return false;
            }
            let mut inputs = self.queued_inputs.lock().await;
            let image_count = input.images.len();
            inputs.push(format!("{}|images={image_count}", input.message));
            true
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    #[tokio::test]
    async fn test_send_photo_ok() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let http = reqwest::Client::new();

        let tmp_path = std::env::temp_dir().join("garyx_send_photo_test.png");
        std::fs::write(&tmp_path, [1_u8, 2_u8, 3_u8, 4_u8]).unwrap();

        let result = send_photo(
            TelegramSendTarget::new(&http, "fake-token", 42, None, &api_base),
            &tmp_path,
            Some("caption"),
            None,
        )
        .await;

        let _ = std::fs::remove_file(&tmp_path);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1001);
    }

    #[tokio::test]
    async fn test_e2e_telegram_dm_full_chain() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::dm(42, "hello world").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        // Wait for debounce window + background bridge dispatch to complete.
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while provider.call_count.load(Ordering::Relaxed) < 1 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("media group dispatch should happen within timeout");

        // Verify provider was called with the correct thread and message
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let session_key = {
            let calls = provider.calls.lock().unwrap();
            assert_eq!(calls[0].message, "hello world");
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
                .resolve_endpoint_thread_id("telegram", "bot1", "42")
                .await
        };
        assert_eq!(current.as_deref(), Some(session_key.as_str()));
        let persisted = store.get(&session_key).await.expect("thread should exist");
        let bindings = bindings_from_value(&persisted);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].endpoint_key(), "telegram::bot1::42");

        // Verify sendMessage was called on mock server
        let send_msg_requests =
            wait_for_matching_requests(&server, std::time::Duration::from_secs(5), 1, |r| {
                r.url.path().contains("sendMessage")
            })
            .await;
        assert!(
            !send_msg_requests.is_empty(),
            "sendMessage should have been called"
        );

        // Verify request body contains correct chat_id and echo response
        let body: serde_json::Value = serde_json::from_slice(&send_msg_requests[0].body).unwrap();
        assert_eq!(body["chat_id"], 42);
        assert!(
            body["text"].as_str().unwrap().contains("echo: hello world"),
            "response text should contain echo"
        );
    }

    #[tokio::test]
    async fn test_e2e_streaming_first_chunk_replies_and_final_keeps_plain_text() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingFirstChunkProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "你好").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while provider.call_count.load(Ordering::Relaxed) < 1 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("media group dispatch should happen within timeout");
        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;
        let send_body = send_bodies
            .iter()
            .find(|body| body["text"].as_str().unwrap_or_default().contains("你好"))
            .expect("first streaming send should contain the first chunk");
        assert_eq!(send_body["reply_to_message_id"], 100);
        let final_body =
            wait_for_telegram_render_body(&capture, std::time::Duration::from_secs(5), |body| {
                body["text"].as_str().unwrap_or_default() == "你好！👋"
            })
            .await;
        let final_text = final_body["text"].as_str().unwrap();
        assert_eq!(final_text, "你好！👋");
        assert!(final_body["parse_mode"].is_null());
    }

    #[tokio::test]
    async fn test_e2e_streaming_does_not_append_loading_indicator() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingSlowProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "slow").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;

        let first =
            wait_for_telegram_render_body(&capture, std::time::Duration::from_secs(5), |body| {
                body["text"].as_str().unwrap_or_default() == "thinking"
            })
            .await;
        assert_eq!(first["text"], "thinking");

        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        let all_texts = capture
            .send_bodies()
            .into_iter()
            .chain(capture.edit_bodies().into_iter())
            .filter_map(|body| body["text"].as_str().map(ToOwned::to_owned))
            .collect::<Vec<_>>();
        assert_no_loading_indicator(&all_texts);
        assert!(
            all_texts.iter().all(|text| text == "thinking"),
            "slow streaming should render only real text, not loading frames: {all_texts:?}"
        );
    }

    #[tokio::test]
    async fn test_e2e_streaming_tool_placeholders_are_replaced_by_text() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingToolPlaceholderProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "run tools").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;

        let first_placeholder =
            wait_for_telegram_render_body(&capture, std::time::Duration::from_secs(5), |body| {
                body_text(body) == "🔧 #1 Bash"
            })
            .await;
        assert_eq!(body_text(&first_placeholder), "🔧 #1 Bash");

        let final_body =
            wait_for_telegram_render_body(&capture, std::time::Duration::from_secs(5), |body| {
                body["text"].as_str().unwrap_or_default() == "done\nnext"
            })
            .await;
        assert_eq!(final_body["text"], "done\nnext");

        let all_texts = capture
            .send_bodies()
            .into_iter()
            .chain(capture.edit_bodies().into_iter())
            .filter_map(|body| body["text"].as_str().map(ToOwned::to_owned))
            .collect::<Vec<_>>();
        assert!(
            all_texts.iter().any(|text| text == "🔧 #1 Bash"),
            "first tool placeholder should show sequence number: {all_texts:?}"
        );
        assert!(
            all_texts.iter().any(|text| {
                text.contains("done")
                    && !text.contains("🔧 #1 Bash")
                    && !text.contains("🔧 #2 Read")
            }),
            "pre-text tool placeholders should be overwritten by first text: {all_texts:?}"
        );
        assert!(
            all_texts.iter().any(|text| text == "done\nnext"),
            "post-text tool placeholder should be overwritten by later text in the same message: {all_texts:?}"
        );
        assert_eq!(
            all_texts
                .iter()
                .filter(|text| text.contains("🔧") || text.contains("done"))
                .count(),
            2,
            "pending Telegram edits should coalesce to the latest visible state: {all_texts:?}"
        );
        assert_no_loading_indicator(&all_texts);

        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;
        let relevant_sends = send_bodies
            .iter()
            .filter(|body| {
                body["text"].as_str().is_some_and(|text| {
                    text.contains("🔧") || text.contains("done") || text.contains("next")
                })
            })
            .count();
        assert_eq!(
            relevant_sends, 1,
            "tool and text phases in one assistant response should reuse one Telegram message"
        );
    }

    #[tokio::test]
    async fn test_e2e_streaming_child_agent_tool_placeholders_are_suppressed() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingChildToolPlaceholderProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "run child tools").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;

        let final_body =
            wait_for_telegram_render_body(&capture, std::time::Duration::from_secs(5), |body| {
                body["text"].as_str().unwrap_or_default() == "done"
            })
            .await;
        assert_eq!(final_body["text"], "done");

        let all_texts = capture
            .send_bodies()
            .into_iter()
            .chain(capture.edit_bodies().into_iter())
            .filter_map(|body| body["text"].as_str().map(ToOwned::to_owned))
            .collect::<Vec<_>>();
        assert!(
            all_texts.iter().all(|text| !text.contains("🔧")),
            "child-agent tool placeholders should not be sent to Telegram: {all_texts:?}"
        );
        assert!(
            all_texts
                .iter()
                .all(|text| !text.contains("Bash") && !text.contains("Read")),
            "child-agent tool names should stay hidden from Telegram: {all_texts:?}"
        );

        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;
        let relevant_sends = send_bodies
            .iter()
            .filter(|body| {
                body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("done") || text.contains("🔧"))
            })
            .count();
        assert_eq!(
            relevant_sends, 1,
            "hidden child-agent tool events should not create Telegram messages"
        );
    }

    #[tokio::test]
    async fn test_e2e_streaming_tool_only_placeholders_show_all_then_delete_on_done() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingToolOnlyProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "only tools").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;

        let first_tool =
            wait_for_telegram_render_body(&capture, std::time::Duration::from_secs(5), |body| {
                body_text(body) == "🔧 #1 Read"
            })
            .await;
        assert_eq!(body_text(&first_tool), "🔧 #1 Read");

        let all_texts = capture
            .send_bodies()
            .into_iter()
            .chain(capture.edit_bodies().into_iter())
            .filter_map(|body| body["text"].as_str().map(ToOwned::to_owned))
            .collect::<Vec<_>>();
        assert!(
            all_texts.iter().any(|text| text == "🔧 #1 Read"),
            "first tool should render as a numbered single-line placeholder: {all_texts:?}"
        );
        assert!(
            all_texts
                .iter()
                .filter(|text| text.contains("🔧"))
                .all(|text| text == "🔧 #1 Read"),
            "pending tool-only edits should be dropped when Done deletes the runtime placeholder: {all_texts:?}"
        );
        assert!(
            all_texts.iter().all(|text| text.lines().count() == 1),
            "tool-only placeholders should not accumulate multiple lines: {all_texts:?}"
        );
        assert_no_loading_indicator(&all_texts);

        wait_for_json_capture_len(
            &capture.delete_messages,
            1,
            std::time::Duration::from_secs(5),
        )
        .await;
        let deletes = capture.delete_bodies();
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0]["message_id"], 1000);
    }

    #[tokio::test]
    async fn test_e2e_streaming_user_ack_deletes_runtime_only_tool_placeholder() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingToolBoundaryProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "interrupt tools").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;

        let placeholder =
            wait_for_telegram_render_body(&capture, std::time::Duration::from_secs(5), |body| {
                body_text(body) == "🔧 #1 Bash"
            })
            .await;
        assert_eq!(body_text(&placeholder), "🔧 #1 Bash");

        wait_for_json_capture_len(
            &capture.delete_messages,
            1,
            std::time::Duration::from_secs(5),
        )
        .await;
        let deletes = capture.delete_bodies();
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0]["message_id"], 1000);

        let final_body =
            wait_for_telegram_render_body(&capture, std::time::Duration::from_secs(5), |body| {
                body["text"].as_str().unwrap_or_default() == "after"
            })
            .await;
        assert_eq!(final_body["text"], "after");

        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            2,
        )
        .await;
        let runtime_or_after_sends = send_bodies
            .iter()
            .filter_map(|body| body["text"].as_str())
            .filter(|text| text.contains("🔧") || *text == "after")
            .collect::<Vec<_>>();
        assert_eq!(
            runtime_or_after_sends.len(),
            2,
            "new user input should clear runtime-only placeholder and start a fresh Telegram message"
        );
    }

    #[tokio::test]
    async fn test_e2e_streaming_user_ack_boundary_splits_telegram_messages() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingSegmentBoundaryProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let router = make_router_with_store(store.clone());
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "split me").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        let first_send = wait_for_json_capture_match(
            &capture.send_messages,
            std::time::Duration::from_secs(5),
            |body| body["text"].as_str().unwrap_or_default().contains("第一段"),
        )
        .await;
        let second_send = wait_for_json_capture_match(
            &capture.send_messages,
            std::time::Duration::from_secs(5),
            |body| body["text"].as_str().unwrap_or_default().contains("第二段"),
        )
        .await;
        assert!(
            first_send["text"]
                .as_str()
                .unwrap_or_default()
                .contains("第一段"),
            "first segment should be sent as independent message"
        );
        assert!(
            second_send["text"]
                .as_str()
                .unwrap_or_default()
                .contains("第二段"),
            "second segment should start as a fresh message after boundary"
        );
        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            2,
        )
        .await;
        assert_eq!(
            send_bodies
                .iter()
                .filter(|body| {
                    body["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("第一段") || text.contains("第二段"))
                })
                .count(),
            2,
            "user ack boundary should fan out into two separate Telegram messages"
        );
        let final_texts = capture
            .send_bodies()
            .into_iter()
            .chain(capture.edit_bodies().into_iter())
            .filter_map(|body| body["text"].as_str().map(ToOwned::to_owned))
            .collect::<Vec<_>>();
        assert!(final_texts.iter().any(|text| text == "第一段"));
        assert!(final_texts.iter().any(|text| text == "第二段"));

        let _ = store;
    }

    #[tokio::test]
    async fn test_e2e_streaming_assistant_segment_keeps_single_telegram_message() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingAssistantSegmentProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "segment me").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        let _first_send = wait_for_json_capture_match(
            &capture.send_messages,
            std::time::Duration::from_secs(5),
            |body| body["text"].as_str().unwrap_or_default().contains("第一段"),
        )
        .await;
        let final_body = wait_for_json_capture_match(
            &capture.edit_messages,
            std::time::Duration::from_secs(5),
            |body| {
                body["text"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("第一段\n\n第二段")
            },
        )
        .await;
        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;
        let relevant_send_bodies: Vec<&Value> = send_bodies
            .iter()
            .filter(|body| {
                body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("第一段") || text.contains("第二段"))
            })
            .collect();

        assert_eq!(
            relevant_send_bodies.len(),
            1,
            "assistant segment boundaries must not create extra Telegram messages"
        );
        assert_eq!(body_text(relevant_send_bodies[0]), "第一段");
        let final_text = final_body["text"].as_str().unwrap_or_default();
        assert!(final_text.contains("第一段\n\n第二段"));
    }

    #[tokio::test]
    async fn test_e2e_streaming_throttled_updates_flush_after_pause() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingDelayedFlushProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "flush me").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        wait_for_json_capture_len(&capture.edit_messages, 1, std::time::Duration::from_secs(5))
            .await;
        let edit_bodies = wait_for_json_capture_quiet_window(
            &capture.edit_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;

        assert!(
            edit_bodies
                .iter()
                .any(|b| b["text"].as_str().unwrap_or_default() == "provider"),
            "throttled stream should flush pending text before the final Done event"
        );
    }

    #[tokio::test]
    async fn test_e2e_streaming_rolls_over_long_segments_without_resending_full_history() {
        let (server, capture) = setup_tg_capture_mock(true).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingOverflowProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "overflow me").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        let continuation_chunk = wait_for_json_capture_match(
            &capture.send_messages,
            std::time::Duration::from_secs(5),
            |body| {
                body["text"].as_str().is_some_and(|text| {
                    !text.is_empty()
                        && text.len() < 1000
                        && !text.starts_with('a')
                        && text.chars().all(|ch| matches!(ch, 'a' | 'b' | 'c'))
                })
            },
        )
        .await;
        let first_segment =
            wait_for_telegram_render_body(&capture, std::time::Duration::from_secs(5), |body| {
                body["text"].as_str().is_some_and(|text| {
                    text.len() == 4000 && text.chars().all(|ch| matches!(ch, 'a' | 'b' | 'c'))
                })
            })
            .await;
        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            2,
        )
        .await;
        let edit_bodies = wait_for_json_capture_quiet_window(
            &capture.edit_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;
        let rendered_lengths: Vec<usize> = send_bodies
            .iter()
            .chain(edit_bodies.iter())
            .filter_map(|body| body["text"].as_str().map(str::len))
            .collect();
        let relevant_send_bodies: Vec<&Value> = send_bodies
            .iter()
            .filter(|body| {
                body["text"].as_str().is_some_and(|text| {
                    !text.is_empty() && text.chars().all(|ch| matches!(ch, 'a' | 'b' | 'c'))
                })
            })
            .collect();

        assert_eq!(
            relevant_send_bodies.len(),
            2,
            "stream rollover should only send a continuation chunk"
        );
        assert!(
            first_segment["text"]
                .as_str()
                .is_some_and(|text| text.len() == 4000),
            "rollover path should preserve a 4000-char first segment; rendered lengths={rendered_lengths:?}"
        );
        assert!(
            edit_bodies.iter().any(|body| {
                body["text"].as_str().is_some_and(|text| {
                    text.len() == MAX_MESSAGE_LENGTH
                        && !has_legacy_loading_suffix(text)
                        && !text.contains("🔧")
                })
            }),
            "rollover should clean runtime state before finalizing the previous Telegram message; edits={edit_bodies:?}"
        );
        let continuation_len = continuation_chunk["text"]
            .as_str()
            .unwrap_or_default()
            .len();
        assert!(
            (1..1000).contains(&continuation_len),
            "stream rollover should only send a bounded continuation chunk, got len={continuation_len}"
        );
        assert!(
            edit_bodies
                .iter()
                .all(|body| body["text"].as_str().unwrap_or_default().len() <= MAX_MESSAGE_LENGTH),
            "streaming edits must stay within Telegram's max message length"
        );
        assert!(
            send_bodies
                .iter()
                .all(|body| body["text"].as_str().unwrap_or_default().len() <= MAX_MESSAGE_LENGTH),
            "stream continuation sends must stay within Telegram's max message length"
        );
    }

    #[tokio::test]
    async fn test_e2e_streaming_boundary_after_overflow_does_not_resend_prior_segments() {
        let (server, capture) = setup_tg_capture_mock(true).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingOverflowBoundaryProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let account = default_account();
        let update = TgUpdateBuilder::dm(42, "overflow boundary").build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        wait_for_json_capture_len(&capture.send_messages, 2, std::time::Duration::from_secs(5))
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let send_bodies = capture.send_bodies();
        let edit_bodies = capture.edit_bodies();

        assert!(
            send_bodies.len() >= 2,
            "boundary after rollover should send overflow continuation bodies"
        );
        assert_eq!(
            send_bodies
                .iter()
                .filter(|body| body["text"].as_str().unwrap_or_default().len() >= 4000)
                .count(),
            1,
            "overflow recovery should not resend the already-started long segment from scratch"
        );
        assert!(
            send_bodies.iter().any(|body| {
                let text = body["text"].as_str().unwrap_or_default();
                text.len() == 104 && text.chars().all(|ch| ch == 'b')
            }),
            "overflow continuation should only send the tail chunk after the first message expands to 4096 chars"
        );
        assert!(
            send_bodies
                .iter()
                .all(|body| body["text"].as_str().unwrap_or_default().len() <= MAX_MESSAGE_LENGTH),
            "overflow path should keep outbound chunks within Telegram max length"
        );
        assert!(
            edit_bodies
                .iter()
                .all(|body| body["text"].as_str().unwrap_or_default().len() <= MAX_MESSAGE_LENGTH),
            "overflow path should keep edit payloads within Telegram max length"
        );
    }

    #[tokio::test]
    async fn test_e2e_streaming_input_stop_before_second_tool_call() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(StreamingInterruptAwareProvider::new());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let first = TgUpdateBuilder::dm(
            42,
            "我们来测试流式输入 你执行 10 次pwd 每次sleep  10s，一次调用一次 bash，串行。",
        )
        .with_message_id(9301)
        .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &first,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let second = TgUpdateBuilder::dm(42, "停止")
            .with_message_id(9302)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &second,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        // First run starts after debounce, then sleeps 10s between tool calls.
        // Waiting here ensures the run has enough time to observe queued input.
        tokio::time::sleep(std::time::Duration::from_secs(13)).await;

        assert_eq!(
            provider.run_count.load(Ordering::Relaxed),
            1,
            "second message should be queued into active stream, not start a new run"
        );
        assert_eq!(
            provider.pwd_call_count.load(Ordering::Relaxed),
            1,
            "stop message should be visible before second pwd call"
        );

        let send_reqs = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("sendMessage"),
        )
        .await;
        assert!(!send_reqs.is_empty());
    }

    #[tokio::test]
    async fn test_e2e_telegram_group_mention() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router_with_store(Arc::new(InMemoryThreadStore::new()));
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::group(42, -100123, "how are you")
            .with_mention("garyx")
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while provider.call_count.load(Ordering::Relaxed) < 1 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("media group dispatch should happen within timeout");
        let send_reqs = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| {
                if !r.url.path().contains("sendMessage") {
                    return false;
                }
                serde_json::from_slice::<serde_json::Value>(&r.body)
                    .ok()
                    .and_then(|body| body["chat_id"].as_i64())
                    == Some(-100123)
            },
        )
        .await;

        // Provider should be called with mention stripped
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].thread_id.starts_with("thread::"));
        assert!(
            !calls[0].message.contains("@garyx"),
            "mention should be stripped from message"
        );
        assert!(calls[0].message.contains("how are you"));

        // Verify sendMessage was called with reply_to_message_id
        let body: serde_json::Value = serde_json::from_slice(&send_reqs[0].body).unwrap();
        assert_eq!(body["chat_id"], -100123);
        assert!(body["reply_to_message_id"].is_number());
    }

    #[tokio::test]
    async fn test_e2e_telegram_reply_routing() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        // Step 1: Send first message to establish a thread
        let update1 = TgUpdateBuilder::dm(42, "first message")
            .with_message_id(1001)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update1,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("sendMessage"),
        )
        .await;

        let initial_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };
        let thread_from_reply = {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let resolved = {
                    let r = router.lock().await;
                    r.resolve_reply_thread_for_chat("telegram", "bot1", Some("42"), None, "999")
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

        // Step 2: Create a reply message that references the bot's response (msg_id=999)
        let bot_reply_msg = TgMessage {
            message_id: 999,
            chat: TgChat {
                id: 42,
                chat_type: "private".to_string(),
                title: None,
                is_forum: None,
            },
            from: Some(TgUser {
                id: 999,
                is_bot: true,
                first_name: "Gary".to_string(),
                last_name: None,
                username: Some("garyx".to_string()),
            }),
            text: Some("echo: first message".to_string()),
            caption: None,
            date: 1700000000,
            message_thread_id: None,
            media_group_id: None,
            reply_to_message: None,
            entities: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        };

        let update2 = TgUpdateBuilder::dm(42, "follow up")
            .with_message_id(1002)
            .with_reply_to(bot_reply_msg)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update2,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 2).await;

        // Both messages should have routed to the same thread
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, calls[1].thread_id);
        assert!(calls[0].thread_id.starts_with("thread::"));
    }

    #[tokio::test]
    async fn test_e2e_telegram_reply_routing_after_endpoint_rebind_keeps_old_thread_without_switching_current()
     {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let http = reqwest::Client::new();

        let update1 = TgUpdateBuilder::dm(42, "first message")
            .with_message_id(3301)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update1,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;
        let _send_reqs = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("sendMessage"),
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
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                binding_key: "42".to_owned(),
                chat_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                display_label: "42".to_owned(),
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

        let bot_reply_msg = TgMessage {
            message_id: 999,
            chat: TgChat {
                id: 42,
                chat_type: "private".to_string(),
                title: None,
                is_forum: None,
            },
            from: Some(TgUser {
                id: 999,
                is_bot: true,
                first_name: "Gary".to_string(),
                last_name: None,
                username: Some("garyx".to_string()),
            }),
            text: Some("echo: first message".to_string()),
            caption: None,
            date: 1700000000,
            message_thread_id: None,
            media_group_id: None,
            reply_to_message: None,
            entities: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        };

        let update2 = TgUpdateBuilder::dm(42, "follow old thread")
            .with_message_id(3302)
            .with_reply_to(bot_reply_msg)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update2,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;
        let _send_reqs = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("sendMessage"),
        )
        .await;
        wait_for_provider_calls(provider.as_ref(), 2).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[1].thread_id, old_thread);
        drop(calls);

        let current = {
            let router_guard = router.lock().await;
            router_guard
                .get_current_thread_id_for_binding("telegram", "bot1", "42")
                .map(str::to_owned)
        };
        assert_eq!(current, None);
        let rebound = {
            let mut router_guard = router.lock().await;
            router_guard
                .resolve_endpoint_thread_id("telegram", "bot1", "42")
                .await
        };
        assert_eq!(rebound.as_deref(), Some(new_thread.as_str()));
    }

    #[tokio::test]
    async fn test_e2e_telegram_reply_routing_after_router_restart() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let http = reqwest::Client::new();

        let update1 = TgUpdateBuilder::dm(42, "first message")
            .with_message_id(3001)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update1,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;
        let _send_reqs = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("sendMessage"),
        )
        .await;
        wait_for_provider_calls(provider.as_ref(), 1).await;

        // Simulate router restart: rebuild reply index and delivery cache from persisted store.
        let restarted_router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        {
            let rebuild_count = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let count = {
                        let mut guard = restarted_router.lock().await;
                        guard.rebuild_routing_index("telegram").await
                    };
                    if count == 1 {
                        break count;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("reply routing should persist before router restart rebuild");
            assert_eq!(rebuild_count, 1);

            let delivery_count = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let count = {
                        let mut guard = restarted_router.lock().await;
                        guard.rebuild_last_delivery_cache().await
                    };
                    if count >= 1 {
                        break count;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("last delivery should persist before router restart rebuild");
            assert!(delivery_count >= 1);
        }

        let bot_reply_msg = TgMessage {
            message_id: 999,
            chat: TgChat {
                id: 42,
                chat_type: "private".to_string(),
                title: None,
                is_forum: None,
            },
            from: Some(TgUser {
                id: 999,
                is_bot: true,
                first_name: "Gary".to_string(),
                last_name: None,
                username: Some("garyx".to_string()),
            }),
            text: Some("echo: first message".to_string()),
            caption: None,
            date: 1700000000,
            message_thread_id: None,
            media_group_id: None,
            reply_to_message: None,
            entities: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        };

        let update2 = TgUpdateBuilder::dm(42, "follow up after restart")
            .with_message_id(3002)
            .with_reply_to(bot_reply_msg)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update2,
            &restarted_router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;
        let _send_reqs = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("sendMessage"),
        )
        .await;
        wait_for_provider_calls(provider.as_ref(), 2).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].thread_id.starts_with("thread::"));
        assert_eq!(calls[0].thread_id, calls[1].thread_id);
    }

    #[tokio::test]
    async fn test_e2e_telegram_reply_routing_falls_back_when_old_thread_is_gone() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let http = reqwest::Client::new();

        let update1 = TgUpdateBuilder::dm(42, "first message")
            .with_message_id(3201)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update1,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;
        wait_for_provider_calls(provider.as_ref(), 1).await;

        let old_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };
        wait_for_thread_delivery_persistence(&store, &old_thread).await;

        let (new_thread, _) = create_thread_record(
            &store,
            ThreadEnsureOptions {
                label: Some("Fallback".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("thread should be created");
        bind_endpoint_to_thread(
            &store,
            &new_thread,
            ChannelBinding {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                binding_key: "42".to_owned(),
                chat_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                display_label: "42".to_owned(),
                last_inbound_at: None,
                last_delivery_at: None,
            },
        )
        .await
        .expect("bind should succeed");
        assert!(
            store.delete(&old_thread).await,
            "old thread should be deleted"
        );
        {
            let mut router_guard = router.lock().await;
            router_guard.rebuild_thread_indexes().await;
        }

        let bot_reply_msg = TgMessage {
            message_id: 999,
            chat: TgChat {
                id: 42,
                chat_type: "private".to_string(),
                title: None,
                is_forum: None,
            },
            from: Some(TgUser {
                id: 999,
                is_bot: true,
                first_name: "Gary".to_string(),
                last_name: None,
                username: Some("garyx".to_string()),
            }),
            text: Some("echo: first message".to_string()),
            caption: None,
            date: 1700000000,
            message_thread_id: None,
            media_group_id: None,
            reply_to_message: None,
            entities: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        };

        let update2 = TgUpdateBuilder::dm(42, "follow missing thread")
            .with_message_id(3202)
            .with_reply_to(bot_reply_msg)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update2,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;
        wait_for_provider_calls(provider.as_ref(), 2).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[1].thread_id, new_thread);
        assert_ne!(calls[1].thread_id, old_thread);
    }

    #[tokio::test]
    async fn test_e2e_telegram_reply_routing_switches_scheduled_thread() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        {
            let mut router_guard = router.lock().await;
            router_guard
                .ensure_thread_entry(
                    "cron::daily_summary",
                    "telegram",
                    "bot1",
                    "42",
                    Some("cron::daily_summary"),
                )
                .await;
            router_guard.record_outbound_message("cron::daily_summary", "telegram", "bot1", "999");
        }

        let bot_reply_msg = TgMessage {
            message_id: 999,
            chat: TgChat {
                id: 42,
                chat_type: "private".to_string(),
                title: None,
                is_forum: None,
            },
            from: Some(TgUser {
                id: 999,
                is_bot: true,
                first_name: "Gary".to_string(),
                last_name: None,
                username: Some("garyx".to_string()),
            }),
            text: Some("#cron::daily_summary\nscheduled ping".to_string()),
            caption: None,
            date: 1700000000,
            message_thread_id: None,
            media_group_id: None,
            reply_to_message: None,
            entities: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        };

        let update = TgUpdateBuilder::dm(42, "follow scheduled context")
            .with_reply_to(bot_reply_msg)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        let send_reqs = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("sendMessage"),
        )
        .await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, "cron::daily_summary");
        drop(calls);

        let switched_thread = {
            let router_guard = router.lock().await;
            router_guard
                .get_current_thread_id_for_binding("telegram", "bot1", "42")
                .map(|s| s.to_owned())
        };
        assert_eq!(switched_thread.as_deref(), Some("cron::daily_summary"));

        assert!(!send_reqs.is_empty());
    }

    #[tokio::test]
    async fn test_e2e_telegram_group_reply_routing_scheduled_switch_scoped_to_group() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        {
            let mut router_guard = router.lock().await;
            router_guard
                .ensure_thread_entry(
                    "cron::daily_summary",
                    "telegram",
                    "bot1",
                    "42",
                    Some("cron::daily_summary"),
                )
                .await;
            router_guard.record_outbound_message("cron::daily_summary", "telegram", "bot1", "999");
        }

        let bot_reply_msg = TgMessage {
            message_id: 999,
            chat: TgChat {
                id: -100123,
                chat_type: "supergroup".to_string(),
                title: Some("test".to_string()),
                is_forum: Some(false),
            },
            from: Some(TgUser {
                id: 999,
                is_bot: true,
                first_name: "Gary".to_string(),
                last_name: None,
                username: Some("garyx".to_string()),
            }),
            text: Some("#cron::daily_summary\nscheduled ping".to_string()),
            caption: None,
            date: 1700000000,
            message_thread_id: None,
            media_group_id: None,
            reply_to_message: None,
            entities: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        };

        let update = TgUpdateBuilder::group(42, -100123, "group follow scheduled context")
            .with_message_id(1101)
            .with_mention("garyx")
            .with_reply_to(bot_reply_msg)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        wait_for_request_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, "cron::daily_summary");
        drop(calls);

        let (group_switched, dm_switched) = {
            let router_guard = router.lock().await;
            let group_switched = router_guard
                .get_current_thread_id_for_binding("telegram", "bot1", "-100123")
                .map(|s| s.to_owned());
            let dm_switched = router_guard
                .get_current_thread_id_for_binding("telegram", "bot1", "42")
                .map(|s| s.to_owned());
            (group_switched, dm_switched)
        };

        assert_eq!(group_switched.as_deref(), Some("cron::daily_summary"));
        assert_eq!(dm_switched, None);

        let send_reqs = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("sendMessage"),
        )
        .await;
        assert!(!send_reqs.is_empty());
    }

    #[tokio::test]
    async fn test_e2e_telegram_reply_routing_switches_cron_session() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        {
            let mut router_guard = router.lock().await;
            router_guard
                .ensure_thread_entry(
                    "cron::daily::42",
                    "telegram",
                    "bot1",
                    "42",
                    Some("cron::daily::42"),
                )
                .await;
            router_guard.record_outbound_message("cron::daily::42", "telegram", "bot1", "888");
        }

        let bot_reply_msg = TgMessage {
            message_id: 888,
            chat: TgChat {
                id: 42,
                chat_type: "private".to_string(),
                title: None,
                is_forum: None,
            },
            from: Some(TgUser {
                id: 999,
                is_bot: true,
                first_name: "Gary".to_string(),
                last_name: None,
                username: Some("garyx".to_string()),
            }),
            text: Some("#cron::daily::42\nscheduled ping".to_string()),
            caption: None,
            date: 1700000000,
            message_thread_id: None,
            media_group_id: None,
            reply_to_message: None,
            entities: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        };

        let update = TgUpdateBuilder::dm(42, "follow scheduled context")
            .with_reply_to(bot_reply_msg)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, "cron::daily::42");
        drop(calls);

        let switched_thread = {
            let router_guard = router.lock().await;
            router_guard
                .get_current_thread_id_for_binding("telegram", "bot1", "42")
                .map(|s| s.to_owned())
        };
        assert_eq!(switched_thread.as_deref(), Some("cron::daily::42"));

        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;
        assert!(
            !send_bodies.is_empty(),
            "cron reply routing should emit a Telegram reply"
        );
    }

    #[tokio::test]
    async fn test_e2e_telegram_multi_account_switched_thread_isolated() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let http = reqwest::Client::new();

        let seeded_bot1_thread = seed_bound_dm_thread(&store, "bot1", "42", "custom").await;
        {
            let mut router_guard = router.lock().await;
            router_guard.rebuild_thread_indexes().await;
        }

        let update_bot2 = TgUpdateBuilder::dm(42, "hello from bot2").build();
        dispatch_update(
            &http,
            "bot2",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update_bot2,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        let update_bot1 = TgUpdateBuilder::dm(42, "hello from bot1").build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update_bot1,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            2,
        )
        .await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 2);
        let reply_texts: Vec<&str> = send_bodies
            .iter()
            .filter_map(|body| body["text"].as_str())
            .filter(|text| {
                text.contains("echo: hello from bot1") || text.contains("echo: hello from bot2")
            })
            .collect();
        assert_eq!(
            reply_texts.len(),
            2,
            "each account dispatch should emit one reply"
        );
        let calls = provider.calls.lock().unwrap();
        let bot1_session = calls
            .iter()
            .find(|call| call.message == "hello from bot1")
            .map(|call| call.thread_id.clone())
            .expect("bot1 dispatch should exist");
        let bot2_session = calls
            .iter()
            .find(|call| call.message == "hello from bot2")
            .map(|call| call.thread_id.clone())
            .expect("bot2 dispatch should exist");
        assert_eq!(bot1_session, seeded_bot1_thread);
        assert!(bot2_session.starts_with("thread::"));
        assert_ne!(bot2_session, seeded_bot1_thread);
    }

    #[tokio::test]
    async fn test_e2e_telegram_session_persistence() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> =
            Arc::new(garyx_router::InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::dm(42, "persist me").build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;

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
        assert_eq!(messages[0]["content"], "persist me");
        assert_eq!(messages[1]["role"], "assistant");
        assert!(messages[1]["content"].as_str().unwrap().contains("echo:"));
        assert!(thread_id.starts_with("thread::"));
    }

    #[tokio::test]
    async fn test_e2e_telegram_command_new_switches_to_named_session() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::dm(42, "/newthread").build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_json_capture_len(&capture.send_messages, 1, std::time::Duration::from_secs(5))
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let send_bodies = capture.send_bodies();

        // Provider should NOT be called (it's a command)
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 0);

        // Verify command response includes the created thread label
        let text = send_bodies
            .iter()
            .find_map(|body| {
                let text = body["text"].as_str().unwrap_or_default();
                text.starts_with("Created and switched to new thread: thread-")
                    .then_some(text)
            })
            .unwrap_or_default();
        assert!(text.starts_with("Created and switched to new thread: thread-"));

        let switched = {
            let router_guard = router.lock().await;
            router_guard
                .get_current_thread_id_for_binding("telegram", "bot1", "42")
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
    }

    #[tokio::test]
    async fn test_e2e_telegram_bind_detach_retargets_next_dm() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = make_router_with_store(store.clone());
        let http = reqwest::Client::new();

        let first = TgUpdateBuilder::dm(42, "first bound thread")
            .with_message_id(4001)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &first,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;
        wait_for_json_capture_len(&capture.send_messages, 1, std::time::Duration::from_secs(5))
            .await;

        let first_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };
        assert!(first_thread.starts_with("thread::"));

        let detached = detach_endpoint_from_thread(&store, "telegram::bot1::42")
            .await
            .expect("detach should succeed");
        assert_eq!(detached.as_deref(), Some(first_thread.as_str()));
        {
            let mut router_guard = router.lock().await;
            router_guard.rebuild_thread_indexes().await;
        }

        let second = TgUpdateBuilder::dm(42, "second rebound thread")
            .with_message_id(4002)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &second,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 2).await;
        wait_for_json_capture_len(&capture.send_messages, 2, std::time::Duration::from_secs(5))
            .await;

        let second_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[1].thread_id.clone()
        };
        assert!(second_thread.starts_with("thread::"));
        assert_ne!(second_thread, first_thread);

        let bot_reply_msg = TgMessage {
            message_id: 1000,
            chat: TgChat {
                id: 42,
                chat_type: "private".to_string(),
                title: None,
                is_forum: None,
            },
            from: Some(TgUser {
                id: 999,
                is_bot: true,
                first_name: "Gary".to_string(),
                last_name: None,
                username: Some("garyx".to_string()),
            }),
            text: Some("echo: first bound thread".to_string()),
            caption: None,
            date: 1700000000,
            message_thread_id: None,
            media_group_id: None,
            reply_to_message: None,
            entities: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        };

        let detached_reply = TgUpdateBuilder::dm(42, "reply after detach should not stick")
            .with_message_id(4004)
            .with_reply_to(bot_reply_msg)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &detached_reply,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 3).await;
        wait_for_json_capture_len(&capture.send_messages, 3, std::time::Duration::from_secs(5))
            .await;

        let detached_reply_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[2].thread_id.clone()
        };
        assert_eq!(detached_reply_thread, first_thread);
        let rebound_after_detached_reply = {
            let mut router_guard = router.lock().await;
            router_guard
                .resolve_endpoint_thread_id("telegram", "bot1", "42")
                .await
        };
        assert_eq!(
            rebound_after_detached_reply.as_deref(),
            Some(second_thread.as_str())
        );

        bind_endpoint_to_thread(
            &store,
            &first_thread,
            ChannelBinding {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                binding_key: "42".to_owned(),
                chat_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                display_label: "42".to_owned(),
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

        let third = TgUpdateBuilder::dm(42, "third back to first")
            .with_message_id(4003)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &third,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 4).await;
        wait_for_json_capture_len(&capture.send_messages, 4, std::time::Duration::from_secs(5))
            .await;

        let third_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[3].thread_id.clone()
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
    async fn test_e2e_telegram_detach_clears_explicit_session_override() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let http = reqwest::Client::new();

        let first = TgUpdateBuilder::dm(42, "first bound thread")
            .with_message_id(4011)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &first,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;

        let first_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[0].thread_id.clone()
        };

        {
            let mut router_guard = router.lock().await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "42");
            router_guard.switch_to_thread(&user_key, &first_thread);
        }

        detach_endpoint_from_thread(&store, "telegram::bot1::42")
            .await
            .expect("detach should succeed");
        {
            let mut router_guard = router.lock().await;
            router_guard.rebuild_thread_indexes().await;
        }

        let second = TgUpdateBuilder::dm(42, "after detach should not stick")
            .with_message_id(4012)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &second,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 2).await;

        let second_thread = {
            let calls = provider.calls.lock().unwrap();
            calls[1].thread_id.clone()
        };
        assert_ne!(second_thread, first_thread);
    }

    #[tokio::test]
    async fn test_e2e_telegram_sessions_lists_named_sessions_with_current_marker() {
        let (server, capture) = setup_tg_capture_mock(false).await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;
        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let http = reqwest::Client::new();

        {
            let mut router_guard = router.lock().await;
            router_guard
                .ensure_thread_entry(
                    "bot1::main::42:thread-a",
                    "telegram",
                    "bot1",
                    "42",
                    Some("thread-a"),
                )
                .await;
            router_guard
                .ensure_thread_entry(
                    "bot1::main::42:thread-b",
                    "telegram",
                    "bot1",
                    "42",
                    Some("thread-b"),
                )
                .await;
            let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "42");
            router_guard.switch_to_thread(&user_key, "bot1::main::42:thread-b");
        }

        let update = TgUpdateBuilder::dm(42, "/threads").build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        let send_bodies = wait_for_json_capture_quiet_window(
            &capture.send_messages,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 0);
        let text = send_bodies
            .iter()
            .find_map(|body| {
                let text = body["text"].as_str().unwrap_or_default();
                text.contains("Your Threads:").then_some(text)
            })
            .unwrap_or_default();
        assert!(text.contains("Your Threads:"));
        assert!(text.contains("thread-a"));
        assert!(text.contains("thread-b ⬅️"));
        assert!(text.contains("Use /newthread to create a thread."));
    }

    #[tokio::test]
    async fn test_e2e_telegram_sessionprev_rebuilds_history_after_restart() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let store: Arc<dyn garyx_router::ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let bridge = make_bridge_with_store(provider.clone(), store.clone()).await;

        store
            .set(
                "bot1::main::42_a",
                serde_json::json!({"from_id": "42", "updated_at": "2026-03-01T10:00:00Z"}),
            )
            .await;
        store
            .set(
                "bot1::main::42_b",
                serde_json::json!({"from_id": "42", "updated_at": "2026-03-01T11:00:00Z"}),
            )
            .await;
        store
            .set(
                "bot1::main::42_c",
                serde_json::json!({"from_id": "42", "updated_at": "2026-03-01T12:00:00Z"}),
            )
            .await;

        let router = Arc::new(Mutex::new(MessageRouter::new(
            store.clone(),
            GaryxConfig::default(),
        )));
        let http = reqwest::Client::new();

        let command = TgUpdateBuilder::dm(42, "/threadprev")
            .with_message_id(3101)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &command,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        let command_reqs = wait_for_request_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
        )
        .await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 0);
        let switched_notice = command_reqs.iter().find(|r| {
            r.url.path().contains("sendMessage")
                && std::str::from_utf8(&r.body)
                    .map(|body| body.contains("Switched to previous thread: bot1::main::42_b"))
                    .unwrap_or(false)
        });
        assert!(
            switched_notice.is_some(),
            "command reply should report rebuilt previous thread"
        );

        let normal = TgUpdateBuilder::dm(42, "after restart command")
            .with_message_id(3102)
            .build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &normal,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        wait_for_counter_at_least(&provider.call_count, 1).await;

        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].thread_id, "bot1::main::42_b");
    }

    #[tokio::test]
    async fn test_e2e_telegram_long_message_split() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        // Provider returns a response longer than 4096 chars
        let provider = Arc::new(ConfigurableTestProvider::with_response(|_| {
            "x".repeat(5000)
        }));
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::dm(42, "give me a long response").build();
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        let send_reqs = wait_for_matching_requests_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            1,
            |r| r.url.path().contains("sendMessage"),
        )
        .await;

        let send_msg_count = send_reqs.iter().count();
        assert!(
            send_msg_count >= 1,
            "long message should produce at least one sendMessage call"
        );
        for request in send_reqs {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("valid sendMessage body");
            let text = body["text"].as_str().unwrap_or_default();
            assert!(
                text.len() <= MAX_MESSAGE_LENGTH,
                "outbound message must respect Telegram max length"
            );
        }
    }

    #[tokio::test]
    async fn test_e2e_text_messages_are_debounced_and_merged() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update1 = TgUpdateBuilder::dm(42, "line one")
            .with_message_id(9101)
            .build();
        let update2 = TgUpdateBuilder::dm(42, "line two")
            .with_message_id(9102)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update1,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update2,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while provider.call_count.load(Ordering::Relaxed) < 1 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("media group dispatch should happen within timeout");

        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            1,
            "debounced text fragments should dispatch once"
        );
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].message, "line one\nline two");
    }

    #[tokio::test]
    async fn test_e2e_media_message_flushes_pending_debounce_text() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let text_update = TgUpdateBuilder::dm(42, "before photo")
            .with_message_id(9201)
            .build();
        let photo_update = TgUpdateBuilder::photo(42, 42, None)
            .with_private_chat()
            .with_message_id(9202)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &text_update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &photo_update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(800)).await;

        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            2,
            "pending debounce text should flush before media dispatch"
        );
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].message, "before photo");
        assert_eq!(calls[0].image_count, 0);
        assert_eq!(calls[1].image_count, 1);
    }

    #[tokio::test]
    async fn test_e2e_duplicate_text_message_is_ignored() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::dm(42, "hello dedup")
            .with_message_id(9001)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;

        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            1,
            "duplicate message should be ignored"
        );
    }

    #[tokio::test]
    async fn test_send_response_retries_transient_error() {
        let server = MockServer::start().await;
        let token = "retry-token-test";

        Mock::given(method("POST"))
            .and(path_regex(r"/bot.+/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "description": "Too Many Requests"
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let result = send_response(
            TelegramSendTarget::new(&http, token, 42, None, &server.uri()),
            "hello retry",
            None,
        )
        .await;

        assert!(result.is_err(), "transient error should fail after retries");
        let requests = server.received_requests().await.unwrap();
        let send_count = requests
            .iter()
            .filter(|r| r.url.path() == format!("/bot{token}/sendMessage"))
            .count();
        assert_eq!(send_count, OUTBOUND_MAX_RETRIES);
    }

    #[tokio::test]
    async fn test_send_response_falls_back_when_reply_target_is_missing() {
        let server = MockServer::start().await;
        let token = "reply-fallback-token-test";

        Mock::given(method("POST"))
            .and(path_regex(r"/bot.+/sendMessage"))
            .respond_with(|req: &wiremock::Request| {
                let body: serde_json::Value =
                    serde_json::from_slice(&req.body).expect("valid sendMessage body");
                let has_reply_to = body.get("reply_to_message_id").is_some();
                if has_reply_to {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "ok": false,
                        "description": "Bad Request: message to be replied not found"
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "ok": true,
                        "result": {
                            "message_id": 999,
                            "chat": {"id": 42, "type": "private"},
                            "date": 1700000000,
                            "text": "fallback ok"
                        }
                    }))
                }
            })
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let result = send_response(
            TelegramSendTarget::new(&http, token, 42, None, &server.uri()),
            "hello fallback",
            Some(100),
        )
        .await;

        assert!(result.is_ok(), "missing reply target should fall back");
        let requests = server.received_requests().await.unwrap();
        let send_requests: Vec<_> = requests
            .iter()
            .filter(|r| r.url.path() == format!("/bot{token}/sendMessage"))
            .collect();
        assert_eq!(send_requests.len(), 2, "should retry once without reply_to");

        let first_body: serde_json::Value =
            serde_json::from_slice(&send_requests[0].body).expect("first body json");
        assert_eq!(first_body["reply_to_message_id"], 100);

        let second_body: serde_json::Value =
            serde_json::from_slice(&send_requests[1].body).expect("second body json");
        assert!(second_body.get("reply_to_message_id").is_none());
    }

    // ---------------------------------------------------------------
    // Policy enforcement E2E tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_e2e_group_disabled_skips_message() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router_with_store(Arc::new(InMemoryThreadStore::new()));
        let http = reqwest::Client::new();

        let mut account = default_account();
        let mut group_config = garyx_models::config::TelegramGroupConfig::default();
        group_config.enabled = false;
        account.groups.insert("-100123".into(), group_config);

        let update = TgUpdateBuilder::photo(42, -100123, Some("blocked photo"))
            .with_message_id(9201)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            0,
            "provider should NOT be called for disabled group"
        );
        let events = {
            let router = router.lock().await;
            router
                .list_message_ledger_events_for_thread("-100123", 20)
                .await
        };
        let blocked = events.iter().find(|event| {
            event.text_excerpt.as_deref() == Some("blocked photo")
                && event.status == garyx_models::MessageLifecycleStatus::Filtered
                && event.terminal_reason
                    == Some(garyx_models::MessageTerminalReason::PolicyFiltered)
                && event
                    .metadata
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("group_disabled")
        });
        assert!(
            blocked.is_some(),
            "disabled group should produce a filtered policy ledger event: {events:#?}"
        );
    }

    #[tokio::test]
    async fn test_e2e_group_allow_from_blocks_unauthorized() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router_with_store(Arc::new(InMemoryThreadStore::new()));
        let http = reqwest::Client::new();

        let mut account = default_account();
        let mut group_config = garyx_models::config::TelegramGroupConfig::default();
        group_config.allow_from = Some(vec!["99".into()]); // only user 99 allowed
        account.groups.insert("-100123".into(), group_config);

        let update = TgUpdateBuilder::photo(42, -100123, Some("blocked allowlist"))
            .with_message_id(9202)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            0,
            "provider should NOT be called for unauthorized user"
        );
        let events = {
            let router = router.lock().await;
            router
                .list_message_ledger_events_for_thread("-100123", 20)
                .await
        };
        let blocked = events.iter().find(|event| {
            event.text_excerpt.as_deref() == Some("blocked allowlist")
                && event.status == garyx_models::MessageLifecycleStatus::Filtered
                && event.terminal_reason
                    == Some(garyx_models::MessageTerminalReason::PolicyFiltered)
                && event
                    .metadata
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("group_allow_from")
        });
        assert!(
            blocked.is_some(),
            "unauthorized group sender should produce allow_from filtered event: {events:#?}"
        );
    }

    #[tokio::test]
    async fn test_e2e_group_allow_from_permits_authorized() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let mut account = default_account();
        let mut group_config = garyx_models::config::TelegramGroupConfig::default();
        group_config.allow_from = Some(vec!["42".into()]); // user 42 is allowed
        account.groups.insert("-100123".into(), group_config);

        let update = TgUpdateBuilder::group(42, -100123, "hello")
            .with_mention("garyx")
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            1,
            "provider should be called for authorized user"
        );
    }

    #[tokio::test]
    async fn test_e2e_topic_disabled_skips_message() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let mut account = default_account();
        let mut group_config = garyx_models::config::TelegramGroupConfig::default();
        let mut topic_config = garyx_models::config::TelegramTopicConfig::default();
        topic_config.enabled = false;
        group_config.topics.insert("555".into(), topic_config);
        account.groups.insert("-100123".into(), group_config);

        let update = TgUpdateBuilder::group(42, -100123, "hello")
            .with_mention("garyx")
            .with_forum()
            .with_thread_id(555)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            0,
            "provider should NOT be called for disabled topic"
        );
    }

    #[tokio::test]
    async fn test_e2e_topic_require_mention_override() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let mut account = default_account();
        let mut group_config = garyx_models::config::TelegramGroupConfig::default();
        group_config.require_mention = true; // group requires mention
        let topic_config = garyx_models::config::TelegramTopicConfig {
            enabled: true,
            require_mention: Some(false), // topic overrides: no mention needed
            allow_from: None,
            system_prompt: None,
        };
        group_config.topics.insert("555".into(), topic_config);
        account.groups.insert("-100123".into(), group_config);

        // Send message WITHOUT mention - should still work because topic overrides
        let update = TgUpdateBuilder::group(42, -100123, "hello without mention")
            .with_forum()
            .with_thread_id(555)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            1,
            "provider should be called when topic overrides require_mention to false"
        );
    }

    #[tokio::test]
    async fn test_e2e_topic_allow_from_overrides_group() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let mut account = default_account();
        let mut group_config = garyx_models::config::TelegramGroupConfig::default();
        group_config.require_mention = false;
        group_config.allow_from = Some(vec!["42".into()]); // group allows user 42
        let topic_config = garyx_models::config::TelegramTopicConfig {
            enabled: true,
            allow_from: Some(vec!["99".into()]), // topic only allows user 99
            require_mention: Some(false),
            system_prompt: None,
        };
        group_config.topics.insert("555".into(), topic_config);
        account.groups.insert("-100123".into(), group_config);

        // User 42 in topic 555 - should be blocked because topic allowlist overrides
        let update = TgUpdateBuilder::group(42, -100123, "hello")
            .with_forum()
            .with_thread_id(555)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            0,
            "topic allow_from should override group allow_from"
        );
    }

    #[tokio::test]
    async fn test_e2e_topic_system_prompt_metadata_override() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let mut account = default_account();
        let mut group_config = garyx_models::config::TelegramGroupConfig::default();
        group_config.require_mention = false;
        group_config.system_prompt = Some("group prompt".into());
        let topic_config = garyx_models::config::TelegramTopicConfig {
            enabled: true,
            allow_from: None,
            require_mention: Some(false),
            system_prompt: Some("topic prompt".into()),
        };
        group_config.topics.insert("555".into(), topic_config);
        account.groups.insert("-100123".into(), group_config);

        let update = TgUpdateBuilder::group(42, -100123, "hello")
            .with_forum()
            .with_thread_id(555)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &account,
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        let metadata_prompt = calls[0]
            .metadata
            .get("system_prompt")
            .and_then(|v| v.as_str());
        assert_eq!(metadata_prompt, Some("topic prompt"));
    }

    #[tokio::test]
    async fn test_e2e_non_forum_thread_id_routes_to_chat_session() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::group(42, -100123, "hello")
            .with_mention("garyx")
            .with_thread_id(555)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].thread_id.starts_with("thread::"));
    }

    #[tokio::test]
    async fn test_e2e_forum_topic_session_key_uses_chat_topic_composite() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::group(42, -100123, "hello")
            .with_mention("garyx")
            .with_forum()
            .with_thread_id(555)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].thread_id.starts_with("thread::"));
    }

    #[tokio::test]
    async fn test_e2e_forum_general_topic_send_omits_thread_but_typing_keeps_it() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::group(42, -100123, "hello")
            .with_mention("garyx")
            .with_forum()
            .with_thread_id(1)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        wait_for_counter_at_least(&provider.call_count, 1).await;
        let requests = wait_for_request_quiet_window(
            &server,
            std::time::Duration::from_millis(200),
            std::time::Duration::from_secs(5),
            2,
        )
        .await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].thread_id.starts_with("thread::"));
        drop(calls);

        let typing_req = requests
            .iter()
            .find(|req| req.url.path().contains("sendChatAction"))
            .expect("expected sendChatAction request");
        let typing_body: serde_json::Value = serde_json::from_slice(&typing_req.body).unwrap();
        assert_eq!(typing_body["message_thread_id"], 1);

        let send_req = requests
            .iter()
            .find(|req| req.url.path().contains("sendMessage"))
            .expect("expected sendMessage request");
        let send_body: serde_json::Value = serde_json::from_slice(&send_req.body).unwrap();
        assert!(
            send_body.get("message_thread_id").is_none()
                || send_body["message_thread_id"].is_null(),
            "general topic sendMessage should omit message_thread_id, got: {send_body}"
        );
    }

    // ---------------------------------------------------------------
    // Media routing E2E tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_e2e_photo_message_routed() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::photo(42, 42, Some("my cat"))
            .with_private_chat()
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].message, "my cat");
        assert_eq!(calls[0].image_count, 1);
        assert_eq!(
            calls[0]
                .metadata
                .get("image_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            1
        );
    }

    #[tokio::test]
    async fn test_e2e_photo_message_without_caption_uses_default_prompt() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::photo(42, 42, None)
            .with_private_chat()
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].message, "请描述这张图片。");
        assert_eq!(calls[0].image_count, 1);
    }

    #[tokio::test]
    async fn test_e2e_media_group_is_buffered_and_dispatched_once() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update1 = TgUpdateBuilder::photo(42, 42, Some("album caption"))
            .with_private_chat()
            .with_media_group_id("album-1")
            .with_message_id(201)
            .build();
        let update2 = TgUpdateBuilder::photo(42, 42, None)
            .with_private_chat()
            .with_media_group_id("album-1")
            .with_message_id(202)
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update1,
            &router,
            &bridge,
            &api_base,
        )
        .await;
        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update2,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while provider.call_count.load(Ordering::Relaxed) < 1 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("media group dispatch should happen within timeout");

        assert_eq!(
            provider.call_count.load(Ordering::Relaxed),
            1,
            "media group should dispatch exactly once"
        );
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].image_count, 2, "media group should merge images");
        assert!(
            calls[0].message.contains("用户发送了 2 张图片"),
            "media group message summary missing: {}",
            calls[0].message
        );
        assert!(
            calls[0].message.contains("album caption"),
            "media group caption should be preserved"
        );
    }

    #[tokio::test]
    async fn test_e2e_voice_message_routed() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::voice(42, 42).with_private_chat().build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].message.contains("voice message"));
    }

    #[tokio::test]
    async fn test_e2e_document_message_routed() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::document(42, 42, "report.pdf")
            .with_private_chat()
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls[0].attachments.len(), 1);
        assert!(calls[0].attachments[0].name.ends_with("report.pdf"));
        assert!(
            calls[0].attachments[0]
                .path
                .contains("garyx-telegram/inbound/")
        );
    }

    #[tokio::test]
    async fn test_e2e_sticker_message_routed() {
        let server = setup_tg_mock().await;
        let api_base = unique_api_base(&server);
        let provider = Arc::new(ConfigurableTestProvider::echo());
        let bridge = make_bridge_with(provider.clone()).await;
        let router = make_router();
        let http = reqwest::Client::new();

        let update = TgUpdateBuilder::sticker(42, 42, "\u{1f600}")
            .with_private_chat()
            .build();

        dispatch_update(
            &http,
            "bot1",
            "fake-token",
            "garyx",
            999,
            &default_account(),
            &update,
            &router,
            &bridge,
            &api_base,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(provider.call_count.load(Ordering::Relaxed), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].message.contains("sticker"));
    }
}
