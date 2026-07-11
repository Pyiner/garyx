use super::*;
use crate::threads::is_thread_key;
use crate::{
    ChannelBinding, ThreadCreator, ThreadEnsureOptions, ThreadStore, WorkspaceMode,
    bindings_from_value, create_thread_record,
};

struct FallbackOnlyThreadCreator;

#[async_trait]
impl ThreadCreator for FallbackOnlyThreadCreator {
    async fn create_thread(
        &self,
        thread_store: Arc<dyn ThreadStore>,
        options: ThreadEnsureOptions,
    ) -> Result<(String, Value), String> {
        match options.agent_id.as_deref() {
            Some("claude") => create_thread_record(&thread_store, options).await,
            Some(agent_id) => Err(format!("unknown agent_id: {agent_id}")),
            None => Err("agent_id is required".to_owned()),
        }
    }
}

struct CapturingThreadCreator {
    options: tokio::sync::Mutex<Vec<ThreadEnsureOptions>>,
}

impl CapturingThreadCreator {
    fn new() -> Self {
        Self {
            options: tokio::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ThreadCreator for CapturingThreadCreator {
    async fn create_thread(
        &self,
        thread_store: Arc<dyn ThreadStore>,
        options: ThreadEnsureOptions,
    ) -> Result<(String, Value), String> {
        self.options.lock().await.push(options.clone());
        let thread_id = "thread::captured";
        let value = json!({
            "thread_id": thread_id,
            "workspace_dir": options.workspace_dir,
            "agent_id": options.agent_id,
        });
        thread_store
            .set(thread_id, value.clone())
            .await
            .map_err(|e| e.to_string())?;
        Ok((thread_id.to_owned(), value))
    }
}

async fn seed_bound_dm_thread(
    store: &Arc<InMemoryThreadStore>,
    thread_id: &str,
    account_id: &str,
    from_id: &str,
    extra: Value,
) {
    let mut base = json!({
        "thread_id": thread_id,
        "thread_id": thread_id,
        "label": format!("telegram/{account_id}/{from_id}"),
        "channel_bindings": [{
            "channel": "telegram",
            "account_id": account_id,
            "binding_key": from_id,
            "chat_id": from_id,
            "display_label": from_id,
            "last_inbound_at": "2026-03-07T10:00:00Z"
        }]
    });
    if let (Some(base_obj), Some(extra_obj)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra_obj {
            base_obj.insert(key.clone(), value.clone());
        }
    }
    store.set(thread_id, base).await.unwrap();
}

fn native_thread_request(command: &str, run_id: &str) -> InboundRequest {
    InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "main".to_owned(),
        from_id: "1000000001".to_owned(),
        is_group: false,
        thread_binding_key: "1000000001".to_owned(),
        message: command.to_owned(),
        run_id: run_id.to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::from([
            (
                NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
                Value::String(command.to_owned()),
            ),
            ("chat_id".to_owned(), Value::String("1000000001".to_owned())),
        ]),
        file_paths: vec![],
    }
}

#[tokio::test]
async fn test_route_and_dispatch_basic() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "hello bot".to_owned(),
        run_id: "run-1".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    assert!(is_thread_key(&result.thread_id));
    assert_eq!(result.metadata.channel.as_deref(), Some("telegram"));
    assert_eq!(result.metadata.from_id.as_deref(), Some("user42"));
    assert!(!result.metadata.is_group);

    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].0, result.thread_id);
    assert_eq!(dispatched[0].1, "hello bot");
    drop(dispatched);

    let saved = router
        .threads
        .get(&result.thread_id)
        .await
        .unwrap()
        .expect("thread should persist delivery context");
    assert_eq!(saved["delivery_context"]["chat_id"], "user42");
    assert!(saved["delivery_context"]["thread_id"].is_null());

    let records = router
        .message_ledger
        .as_ref()
        .expect("message ledger configured")
        .records_for_thread(&result.thread_id, 10)
        .await
        .expect("read message ledger");
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].status,
        garyx_models::MessageLifecycleStatus::ThreadResolved
    );
    assert_eq!(records[0].run_id.as_deref(), Some("run-1"));
}

#[tokio::test]
async fn test_route_and_dispatch_maps_legacy_claude_tty_provider_to_claude_code() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "hello interactive claude".to_owned(),
        run_id: "run-claude-tty".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::from([(
            "requested_provider_type".to_owned(),
            json!("claude_tty"),
        )]),
        file_paths: vec![],
    };

    router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    let requested = dispatcher.requested_providers.lock().await;
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0], Some(ProviderType::ClaudeCode));
}

#[tokio::test]
async fn test_route_and_dispatch_falls_back_to_claude_for_invalid_channel_agent() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("examplebot")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::PluginAccountEntry {
                enabled: true,
                agent_id: Some("missing-agent".to_owned()),
                config: json!({ "token": "test-token" }),
                ..Default::default()
            },
        );

    let (mut router, _) = test_router(store.clone(), config);
    router.set_message_ledger_store(Arc::new(crate::message_ledger::MessageLedgerStore::memory()));
    router.set_thread_creator(Arc::new(FallbackOnlyThreadCreator));
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "examplebot".to_owned(),
        account_id: "main".to_owned(),
        from_id: "user-1".to_owned(),
        is_group: false,
        thread_binding_key: "issue-1".to_owned(),
        message: "hello".to_owned(),
        run_id: "run-examplebot-invalid-agent".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::from([
            ("chat_id".to_owned(), Value::String("issue-1".to_owned())),
            ("issue_id".to_owned(), Value::String("issue-1".to_owned())),
        ]),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    let saved = store
        .get(&result.thread_id)
        .await
        .unwrap()
        .expect("fallback thread should be persisted");
    assert_eq!(saved["agent_id"], "claude");
    assert_eq!(saved["channel"], "examplebot");
    assert_eq!(saved["account_id"], "main");

    let bindings = bindings_from_value(&saved);
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].channel, "examplebot");
    assert_eq!(bindings[0].account_id, "main");
    assert_eq!(bindings[0].binding_key, "issue-1");

    assert_eq!(
        router
            .resolve_endpoint_thread_id("examplebot", "main", "issue-1")
            .await
            .as_deref(),
        Some(result.thread_id.as_str())
    );
}

#[tokio::test]
async fn test_inbound_thread_creation_uses_configured_bot_workspace_mode() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut config = GaryxConfig::default();
    let data_root = tempfile::tempdir().expect("temp data root");
    config.sessions.data_dir = Some(
        data_root
            .path()
            .join("sessions")
            .to_string_lossy()
            .to_string(),
    );
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::PluginAccountEntry {
                enabled: true,
                agent_id: Some("claude".to_owned()),
                workspace_dir: Some("/tmp/test-repo".to_owned()),
                workspace_mode: Some("worktree".to_owned()),
                config: json!({ "token": "test-token" }),
                ..Default::default()
            },
        );

    let creator = Arc::new(CapturingThreadCreator::new());
    let mut router = MessageRouter::new(store, config);
    router.set_thread_creator(creator.clone());

    let thread_id = router
        .resolve_or_create_inbound_thread("telegram", "main", "1000000001", &HashMap::new())
        .await;

    assert_eq!(thread_id, "thread::captured");
    let options = creator.options.lock().await;
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].workspace_mode, WorkspaceMode::Worktree);
    assert_eq!(options[0].workspace_dir.as_deref(), Some("/tmp/test-repo"));
    assert_eq!(
        options[0].worktree_base_dir.as_deref(),
        Some(data_root.path().join("worktrees").as_path())
    );
}

#[tokio::test]
async fn test_route_and_dispatch_uses_explicit_delivery_thread_id() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: true,
        thread_binding_key: "-100123_t555".to_owned(),
        message: "hello topic".to_owned(),
        run_id: "run-topic-explicit".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::from([
            ("chat_id".to_owned(), Value::String("-100123".to_owned())),
            (
                "delivery_thread_id".to_owned(),
                Value::String("555".to_owned()),
            ),
        ]),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    let saved = router
        .threads
        .get(&result.thread_id)
        .await
        .unwrap()
        .expect("thread should persist delivery context");
    assert_eq!(saved["delivery_context"]["chat_id"], "-100123");
    assert_eq!(saved["delivery_context"]["thread_id"], "555");
}

#[tokio::test]
async fn test_route_and_dispatch_weixin_reuses_endpoint_thread_for_same_user() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let first = InboundRequest {
        channel: "weixin".to_owned(),
        account_id: "wx-main".to_owned(),
        from_id: "u@im.wechat".to_owned(),
        is_group: false,
        thread_binding_key: "u@im.wechat".to_owned(),
        message: "first".to_owned(),
        run_id: "run-wx-1".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };
    let first_result = router
        .route_and_dispatch(first, &dispatcher, None)
        .await
        .expect("first dispatch");

    let second = InboundRequest {
        channel: "weixin".to_owned(),
        account_id: "wx-main".to_owned(),
        from_id: "u@im.wechat".to_owned(),
        is_group: false,
        thread_binding_key: "u@im.wechat".to_owned(),
        message: "second".to_owned(),
        run_id: "run-wx-2".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };
    let second_result = router
        .route_and_dispatch(second, &dispatcher, None)
        .await
        .expect("second dispatch");

    assert_eq!(second_result.thread_id, first_result.thread_id);
    assert_eq!(
        router
            .resolve_endpoint_thread_id("weixin", "wx-main", "u@im.wechat")
            .await
            .as_deref(),
        Some(first_result.thread_id.as_str())
    );
}

#[tokio::test]
async fn test_route_and_dispatch_injects_runtime_context_and_workspace() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "bot1".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/runtime-ws".to_owned()),
                    owner_target: None,
                    groups: Default::default(),
                },
            ),
        );
    let mut router = MessageRouter::new(store, config);
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "hello context".to_owned(),
        run_id: "run-ctx-1".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .expect("dispatch should succeed");

    let metadata = dispatcher.metadata.lock().await;
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata[0]["resolved_thread_id"], result.thread_id);
    assert_eq!(metadata[0]["workspace_dir"], "/tmp/runtime-ws");
    assert_eq!(metadata[0]["runtime_context"]["channel"], "telegram");
    assert_eq!(metadata[0]["runtime_context"]["account_id"], "bot1");
    assert_eq!(metadata[0]["runtime_context"]["from_id"], "user42");
    assert_eq!(metadata[0]["runtime_context"]["bot_id"], "telegram:bot1");
    assert_eq!(
        metadata[0]["runtime_context"]["bot"]["thread_binding_key"],
        "user42"
    );
    assert_eq!(
        metadata[0]["runtime_context"]["thread_id"],
        result.thread_id
    );
    assert_eq!(
        metadata[0]["runtime_context"]["workspace_dir"],
        "/tmp/runtime-ws"
    );
    drop(metadata);

    let workspace_dirs = dispatcher.workspace_dirs.lock().await;
    assert_eq!(workspace_dirs.len(), 1);
    assert_eq!(workspace_dirs[0].as_deref(), Some("/tmp/runtime-ws"));
}

#[tokio::test]
async fn test_endpoint_binding_is_binding_key_driven() {
    let router = make_router();
    let metadata = HashMap::new();

    for (channel, account_id) in [
        ("telegram", "bot1"),
        ("feishu", "app1"),
        ("weixin", "wx-main"),
        ("internal", "main"),
    ] {
        let binding = router
            .endpoint_binding_from_inbound(channel, account_id, "u1", &metadata, None)
            .await;
        assert_eq!(binding.channel, channel);
        assert_eq!(binding.account_id, account_id);
        assert_eq!(binding.binding_key, "u1");
    }
}

#[tokio::test]
async fn test_route_and_dispatch_handles_native_sessions_locally() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let mut extra_metadata = HashMap::new();
    extra_metadata.insert(
        NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        Value::String("/threads".to_owned()),
    );

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "/threads".to_owned(),
        run_id: "run-cmd-1".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata,
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    assert!(is_thread_key(&result.thread_id));
    assert!(
        result
            .local_reply
            .as_deref()
            .is_some_and(|text| text.contains("No recent threads yet."))
    );
    let dispatched = dispatcher.dispatched.lock().await;
    assert!(dispatched.is_empty());
}

#[tokio::test]
async fn test_route_and_dispatch_new_session_sets_last_delivery_on_new_thread() {
    let store = Arc::new(InMemoryThreadStore::new());
    let (mut router, _) = test_router(store, GaryxConfig::default());
    let dispatcher = MockDispatcher::new();

    let mut extra_metadata = HashMap::new();
    extra_metadata.insert(
        NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        Value::String("/newthread".to_owned()),
    );
    extra_metadata.insert("chat_id".to_owned(), Value::String("user42".to_owned()));

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "/newthread".to_owned(),
        run_id: "run-cmd-new".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata,
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    let current_thread_id = router
        .get_current_thread_id_for_binding("telegram", "bot1", "user42")
        .expect("new thread should become current thread")
        .to_owned();
    assert!(is_thread_key(&current_thread_id));
    assert_eq!(result.thread_id, current_thread_id);

    let delivery = router
        .get_last_delivery(&current_thread_id)
        .cloned()
        .expect("new thread should get immediate delivery target");
    assert_eq!(delivery.channel, "telegram");
    assert_eq!(delivery.account_id, "bot1");
    assert_eq!(delivery.chat_id, "user42");
}

#[tokio::test]
async fn test_route_and_dispatch_weixin_newthread_binds_endpoint() {
    let store = Arc::new(InMemoryThreadStore::new());
    let (mut router, _) = test_router(store, GaryxConfig::default());
    let dispatcher = MockDispatcher::new();

    let mut extra_metadata = HashMap::new();
    extra_metadata.insert(
        NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        Value::String("/newthread".to_owned()),
    );
    extra_metadata.insert(
        "chat_id".to_owned(),
        Value::String("u@im.wechat".to_owned()),
    );

    let request = InboundRequest {
        channel: "weixin".to_owned(),
        account_id: "wx-main".to_owned(),
        from_id: "u@im.wechat".to_owned(),
        is_group: false,
        thread_binding_key: "u@im.wechat".to_owned(),
        message: "/newthread".to_owned(),
        run_id: "run-cmd-new-wx".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata,
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();
    assert!(
        result
            .local_reply
            .as_deref()
            .is_some_and(|text| text.starts_with("Created and switched to new thread:"))
    );
    assert_eq!(
        router
            .resolve_endpoint_thread_id("weixin", "wx-main", "u@im.wechat")
            .await
            .as_deref(),
        Some(result.thread_id.as_str())
    );
}

#[tokio::test]
async fn test_route_and_dispatch_recent_list_uses_reader_after_newthread_rebind() {
    let store = Arc::new(InMemoryThreadStore::new());
    seed_bound_dm_thread(
        &store,
        "thread::legacy-user42",
        "bot1",
        "user42",
        json!({
            "label": "legacy-thread",
            "channel": "telegram",
            "account_id": "bot1",
            "from_id": "user42",
            "updated_at": "2026-03-07T10:00:00Z"
        }),
    )
    .await;

    let binding = bindings_from_value(
        &store
            .get("thread::legacy-user42")
            .await
            .expect("test store")
            .expect("seeded thread should exist"),
    )
    .into_iter()
    .next()
    .expect("seeded thread should have a binding");
    let (mut router, mutator) = test_router(store, GaryxConfig::default());
    mutator.seed_owner("thread::legacy-user42", binding).await;
    let reader = Arc::new(TestRecentThreadPageReader::new(vec![recent_entry(
        "thread::legacy-user42",
        "legacy-thread",
    )]));
    router.set_recent_thread_page_reader(reader.clone());
    let dispatcher = MockDispatcher::new();

    let mut newthread_meta = HashMap::new();
    newthread_meta.insert(
        NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        Value::String("/newthread".to_owned()),
    );
    newthread_meta.insert("chat_id".to_owned(), Value::String("user42".to_owned()));
    let newthread_result = router
        .route_and_dispatch(
            InboundRequest {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                from_id: "user42".to_owned(),
                is_group: false,
                thread_binding_key: "user42".to_owned(),
                message: "/newthread".to_owned(),
                run_id: "run-cmd-new-threads-list".to_owned(),
                reply_to_message_id: None,
                images: vec![],
                extra_metadata: newthread_meta,
                file_paths: vec![],
            },
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert!(
        newthread_result
            .local_reply
            .as_deref()
            .is_some_and(|text| text.starts_with("Created and switched to new thread: thread-"))
    );
    reader
        .replace_entries(vec![
            recent_entry(&newthread_result.thread_id, "new global thread"),
            recent_entry("thread::legacy-user42", "legacy-thread"),
        ])
        .await;

    let mut threads_meta = HashMap::new();
    threads_meta.insert(
        NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        Value::String("/threads".to_owned()),
    );
    let threads_result = router
        .route_and_dispatch(
            InboundRequest {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                from_id: "user42".to_owned(),
                is_group: false,
                thread_binding_key: "user42".to_owned(),
                message: "/threads".to_owned(),
                run_id: "run-cmd-threads-after-new".to_owned(),
                reply_to_message_id: None,
                images: vec![],
                extra_metadata: threads_meta,
                file_paths: vec![],
            },
            &dispatcher,
            None,
        )
        .await
        .unwrap();

    let list_text = threads_result.local_reply.unwrap_or_default();
    assert!(list_text.contains("Recent threads · page 1/1 (2 total)"));
    assert!(list_text.contains("legacy-thread"));
    assert!(list_text.contains("new global thread"));
    assert!(list_text.contains("⬅️"));
}

#[tokio::test]
async fn test_recent_pages_bind_exact_snapshot_after_projection_order_drifts() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::recent-12",
            json!({"thread_id": "thread::recent-12", "label": "Thread 12"}),
        )
        .await
        .unwrap();
    let (mut router, mutator) = test_router(store, GaryxConfig::default());
    let entries = (1..=12)
        .map(|index| {
            recent_entry(
                &format!("thread::recent-{index}"),
                &format!("Thread {index}"),
            )
        })
        .collect::<Vec<_>>();
    let reader = Arc::new(TestRecentThreadPageReader::new(entries.clone()));
    router.set_recent_thread_page_reader(reader.clone());
    let dispatcher = MockDispatcher::new();

    let first = router
        .route_and_dispatch(
            native_thread_request("/threads", "run-recent-page-1"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert!(first.local_reply.unwrap().contains("page 1/2 (12 total)"));
    let second = router
        .route_and_dispatch(
            native_thread_request("/threads next", "run-recent-page-2"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert!(second.local_reply.unwrap().contains("12. Thread 12"));

    let mut drifted = vec![recent_entry("thread::new-head", "New head")];
    drifted.extend(entries);
    reader.replace_entries(drifted).await;

    let bound = router
        .route_and_dispatch(
            native_thread_request("/bindthread 12", "run-bind-snapshot-12"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(bound.thread_id, "thread::recent-12");
    assert_eq!(
        bound.local_reply.as_deref(),
        Some("Switched to thread: Thread 12")
    );
    let owner = mutator
        .binding_for_endpoint("telegram::main::1000000001")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owner.thread_id, "thread::recent-12");

    let ordinary = router
        .route_and_dispatch(
            native_thread_request("hello after binding", "run-after-bind"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(ordinary.thread_id, "thread::recent-12");
    assert_eq!(dispatcher.dispatched.lock().await[0].0, "thread::recent-12");
}

#[tokio::test]
async fn test_recent_page_navigation_boundaries_usage_and_reader_failure() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::page-11",
            json!({"thread_id": "thread::page-11", "label": "Page row 11"}),
        )
        .await
        .unwrap();
    let (mut router, _) = test_router(store, GaryxConfig::default());
    let reader = Arc::new(TestRecentThreadPageReader::new(
        (1..=12)
            .map(|index| {
                recent_entry(
                    &format!("thread::page-{index}"),
                    &format!("Page row {index}"),
                )
            })
            .collect(),
    ));
    router.set_recent_thread_page_reader(reader.clone());
    let dispatcher = MockDispatcher::new();

    let first = router
        .route_and_dispatch(
            native_thread_request("/threads next", "run-next-without-state"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert!(first.local_reply.unwrap().contains("page 1/2"));
    let second = router
        .route_and_dispatch(
            native_thread_request("/threads next", "run-next-page-2"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert!(second.local_reply.unwrap().contains("page 2/2"));
    let boundary = router
        .route_and_dispatch(
            native_thread_request("/threads next", "run-next-boundary"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert!(
        boundary
            .local_reply
            .unwrap()
            .starts_with("Already on the last page.\nRecent threads · page 2/2")
    );
    let previous = router
        .route_and_dispatch(
            native_thread_request("/threads prev", "run-prev-page-1"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert!(previous.local_reply.unwrap().contains("page 1/2"));

    let out_of_range = router
        .route_and_dispatch(
            native_thread_request("/threads 7", "run-page-out-of-range"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        out_of_range.local_reply.as_deref(),
        Some("Page 7 is out of range (2 pages). Use /threads 2.")
    );
    let preserved_snapshot = router
        .route_and_dispatch(
            native_thread_request("/bindthread 11", "run-bind-after-out-of-range"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(preserved_snapshot.thread_id, "thread::page-11");
    let calls_before_invalid = reader.page_calls.load(Ordering::SeqCst);
    let invalid = router
        .route_and_dispatch(
            native_thread_request("/threads later", "run-page-invalid"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        invalid.local_reply.as_deref(),
        Some("Usage: /threads [page|next|prev]")
    );
    assert_eq!(
        reader.page_calls.load(Ordering::SeqCst),
        calls_before_invalid
    );

    reader.fail_page.store(true, Ordering::SeqCst);
    let unavailable = router
        .route_and_dispatch(
            native_thread_request("/threads 2", "run-page-reader-failure"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        unavailable.local_reply.as_deref(),
        Some("Recent threads are temporarily unavailable. Try again.")
    );
}

#[tokio::test]
async fn test_bindthread_direct_guard_idempotence_and_deprecated_commands() {
    let store = Arc::new(InMemoryThreadStore::new());
    let target = "thread::12345678-1234-1234-1234-123456789abc";
    let incompatible = "thread::aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    store
        .set(
            target,
            json!({"thread_id": target, "label": "Direct target"}),
        )
        .await
        .unwrap();
    store
        .set(
            incompatible,
            json!({
                "thread_id": incompatible,
                "label": "Other channel",
                "channel": "weixin",
                "account_id": "main"
            }),
        )
        .await
        .unwrap();
    let (mut router, mutator) = test_router(store, GaryxConfig::default());
    let reader = Arc::new(TestRecentThreadPageReader::new(vec![
        recent_entry(target, "Direct target"),
        recent_entry(incompatible, "Other channel"),
    ]));
    router.set_recent_thread_page_reader(reader.clone());
    let dispatcher = MockDispatcher::new();

    let direct = router
        .route_and_dispatch(
            native_thread_request(
                &format!("/bindthread {target}"),
                "run-bind-direct-canonical",
            ),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(direct.thread_id, target);
    assert_eq!(
        direct.local_reply.as_deref(),
        Some("Switched to thread: Direct target")
    );

    let idempotent = router
        .route_and_dispatch(
            native_thread_request(
                &format!("/bindthread {target}"),
                "run-bind-direct-idempotent",
            ),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        idempotent.local_reply.as_deref(),
        Some("Already on thread: Direct target")
    );

    let rejected = router
        .route_and_dispatch(
            native_thread_request(
                &format!("/bindthread {incompatible}"),
                "run-bind-incompatible",
            ),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert!(
        rejected
            .local_reply
            .as_deref()
            .is_some_and(|reply| reply.contains("thread belongs to channel 'weixin'"))
    );
    assert_eq!(rejected.thread_id, target);

    reader.replace_entries(Vec::new()).await;
    let no_longer_selectable = router
        .route_and_dispatch(
            native_thread_request(
                &format!("/bindthread {target}"),
                "run-bind-no-longer-selectable",
            ),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        no_longer_selectable.local_reply.as_deref(),
        Some("That thread no longer exists. Run /threads again.")
    );

    for (command, direction) in [("/threadprev", "prev"), ("/threadnext", "next")] {
        let deprecated = router
            .route_and_dispatch(
                native_thread_request(command, &format!("run-deprecated-{direction}")),
                &dispatcher,
                None,
            )
            .await
            .unwrap();
        assert_eq!(deprecated.thread_id, target);
        assert_eq!(
            deprecated.local_reply.as_deref(),
            Some(
                format!(
                    "{command} no longer switches threads. Use /threads {direction}, then /bindthread <n>."
                )
                .as_str()
            )
        );
    }
    assert_eq!(
        mutator
            .binding_for_endpoint("telegram::main::1000000001")
            .await
            .unwrap()
            .unwrap()
            .thread_id,
        target
    );
}

#[tokio::test]
async fn test_newthread_clears_recent_selection_snapshot() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::snapshot-target",
            json!({"thread_id": "thread::snapshot-target", "label": "Snapshot target"}),
        )
        .await
        .unwrap();
    let (mut router, mutator) = test_router(store, GaryxConfig::default());
    router.set_recent_thread_page_reader(Arc::new(TestRecentThreadPageReader::new(vec![
        recent_entry("thread::snapshot-target", "Snapshot target"),
    ])));
    let dispatcher = MockDispatcher::new();

    router
        .route_and_dispatch(
            native_thread_request("/threads", "run-snapshot-before-new"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    let created = router
        .route_and_dispatch(
            native_thread_request("/newthread", "run-clear-snapshot-new"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    let missing_snapshot = router
        .route_and_dispatch(
            native_thread_request("/bindthread 1", "run-cleared-snapshot-bind"),
            &dispatcher,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        missing_snapshot.local_reply.as_deref(),
        Some("Run /threads first, then /bindthread <n>.")
    );
    assert_eq!(missing_snapshot.thread_id, created.thread_id);
    assert_eq!(
        mutator
            .binding_for_endpoint("telegram::main::1000000001")
            .await
            .unwrap()
            .unwrap()
            .thread_id,
        created.thread_id
    );
}

#[tokio::test]
async fn test_recent_commands_have_explicit_missing_reader_error() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    router.set_endpoint_binding_mutator(Arc::new(TestEndpointBindingMutator::new(store)));
    let dispatcher = MockDispatcher::new();

    for (command, run_id) in [
        ("/threads", "run-missing-reader-list"),
        (
            "/bindthread thread::12345678-1234-1234-1234-123456789abc",
            "run-missing-reader-bind",
        ),
    ] {
        let result = router
            .route_and_dispatch(native_thread_request(command, run_id), &dispatcher, None)
            .await
            .unwrap();
        assert_eq!(
            result.local_reply.as_deref(),
            Some("Recent threads are temporarily unavailable. Try again.")
        );
    }
}

#[tokio::test]
async fn test_threads_bindthread_and_newthread_never_list_store_keys() {
    let store = Arc::new(NoScanThreadStore::new());
    store
        .set(
            "thread::no-scan-target",
            json!({"thread_id": "thread::no-scan-target", "label": "No scan target"}),
        )
        .await
        .unwrap();
    let (mut router, _) = test_router(store.clone(), GaryxConfig::default());
    router.set_recent_thread_page_reader(Arc::new(TestRecentThreadPageReader::new(vec![
        recent_entry("thread::no-scan-target", "No scan target"),
    ])));
    let dispatcher = MockDispatcher::new();

    for (command, run_id) in [
        ("/threads", "run-no-scan-list"),
        ("/bindthread 1", "run-no-scan-bind"),
        ("/newthread", "run-no-scan-new"),
    ] {
        router
            .route_and_dispatch(native_thread_request(command, run_id), &dispatcher, None)
            .await
            .unwrap();
    }
    assert_eq!(store.list_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_route_and_dispatch_native_command_uses_metadata_text() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let mut extra_metadata = HashMap::new();
    extra_metadata.insert(
        NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        Value::String("/threads".to_owned()),
    );
    let request = InboundRequest {
        channel: "feishu".to_owned(),
        account_id: "app1".to_owned(),
        from_id: "ou_user".to_owned(),
        is_group: false,
        thread_binding_key: "ou_user".to_owned(),
        message: "ou_user: /threads".to_owned(),
        run_id: "run-cmd-2".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata,
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();
    assert!(
        result
            .local_reply
            .as_deref()
            .is_some_and(|text| text.contains("No recent threads yet."))
    );
    let dispatched = dispatcher.dispatched.lock().await;
    assert!(dispatched.is_empty());
}

#[tokio::test]
async fn test_route_and_dispatch_transforms_custom_slash_command() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut config = GaryxConfig::default();
    config.commands.push(SlashCommand {
        name: "summary".to_owned(),
        description: "Summarize the thread".to_owned(),
        prompt: Some("Please summarize the active conversation.".to_owned()),
        skill_id: Some("summary-skill".to_owned()),
    });
    let mut router = MessageRouter::new(store, config);
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "/summary".to_owned(),
        run_id: "run-custom-command".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].1, "Please summarize the active conversation.");
    drop(dispatched);

    let metadata = dispatcher.metadata.lock().await;
    assert_eq!(
        metadata[0].get("slash_command_name"),
        Some(&json!("summary"))
    );
    assert_eq!(
        metadata[0].get("slash_command_skill_id"),
        Some(&json!("summary-skill"))
    );
    assert_eq!(
        result.metadata.extra.get("slash_command_name"),
        Some(&json!("summary"))
    );
}

#[tokio::test]
async fn test_route_and_dispatch_group() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: true,
        thread_binding_key: "group_123".to_owned(),
        message: "group msg".to_owned(),
        run_id: "run-2".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    assert!(result.thread_id.starts_with("thread::"));
    assert!(result.metadata.is_group);
    assert_eq!(result.metadata.thread_id.as_deref(), Some("group_123"));
}

#[tokio::test]
async fn test_route_and_dispatch_group_reuses_thread() {
    // Feishu / Telegram groups: consecutive messages in the same group chat
    // must resolve to the same canonical thread, not create a new one each time.
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let make_req = |run_id: &str| InboundRequest {
        channel: "feishu".to_owned(),
        account_id: "main".to_owned(),
        from_id: "oc_abc123".to_owned(),
        is_group: true,
        thread_binding_key: "oc_abc123".to_owned(),
        message: "hello".to_owned(),
        run_id: run_id.to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result1 = router
        .route_and_dispatch(make_req("run-g1"), &dispatcher, None)
        .await
        .unwrap();

    let result2 = router
        .route_and_dispatch(make_req("run-g2"), &dispatcher, None)
        .await
        .unwrap();

    assert!(result1.thread_id.starts_with("thread::"));
    assert_eq!(
        result1.thread_id, result2.thread_id,
        "consecutive group messages must route to the same thread"
    );
}

#[tokio::test]
async fn test_route_and_dispatch_with_reply_routing() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();
    router
        .threads
        .set("thread::special", json!({"messages": []}))
        .await
        .unwrap();

    // Record an outbound message first
    router.record_outbound_message("thread::special", "telegram", "bot1", "msg42");

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "reply msg".to_owned(),
        run_id: "run-3".to_owned(),
        reply_to_message_id: Some("msg42".to_owned()),
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    // Should route to the reply thread, not the default
    assert_eq!(result.thread_id, "thread::special");
    let metadata = dispatcher.metadata.lock().await;
    assert_eq!(metadata.len(), 1);
    assert_eq!(
        metadata[0].get("reply_to_message_id"),
        Some(&json!("msg42"))
    );
    assert_eq!(metadata[0].get("is_reply_routed"), Some(&json!(true)));
}

#[tokio::test]
async fn test_route_and_dispatch_reply_routing_switches_scheduled_thread() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();
    router
        .threads
        .set("cron::daily::user42", json!({"messages": []}))
        .await
        .unwrap();
    router.record_outbound_message("cron::daily::user42", "telegram", "bot1", "msg42");

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "reply msg".to_owned(),
        run_id: "run-3s".to_owned(),
        reply_to_message_id: Some("msg42".to_owned()),
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();
    assert_eq!(result.thread_id, "cron::daily::user42");
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "bot1", "user42"),
        Some("cron::daily::user42")
    );
}

#[tokio::test]
async fn test_route_and_dispatch_reply_routing_is_scoped_by_chat_id() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();
    router
        .threads
        .set("session_chat_1", json!({"messages": []}))
        .await
        .unwrap();
    router
        .threads
        .set("session_chat_2", json!({"messages": []}))
        .await
        .unwrap();

    router.record_outbound_message_for_chat(
        "session_chat_1",
        "telegram",
        "bot1",
        "chat-1",
        None,
        "42",
    );
    router.record_outbound_message_for_chat(
        "session_chat_2",
        "telegram",
        "bot1",
        "chat-2",
        None,
        "42",
    );

    let mut extra_metadata = HashMap::new();
    extra_metadata.insert("chat_id".to_owned(), json!("chat-2"));

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "reply msg".to_owned(),
        run_id: "run-chat-scope".to_owned(),
        reply_to_message_id: Some("42".to_owned()),
        images: vec![],
        extra_metadata,
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();
    assert_eq!(result.thread_id, "session_chat_2");
}

#[tokio::test]
async fn test_route_and_dispatch_reply_routing_backfills_missing_thread_context() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::special",
            json!({
                "messages": [{"role": "assistant", "content": "hello"}]
            }),
        )
        .await
        .unwrap();

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    let dispatcher = MockDispatcher::new();
    router.record_outbound_message("thread::special", "telegram", "bot1", "msg42");

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "reply msg".to_owned(),
        run_id: "run-3b".to_owned(),
        reply_to_message_id: Some("msg42".to_owned()),
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();
    assert_eq!(result.thread_id, "thread::special");

    let thread_state = store.get("thread::special").await.unwrap().unwrap();
    assert_eq!(thread_state["channel"], "telegram");
    assert_eq!(thread_state["account_id"], "bot1");
    assert_eq!(thread_state["from_id"], "user42");
    assert_eq!(thread_state["is_group"], false);
    assert!(thread_state.get("updated_at").is_some());
}

#[tokio::test]
async fn test_route_and_dispatch_ignores_stale_reply_route_for_missing_thread() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    let dispatcher = MockDispatcher::new();

    router.record_outbound_message_for_chat(
        "thread::missing",
        "telegram",
        "bot1",
        "42",
        None,
        "msg42",
    );

    let mut extra_metadata = HashMap::new();
    extra_metadata.insert("chat_id".to_owned(), json!("42"));

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "42".to_owned(),
        is_group: false,
        thread_binding_key: "42".to_owned(),
        message: "reply msg".to_owned(),
        run_id: "run-stale-reply".to_owned(),
        reply_to_message_id: Some("msg42".to_owned()),
        images: vec![],
        extra_metadata,
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();
    assert!(is_thread_key(&result.thread_id));
    assert_ne!(result.thread_id, "thread::missing");
    assert!(!result.metadata.extra.contains_key("is_reply_routed"));
}

#[tokio::test]
async fn test_route_and_dispatch_backfill_does_not_override_existing_context() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::special",
            json!({
                "channel": "feishu",
                "account_id": "app1",
                "from_id": "ou_existing",
                "is_group": true
            }),
        )
        .await
        .unwrap();

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    let dispatcher = MockDispatcher::new();
    router.record_outbound_message("thread::special", "telegram", "bot1", "msg42");

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "reply msg".to_owned(),
        run_id: "run-3c".to_owned(),
        reply_to_message_id: Some("msg42".to_owned()),
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();
    assert_eq!(result.thread_id, "thread::special");

    let thread_state = store.get("thread::special").await.unwrap().unwrap();
    assert_eq!(thread_state["channel"], "feishu");
    assert_eq!(thread_state["account_id"], "app1");
    assert_eq!(thread_state["from_id"], "ou_existing");
    assert_eq!(thread_state["is_group"], true);
}

#[tokio::test]
async fn test_route_and_dispatch_with_images() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let images = vec![ImagePayload {
        name: "probe.png".to_owned(),
        data: "abc123".to_owned(),
        media_type: "image/png".to_owned(),
    }];

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "analyze this".to_owned(),
        run_id: "run-4".to_owned(),
        reply_to_message_id: None,
        images,
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    assert!(is_thread_key(&result.thread_id));

    let dispatched = dispatcher.dispatched.lock().await;
    let imgs = dispatched[0].2.as_ref().unwrap();
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].data, "abc123");
    assert_eq!(imgs[0].media_type, "image/png");
}

#[tokio::test]
async fn test_route_and_dispatch_persists_attached_file_paths_as_metadata() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "please inspect this document".to_owned(),
        run_id: "run-file-1".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![
            "/tmp/garyx-telegram/inbound/a-report.pdf".to_owned(),
            "/tmp/garyx-telegram/inbound/b-notes.txt".to_owned(),
        ],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    assert!(is_thread_key(&result.thread_id));

    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched[0].1, "please inspect this document");
    drop(dispatched);

    let metadata = dispatcher.metadata.lock().await;
    let attachments = metadata[0]
        .get("attachments")
        .and_then(Value::as_array)
        .expect("attachments metadata");
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0]["kind"], "file");
    assert_eq!(
        attachments[0]["path"],
        "/tmp/garyx-telegram/inbound/a-report.pdf"
    );
    assert_eq!(
        attachments[1]["path"],
        "/tmp/garyx-telegram/inbound/b-notes.txt"
    );
}

#[tokio::test]
async fn test_route_and_dispatch_preserves_existing_path_image_attachments_metadata() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "acmechat".to_owned(),
        account_id: "main".to_owned(),
        from_id: "issue-42".to_owned(),
        is_group: false,
        thread_binding_key: "issue-42".to_owned(),
        message: "please inspect the issue attachments".to_owned(),
        run_id: "run-acmechat-attachments".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::from([(
            "attachments".to_owned(),
            json!([
                {
                    "kind": "image",
                    "path": "/tmp/garyx-acmechat/inbound/shot.png",
                    "name": "shot.png",
                    "media_type": "image/png"
                }
            ]),
        )]),
        file_paths: vec!["/tmp/garyx-acmechat/inbound/spec.pdf".to_owned()],
    };

    router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    let metadata = dispatcher.metadata.lock().await;
    let attachments = metadata[0]
        .get("attachments")
        .and_then(Value::as_array)
        .expect("attachments metadata");
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0]["kind"], "image");
    assert_eq!(
        attachments[0]["path"],
        "/tmp/garyx-acmechat/inbound/shot.png"
    );
    assert_eq!(attachments[1]["kind"], "file");
    assert_eq!(
        attachments[1]["path"],
        "/tmp/garyx-acmechat/inbound/spec.pdf"
    );
}

#[tokio::test]
async fn test_route_and_dispatch_updates_last_delivery() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "hello".to_owned(),
        run_id: "run-5".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    let delivery = router.get_last_delivery(&result.thread_id).unwrap();
    assert_eq!(delivery.channel, "telegram");
    assert_eq!(delivery.account_id, "bot1");
    assert_eq!(delivery.chat_id, "user42");
}

#[tokio::test]
async fn test_route_and_dispatch_persists_last_delivery_context() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    let dispatcher = MockDispatcher::new();
    store
        .set(
            "thread::topic-1-seeded",
            json!({
                "messages": []
            }),
        )
        .await
        .unwrap();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: true,
        thread_binding_key: "topic-1".to_owned(),
        message: "hello".to_owned(),
        run_id: "run-5-persist".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    let saved = store
        .get(&result.thread_id)
        .await
        .unwrap()
        .expect("group thread should persist delivery context");
    assert_eq!(saved["last_channel"], "telegram");
    assert_eq!(saved["last_to"], "user42");
    assert_eq!(saved["last_account_id"], "bot1");
    assert!(saved["lastUpdatedAt"].is_string());
    assert_eq!(saved["delivery_context"]["thread_id"], "topic-1");
}

#[tokio::test]
async fn test_route_and_dispatch_last_delivery_prefers_chat_id_metadata() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::new();

    let mut extra = HashMap::new();
    extra.insert(
        "chat_id".to_owned(),
        Value::Number(serde_json::Number::from(777)),
    );

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: true,
        thread_binding_key: "topic-1".to_owned(),
        message: "hello".to_owned(),
        run_id: "run-5b".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: extra,
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    let delivery = router.get_last_delivery(&result.thread_id).unwrap();
    assert_eq!(delivery.chat_id, "777");
    assert_eq!(delivery.user_id, "user42");
}

#[tokio::test]
async fn test_route_and_dispatch_auto_recovery() {
    let store = Arc::new(InMemoryThreadStore::new());
    seed_bound_dm_thread(
        &store,
        "thread::user42-v1",
        "bot1",
        "user42",
        json!({
            "auto_recover_next_thread": "thread::user42-v2"
        }),
    )
    .await;
    store
        .set(
            "thread::user42-v2",
            json!({
                "thread_id": "thread::user42-v2",
                "thread_id": "thread::user42-v2",
                "messages": []
            }),
        )
        .await
        .unwrap();

    // Endpoint owner resolution is a mutator point lookup now (no
    // startup rebuild): seed the owner exactly like the production
    // bootstrap-projected state (#TASK-2099).
    let (mut router, mutator) = test_router(store.clone(), GaryxConfig::default());
    let seeded_binding = bindings_from_value(
        &store
            .get("thread::user42-v1")
            .await
            .expect("test store")
            .expect("seeded thread should exist"),
    )
    .into_iter()
    .next()
    .expect("seeded thread should have a binding");
    mutator
        .seed_owner("thread::user42-v1", seeded_binding)
        .await;
    let dispatcher = MockDispatcher::new();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "hello".to_owned(),
        run_id: "run-6".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    // Should have been redirected
    assert_eq!(result.thread_id, "thread::user42-v2");
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "bot1", "user42"),
        Some("thread::user42-v2")
    );

    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched[0].0, "thread::user42-v2");
}

#[tokio::test]
async fn test_route_and_dispatch_auto_recovery_ignores_missing_target() {
    let store = Arc::new(InMemoryThreadStore::new());
    seed_bound_dm_thread(
        &store,
        "thread::user42-v1",
        "bot1",
        "user42",
        json!({
            "auto_recover_next_thread": "thread::missing-target"
        }),
    )
    .await;

    // Endpoint owner resolution is a mutator point lookup now (no
    // startup rebuild): seed the owner exactly like the production
    // bootstrap-projected state (#TASK-2099).
    let (mut router, mutator) = test_router(store.clone(), GaryxConfig::default());
    let seeded_binding = bindings_from_value(
        &store
            .get("thread::user42-v1")
            .await
            .expect("test store")
            .expect("seeded thread should exist"),
    )
    .into_iter()
    .next()
    .expect("seeded thread should have a binding");
    mutator
        .seed_owner("thread::user42-v1", seeded_binding)
        .await;
    let dispatcher = MockDispatcher::new();
    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "hello".to_owned(),
        run_id: "run-6b".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    assert_eq!(result.thread_id, "thread::user42-v1");
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "bot1", "user42"),
        None
    );
    assert_eq!(
        router
            .resolve_endpoint_thread_id("telegram", "bot1", "user42")
            .await
            .as_deref(),
        Some("thread::user42-v1")
    );
    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched[0].0, "thread::user42-v1");
}

#[tokio::test]
async fn test_route_and_dispatch_uses_projected_owner_without_rebuilt_endpoint_map() {
    let store = Arc::new(InMemoryThreadStore::new());
    seed_bound_dm_thread(
        &store,
        "thread::user42-existing",
        "bot1",
        "user42",
        json!({}),
    )
    .await;

    let binding = bindings_from_value(
        &store
            .get("thread::user42-existing")
            .await
            .expect("test store")
            .expect("seeded thread should exist"),
    )
    .into_iter()
    .next()
    .expect("seeded thread should have a binding");
    let (mut router, mutator) = test_router(store.clone(), GaryxConfig::default());
    mutator.seed_owner("thread::user42-existing", binding).await;
    let dispatcher = MockDispatcher::new();
    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "hello after restart".to_owned(),
        run_id: "run-lazy-endpoint-binding".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    assert!(router.thread_nav.endpoint_thread_map.is_empty());
    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    assert_eq!(result.thread_id, "thread::user42-existing");
    assert_eq!(
        router
            .resolve_endpoint_thread_id("telegram", "bot1", "user42")
            .await
            .as_deref(),
        Some("thread::user42-existing")
    );
    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched[0].0, "thread::user42-existing");
}

#[tokio::test]
async fn test_route_and_dispatch_reply_routing_falls_back_to_rebound_thread_when_old_thread_is_missing()
 {
    let store = Arc::new(InMemoryThreadStore::new());
    let store_dyn: Arc<dyn crate::ThreadStore> = store.clone();
    seed_bound_dm_thread(&store, "thread::old", "bot1", "user42", json!({})).await;
    let (new_thread, _) = create_thread_record(
        &store_dyn,
        ThreadEnsureOptions {
            label: Some("Rebound".to_owned()),
            ..Default::default()
        },
    )
    .await
    .expect("thread should be created");
    let (mut router, mutator) = test_router(store.clone(), GaryxConfig::default());
    let old_binding = bindings_from_value(&store.get("thread::old").await.unwrap().unwrap())
        .into_iter()
        .next()
        .unwrap();
    mutator.seed_owner("thread::old", old_binding).await;
    router
        .bind_endpoint_runtime(
            &new_thread,
            ChannelBinding {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                binding_key: "user42".to_owned(),
                chat_id: "user42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "user42".to_owned(),
                display_label: "user42".to_owned(),
                last_inbound_at: None,
                last_delivery_at: None,
            },
        )
        .await
        .expect("bind should succeed");

    router.record_outbound_message_for_chat(
        "thread::old",
        "telegram",
        "bot1",
        "user42",
        None,
        "reply-1",
    );
    assert!(store.delete("thread::old").await.unwrap());

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "follow missing thread".to_owned(),
        run_id: "run-rebound-fallback".to_owned(),
        reply_to_message_id: Some("reply-1".to_owned()),
        images: vec![],
        extra_metadata: HashMap::from([("chat_id".to_owned(), Value::String("user42".to_owned()))]),
        file_paths: vec![],
    };

    let dispatcher = MockDispatcher::new();
    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    assert_eq!(result.thread_id, new_thread);
    assert_eq!(
        router
            .resolve_endpoint_thread_id("telegram", "bot1", "user42")
            .await
            .as_deref(),
        Some(new_thread.as_str())
    );
    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched[0].0, new_thread);
}

#[tokio::test]
async fn test_route_and_dispatch_reply_routing_falls_back_after_real_initial_dispatch_and_delete() {
    let store = Arc::new(InMemoryThreadStore::new());
    let store_dyn: Arc<dyn crate::ThreadStore> = store.clone();
    let (mut router, _) = test_router(store.clone(), GaryxConfig::default());
    let dispatcher = MockDispatcher::new();

    let initial = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "first message".to_owned(),
        run_id: "run-initial".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::from([("chat_id".to_owned(), Value::String("user42".to_owned()))]),
        file_paths: vec![],
    };

    let initial_result = router
        .route_and_dispatch(initial, &dispatcher, None)
        .await
        .unwrap();
    router.record_outbound_message_for_chat(
        &initial_result.thread_id,
        "telegram",
        "bot1",
        "user42",
        None,
        "reply-1",
    );

    let (new_thread, _) = create_thread_record(
        &store_dyn,
        ThreadEnsureOptions {
            label: Some("Rebound".to_owned()),
            ..Default::default()
        },
    )
    .await
    .expect("thread should be created");
    router
        .bind_endpoint_runtime(
            &new_thread,
            ChannelBinding {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                binding_key: "user42".to_owned(),
                chat_id: "user42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "user42".to_owned(),
                display_label: "user42".to_owned(),
                last_inbound_at: None,
                last_delivery_at: None,
            },
        )
        .await
        .expect("bind should succeed");

    assert!(store.delete(&initial_result.thread_id).await.unwrap());

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "follow missing thread".to_owned(),
        run_id: "run-rebound-fallback-real".to_owned(),
        reply_to_message_id: Some("reply-1".to_owned()),
        images: vec![],
        extra_metadata: HashMap::from([("chat_id".to_owned(), Value::String("user42".to_owned()))]),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();

    assert_eq!(result.thread_id, new_thread);
    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched.last().unwrap().0, new_thread);
}

#[tokio::test]
async fn test_route_and_dispatch_scheduled_thread_skips_auto_recovery() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "cron::daily::user42",
            json!({
                "auto_recover_next_thread": "bot1::main::recovered"
            }),
        )
        .await
        .unwrap();
    store
        .set("bot1::main::recovered", json!({"messages": []}))
        .await
        .unwrap();

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let dispatcher = MockDispatcher::new();
    router.record_outbound_message("cron::daily::user42", "telegram", "bot1", "msg42");

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "reply msg".to_owned(),
        run_id: "run-6c".to_owned(),
        reply_to_message_id: Some("msg42".to_owned()),
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let result = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap();
    assert_eq!(result.thread_id, "cron::daily::user42");
    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched[0].0, "cron::daily::user42");
}

#[tokio::test]
async fn test_route_and_dispatch_failure() {
    let mut router = make_router();
    let dispatcher = MockDispatcher::failing();

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user42".to_owned(),
        is_group: false,
        thread_binding_key: "user42".to_owned(),
        message: "hello".to_owned(),
        run_id: "run-7".to_owned(),
        reply_to_message_id: None,
        images: vec![],
        extra_metadata: HashMap::new(),
        file_paths: vec![],
    };

    let err = router
        .route_and_dispatch(request, &dispatcher, None)
        .await
        .unwrap_err();

    assert_eq!(err, "mock dispatch failure");
}

#[tokio::test]
async fn test_dispatch_to_existing_session_keeps_explicit_target() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    let dispatcher = MockDispatcher::new();

    seed_bound_dm_thread(
        &store,
        "thread::old-session",
        "bot1",
        "user42",
        json!({
            "channel": "telegram",
            "account_id": "bot1",
            "from_id": "user42",
            "is_group": false,
        }),
    )
    .await;
    seed_bound_dm_thread(
        &store,
        "thread::current-session",
        "bot1",
        "user42",
        json!({
            "channel": "telegram",
            "account_id": "bot1",
            "from_id": "user42",
            "is_group": false,
        }),
    )
    .await;

    let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "user42");
    router.switch_to_thread(&user_key, "thread::current-session");

    let request = ThreadMessageRequest {
        message: "continue working".to_owned(),
        run_id: "run-explicit-session".to_owned(),
        extra_metadata: HashMap::new(),
        images: vec![],
        file_paths: vec![],
    };

    let result = router
        .dispatch_message_to_thread("thread::old-session", request, &dispatcher, None)
        .await
        .unwrap();

    assert_eq!(result.thread_id, "thread::old-session");
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "bot1", "user42"),
        Some("thread::current-session")
    );

    let dispatched = dispatcher.dispatched.lock().await;
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].0, "thread::old-session");
    assert_eq!(dispatched[0].1, "continue working");
}

#[test]
fn test_wrap_response_callback() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let inner_called = Arc::new(AtomicBool::new(false));
    let inner_flag = inner_called.clone();
    let inner: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        if matches!(event, StreamEvent::Done) {
            inner_flag.store(true, Ordering::Relaxed);
        }
    });

    let record_called = Arc::new(AtomicBool::new(false));
    let record_flag = record_called.clone();

    let wrapped = MessageRouter::wrap_response_callback(inner, move |_msg_id| {
        record_flag.store(true, Ordering::Relaxed);
    });

    // Non-final call
    wrapped(StreamEvent::Delta {
        text: "chunk".to_owned(),
    });
    assert!(!inner_called.load(Ordering::Relaxed));
    assert!(!record_called.load(Ordering::Relaxed));

    // Final call
    wrapped(StreamEvent::Done);
    assert!(inner_called.load(Ordering::Relaxed));
    assert!(record_called.load(Ordering::Relaxed));
}
