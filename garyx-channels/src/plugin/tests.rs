use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use garyx_models::ChannelOutboundContent;
use garyx_models::config::{FeishuAccount, GaryxConfig, TelegramAccount, WeixinAccount};
use garyx_router::{InMemoryThreadStore, ThreadStore};

fn insert_telegram_plugin_account(
    config: &mut GaryxConfig,
    account_id: &str,
    account: TelegramAccount,
) {
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            account_id.to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&account),
        );
}

fn insert_feishu_plugin_account(
    config: &mut GaryxConfig,
    account_id: &str,
    account: FeishuAccount,
) {
    config
        .channels
        .plugin_channel_mut("feishu")
        .accounts
        .insert(
            account_id.to_owned(),
            garyx_models::config::feishu_account_to_plugin_entry(&account),
        );
}

fn insert_weixin_plugin_account(
    config: &mut GaryxConfig,
    account_id: &str,
    account: WeixinAccount,
) {
    config
        .channels
        .plugin_channel_mut("weixin")
        .accounts
        .insert(
            account_id.to_owned(),
            garyx_models::config::weixin_account_to_plugin_entry(&account),
        );
}

#[test]
fn read_icon_as_data_url_handles_svg() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("icon.svg");
    // Minimal but valid SVG — good enough for base64 round-trip.
    std::fs::write(
        &path,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#,
    )
    .unwrap();
    let url = read_icon_as_data_url(&path).expect("icon read");
    assert!(
        url.starts_with("data:image/svg+xml;base64,"),
        "expected SVG data URL, got {url}"
    );
    // Base64 round-trip sanity: decoding the tail should reproduce the bytes.
    use base64::Engine as _;
    let payload = url.trim_start_matches("data:image/svg+xml;base64,");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .expect("decode");
    assert_eq!(decoded, std::fs::read(&path).unwrap());
}

#[test]
fn catalog_entry_projects_account_config_through_schema() {
    let mut entry = SubprocessPluginCatalogEntry {
        id: "schema-bound".to_owned(),
        display_name: "Schema Bound".to_owned(),
        version: "0.1.0".to_owned(),
        description: "test".to_owned(),
        state: PluginState::Ready,
        last_error: None,
        capabilities: crate::builtin_catalog::builtin_capabilities(true, true, false),
        schema: json!({
            "type": "object",
            "properties": {
                "token": { "type": "string" },
                "nested": {
                    "type": "object",
                    "properties": {
                        "keep": { "type": "boolean" }
                    }
                },
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" }
                        }
                    }
                }
            }
        }),
        auth_flows: Vec::new(),
        config_methods: Vec::new(),
        accounts: vec![AccountDescriptor {
            id: "main".to_owned(),
            enabled: true,
            config: json!({
                "token": "secret",
                "legacy_policy": "open",
                "nested": {
                    "keep": true,
                    "drop": true
                },
                "items": [
                    { "name": "one", "drop": "x" }
                ]
            }),
        }],
        icon_data_url: None,
        account_root_behavior: AccountRootBehavior::OpenDefault,
    };

    entry.project_account_configs_through_schema();

    assert_eq!(
        entry.accounts[0].config,
        json!({
            "token": "secret",
            "nested": { "keep": true },
            "items": [
                { "name": "one" }
            ]
        })
    );
}

#[test]
fn read_icon_as_data_url_picks_media_type_by_extension() {
    let dir = tempfile::tempdir().unwrap();
    // PNG magic bytes so even a sniffer would agree — but the
    // function only looks at the extension, which is the part
    // we want pinned.
    std::fs::write(
        dir.path().join("icon.png"),
        [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
    )
    .unwrap();
    let url = read_icon_as_data_url(&dir.path().join("icon.png")).unwrap();
    assert!(url.starts_with("data:image/png;base64,"));

    std::fs::write(dir.path().join("icon.webp"), b"fake webp").unwrap();
    let webp = read_icon_as_data_url(&dir.path().join("icon.webp")).unwrap();
    assert!(webp.starts_with("data:image/webp;base64,"));
}

#[test]
fn read_icon_as_data_url_returns_none_for_missing_file() {
    // Catalog builders call this in hot paths — a missing icon
    // must degrade to "no icon" rather than panic or error.
    let result = read_icon_as_data_url(std::path::Path::new("/does/not/exist.svg"));
    assert!(result.is_none());
}

#[test]
fn read_icon_as_data_url_rejects_unknown_extension() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("icon.bmp");
    std::fs::write(&path, b"whatever").unwrap();
    assert!(read_icon_as_data_url(&path).is_none());
}

#[test]
fn read_icon_as_data_url_enforces_size_cap() {
    // A plugin shouldn't be able to bloat the catalog payload
    // with a 10 MB icon. The cap is 1 MB; we verify with an
    // obviously-too-large file so a future cap adjustment
    // doesn't silently break the test.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("icon.png");
    let oversize = vec![0u8; (MAX_ICON_BYTES + 1) as usize];
    std::fs::write(&path, &oversize).unwrap();
    assert!(
        read_icon_as_data_url(&path).is_none(),
        "catalog must drop oversized icons rather than embed them"
    );
}

#[test]
fn resolve_plugin_icon_path_rejects_parent_traversal() {
    let manifest_dir = std::path::Path::new("/plugins/sample");
    assert!(resolve_plugin_icon_path(manifest_dir, "./icon.svg").is_some());
    assert!(resolve_plugin_icon_path(manifest_dir, "sub/icon.svg").is_some());
    // `..` traversal MUST be blocked even if the final absolute
    // path would happen to still be valid.
    assert!(resolve_plugin_icon_path(manifest_dir, "../icon.svg").is_none());
    assert!(resolve_plugin_icon_path(manifest_dir, "a/../../b/icon.svg").is_none());
}

#[test]
fn resolve_plugin_icon_path_rejects_absolute_paths() {
    let manifest_dir = std::path::Path::new("/plugins/sample");
    assert!(resolve_plugin_icon_path(manifest_dir, "/etc/passwd").is_none());
    #[cfg(unix)]
    assert!(resolve_plugin_icon_path(manifest_dir, "/tmp/icon.svg").is_none());
}

struct TestPlugin {
    meta: PluginMetadata,
    fail_start: bool,
    starts: Arc<AtomicUsize>,
    stops: Arc<AtomicUsize>,
}

#[async_trait]
impl PluginLifecycle for TestPlugin {
    async fn initialize(&self) -> Result<(), String> {
        Ok(())
    }

    async fn start(&self) -> Result<(), String> {
        self.starts.fetch_add(1, Ordering::Relaxed);
        if self.fail_start {
            Err("boom".to_owned())
        } else {
            Ok(())
        }
    }

    async fn stop(&self) -> Result<(), String> {
        self.stops.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn cleanup(&self) -> Result<(), String> {
        Ok(())
    }
}

impl ChannelPlugin for TestPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.meta
    }
}

#[test]
fn registry_rejects_duplicate_alias() {
    let mut registry = PluginRegistry::default();
    let p1 = PluginMetadata {
        id: "a".to_owned(),
        aliases: vec!["shared".to_owned()],
        display_name: "A".to_owned(),
        version: "1".to_owned(),
        description: String::new(),
        source: "test".to_owned(),
        config_methods: Vec::new(),
    };
    let p2 = PluginMetadata {
        id: "b".to_owned(),
        aliases: vec!["shared".to_owned()],
        display_name: "B".to_owned(),
        version: "1".to_owned(),
        description: String::new(),
        source: "test".to_owned(),
        config_methods: Vec::new(),
    };
    assert!(registry.register(&p1).is_ok());
    assert!(registry.register(&p2).is_err());
}

#[test]
fn registry_multi_alias_rollback_is_transactional() {
    // If a later alias collides, EVERY earlier alias from the same
    // attempt MUST be rolled back, along with the id insert.
    // Without this, a partial commit would leave ghost aliases in
    // the map that no subsequent unregister could remove (they
    // point at a failed id that never finished registering).
    let mut registry = PluginRegistry::default();
    let existing = PluginMetadata {
        id: "a".to_owned(),
        aliases: vec!["c".to_owned()],
        display_name: "A".to_owned(),
        version: "1".to_owned(),
        description: String::new(),
        source: "test".to_owned(),
        config_methods: Vec::new(),
    };
    registry.register(&existing).unwrap();

    let candidate = PluginMetadata {
        id: "b".to_owned(),
        // `first` should get inserted, then `c` collides with
        // existing → rollback must remove `first` AND the id `b`.
        aliases: vec!["first".to_owned(), "c".to_owned()],
        display_name: "B".to_owned(),
        version: "1".to_owned(),
        description: String::new(),
        source: "test".to_owned(),
        config_methods: Vec::new(),
    };
    let err = registry
        .register(&candidate)
        .expect_err("colliding alias must reject");
    assert!(matches!(err, PluginRegistryError::DuplicateAlias { .. }));

    // Retry under the same id with a non-colliding alias set must
    // succeed — proves id was released.
    let retry = PluginMetadata {
        id: "b".to_owned(),
        aliases: vec!["first".to_owned()],
        ..candidate
    };
    registry
        .register(&retry)
        .expect("retry should succeed after transactional rollback");
    // And the surviving alias/id set is exactly what retry wrote:
    // existing owns `c`, retry owns `b` + `first`. No ghosts.
    assert_eq!(
        registry.alias_to_id.get("first").map(String::as_str),
        Some("b"),
    );
    assert_eq!(registry.alias_to_id.get("c").map(String::as_str), Some("a"),);
    assert!(registry.ids.contains("a"));
    assert!(registry.ids.contains("b"));
}

#[tokio::test]
async fn manager_isolates_start_failures() {
    let mut manager = ChannelPluginManager::new();
    let starts_ok = Arc::new(AtomicUsize::new(0));
    let starts_fail = Arc::new(AtomicUsize::new(0));
    let stops = Arc::new(AtomicUsize::new(0));

    manager
        .register_plugin(Box::new(TestPlugin {
            meta: PluginMetadata {
                id: "ok".to_owned(),
                aliases: vec![],
                display_name: "ok".to_owned(),
                version: "1".to_owned(),
                description: String::new(),
                source: "test".to_owned(),
                config_methods: Vec::new(),
            },
            fail_start: false,
            starts: starts_ok.clone(),
            stops: stops.clone(),
        }))
        .unwrap();
    manager
        .register_plugin(Box::new(TestPlugin {
            meta: PluginMetadata {
                id: "fail".to_owned(),
                aliases: vec![],
                display_name: "fail".to_owned(),
                version: "1".to_owned(),
                description: String::new(),
                source: "test".to_owned(),
                config_methods: Vec::new(),
            },
            fail_start: true,
            starts: starts_fail.clone(),
            stops: stops.clone(),
        }))
        .unwrap();

    manager.start_all().await;
    let statuses = manager.statuses();
    assert!(
        statuses
            .iter()
            .any(|s| s.metadata.id == "ok" && s.state == PluginState::Running)
    );
    assert!(
        statuses
            .iter()
            .any(|s| s.metadata.id == "fail" && s.state == PluginState::Error)
    );
    assert_eq!(starts_ok.load(Ordering::Relaxed), 1);
    assert_eq!(starts_fail.load(Ordering::Relaxed), 1);

    manager.stop_all().await;
    assert!(stops.load(Ordering::Relaxed) >= 2);
}

#[tokio::test]
async fn builtin_discoverer_discovers_enabled_channels() {
    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "bot1",
        TelegramAccount {
            token: "fake".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    insert_feishu_plugin_account(
        &mut config,
        "bot2",
        FeishuAccount {
            app_id: "a".to_owned(),
            app_secret: "b".to_owned(),
            enabled: true,
            domain: Default::default(),
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: Default::default(),
        },
    );
    insert_weixin_plugin_account(
        &mut config,
        "bot3",
        WeixinAccount {
            token: "token".to_owned(),
            uin: "MTIz".to_owned(),
            enabled: true,
            base_url: "https://ilinkai.weixin.qq.com".to_owned(),
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
        },
    );

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let router = Arc::new(Mutex::new(MessageRouter::new(store, config.clone())));
    let bridge = Arc::new(MultiProviderBridge::new());
    let discoverer =
        BuiltInPluginDiscoverer::new(config.channels.clone(), router, bridge, String::new());

    let discovered = discoverer.discover().unwrap();
    assert_eq!(discovered.len(), 3);
}

#[test]
fn builtin_plugin_metadata_comes_from_shared_descriptor_catalog() {
    for descriptor in crate::builtin_catalog::builtin_channel_descriptors() {
        let metadata =
            builtin_plugin_metadata(descriptor.id).expect("builtin plugin metadata exists");
        assert_eq!(metadata.id, descriptor.id);
        assert_eq!(
            metadata.aliases,
            descriptor
                .aliases
                .iter()
                .map(|alias| (*alias).to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(metadata.display_name, descriptor.display_name);
        assert_eq!(metadata.description, descriptor.description);
        assert_eq!(metadata.config_methods, descriptor.config_methods());
    }
}

#[tokio::test]
async fn builtin_discoverer_sets_config_methods_per_channel() {
    // Pin the per-channel `config_methods` contract. The Mac App
    // reads this array verbatim to decide whether to render an
    // auto-login button next to the schema form.
    //   - telegram: form only (bot-token copy-paste, no SSO).
    //   - feishu:   form + auto_login (device-code OAuth).
    //   - weixin:   form + auto_login (QR-code login).
    use crate::auth_flow::ConfigMethod;

    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "bot1",
        TelegramAccount {
            token: "fake".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    insert_feishu_plugin_account(
        &mut config,
        "bot2",
        FeishuAccount {
            app_id: "a".to_owned(),
            app_secret: "b".to_owned(),
            enabled: true,
            domain: Default::default(),
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: Default::default(),
        },
    );
    insert_weixin_plugin_account(
        &mut config,
        "bot3",
        WeixinAccount {
            token: "token".to_owned(),
            uin: "MTIz".to_owned(),
            enabled: true,
            base_url: "https://ilinkai.weixin.qq.com".to_owned(),
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
        },
    );

    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let router = Arc::new(Mutex::new(MessageRouter::new(store, config.clone())));
    let bridge = Arc::new(MultiProviderBridge::new());
    let discoverer =
        BuiltInPluginDiscoverer::new(config.channels.clone(), router, bridge, String::new());
    let discovered = discoverer.discover().unwrap();

    let by_id = |id: &str| {
        discovered
            .iter()
            .find(|p| p.metadata().id == id)
            .map(|p| p.metadata().config_methods.clone())
            .unwrap_or_else(|| panic!("missing plugin {id}"))
    };

    assert_eq!(by_id("telegram"), vec![ConfigMethod::Form]);
    assert_eq!(
        by_id("feishu"),
        vec![ConfigMethod::Form, ConfigMethod::AutoLogin]
    );
    assert_eq!(
        by_id("weixin"),
        vec![ConfigMethod::Form, ConfigMethod::AutoLogin]
    );

    // Pin the auth-flow executor contract alongside the catalog
    // contract: Telegram has no auto-login path, Feishu and
    // Weixin each expose one. This is how the gateway decides
    // at dispatch time whether it can satisfy a `/auth_flow/start`
    // request — `metadata().config_methods` is the UI hint, the
    // actual executor is the enforceable truth.
    let mut manager = ChannelPluginManager::new();
    for plugin in discovered {
        manager.register_plugin(plugin).unwrap();
    }
    assert!(
        manager.auth_flow_executor("telegram").is_none(),
        "Telegram advertises Form only and must not expose an executor"
    );
    assert!(
        manager.auth_flow_executor("feishu").is_some(),
        "Feishu's AutoLogin claim must be backed by a real executor"
    );
    // Alias resolution: `lark` → feishu. The gateway accepts
    // whichever id the UI ships, so the manager MUST accept the
    // alias too or users hit "plugin not found" even when the
    // catalog said it existed.
    assert!(
        manager.auth_flow_executor("lark").is_some(),
        "feishu alias `lark` must resolve to the same executor"
    );
    assert!(
        manager.auth_flow_executor("weixin").is_some(),
        "Weixin's AutoLogin claim must be backed by a real executor"
    );
    assert!(
        manager.auth_flow_executor("does-not-exist").is_none(),
        "unknown ids must surface as None, not panic"
    );
}

/// `reload_builtin_senders` picks up a new account added via
/// runtime-config update without rebuilding the plugin. Before
/// reload: `dispatch_outbound("new_account")` fails with "not
/// registered". After reload: the lookup finds the sender and
/// the dispatch actually reaches the network (surfaces as a
/// transport error, NOT as the routing-failure Config message).
/// Pins the §9.4 Codex P1 fix.
#[tokio::test]
async fn reload_builtin_senders_picks_up_new_account() {
    use crate::dispatcher::OutboundMessage;
    let mut config = GaryxConfig::default();
    // Seed one account so BuiltInPluginDiscoverer registers the
    // telegram plugin at all — an enabled-count-of-zero would
    // otherwise skip the plugin entirely.
    insert_telegram_plugin_account(
        &mut config,
        "seed",
        TelegramAccount {
            token: "seed-token".into(),
            enabled: true,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let router = Arc::new(Mutex::new(MessageRouter::new(store, config.clone())));
    let bridge = Arc::new(MultiProviderBridge::new());
    let discoverer =
        BuiltInPluginDiscoverer::new(config.channels.clone(), router, bridge, String::new());
    let mut manager = ChannelPluginManager::new();
    for plugin in discoverer.discover().unwrap() {
        manager.register_plugin(plugin).unwrap();
    }

    // Before reload: `new_account` is not in any sender map.
    let msg_for = |id: &str| OutboundMessage {
        channel: "telegram".into(),
        account_id: id.into(),
        chat_id: "123".into(),
        delivery_target_type: "chat_id".into(),
        delivery_target_id: "123".into(),
        content: ChannelOutboundContent::text("x"),
        reply_to: None,
        thread_id: None,
    };
    let entry = manager
        .plugins
        .get("telegram")
        .expect("telegram registered");
    let before = entry
        .plugin
        .dispatch_outbound(msg_for("new_account"))
        .await
        .expect_err("new_account must be unknown before reload");
    match before {
        ChannelError::Config(msg) => {
            assert!(
                msg.contains("not registered"),
                "unexpected pre-reload error: {msg}",
            );
        }
        other => panic!("expected Config error before reload, got {other:?}"),
    }

    // Simulate the gateway's apply_runtime_config flow: new
    // account is added to the config, then reload_builtin_senders
    // is called.
    insert_telegram_plugin_account(
        &mut config,
        "new_account",
        TelegramAccount {
            token: "new-token".into(),
            enabled: true,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    manager.reload_builtin_senders(&config.channels);

    // After reload: `new_account` resolves; the error (if any)
    // must come from the network / HTTP layer, not the routing
    // lookup.
    let after = entry.plugin.dispatch_outbound(msg_for("new_account")).await;
    match after {
        Ok(_) => {} // unexpected but fine — test passed
        Err(ChannelError::Config(msg)) => {
            assert!(
                !msg.contains("not registered"),
                "post-reload still sees 'not registered': {msg}"
            );
        }
        Err(_) => {
            // Network / transport error — exactly what we want.
        }
    }
}

/// Trait-level `dispatch_outbound` works end-to-end for built-in
/// channels. The `telegram` plugin picks an `OutboundSender` out
/// of its internal per-account map and delegates — no
/// `SwappableDispatcher` in the loop. We can't actually hit the
/// real Telegram API in a unit test, so we verify the *routing*:
///   - known account: `dispatch_outbound` reaches the sender
///     (surfaces as a transport error trying to hit
///     `api.telegram.org`, NOT as "account not registered")
///   - unknown account: clean `Config("… not registered")` error
///     instead of falling through to Unsupported.
/// Ensures the trait path is the one that's wired, without
/// depending on network I/O for the happy case.
#[tokio::test]
async fn managed_channel_plugin_dispatch_outbound_routes_by_account() {
    use crate::dispatcher::OutboundMessage;
    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "bot_main",
        TelegramAccount {
            token: "fake-token".into(),
            enabled: true,
            name: None,
            agent_id: "claude".into(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let router = Arc::new(Mutex::new(MessageRouter::new(store, config.clone())));
    let bridge = Arc::new(MultiProviderBridge::new());
    let discoverer =
        BuiltInPluginDiscoverer::new(config.channels.clone(), router, bridge, String::new());
    let plugins = discoverer.discover().unwrap();
    let telegram = plugins
        .iter()
        .find(|p| p.metadata().id == "telegram")
        .unwrap();

    let msg = OutboundMessage {
        channel: "telegram".into(),
        account_id: "bot_main".into(),
        chat_id: "123".into(),
        delivery_target_type: "chat_id".into(),
        delivery_target_id: "123".into(),
        content: ChannelOutboundContent::text("hello"),
        reply_to: None,
        thread_id: None,
    };
    let result = telegram.dispatch_outbound(msg).await;
    // Won't succeed — the token is fake and we'd hit the real
    // Telegram API. But it MUST NOT be the "no outbound_senders
    // map" or "account not registered" Config error — those
    // would mean routing didn't reach the sender. Any SendFailed
    // (HTTP layer) or Connection error is fine for the test's
    // purpose.
    match result {
        Ok(_) => {}
        Err(ChannelError::Config(msg)) => {
            assert!(
                !msg.contains("no outbound_senders") && !msg.contains("not registered"),
                "routing fell back to unsupported/unregistered instead of reaching the sender: {msg}"
            );
        }
        Err(_) => {
            // Any non-Config error (SendFailed / Connection /
            // etc.) means the sender ran and hit the network —
            // exactly what we want in a unit test.
        }
    }

    // Unknown account: clean "not registered" error, NOT
    // Unsupported. Proves the lookup path is active.
    let bad = OutboundMessage {
        channel: "telegram".into(),
        account_id: "no-such-account".into(),
        chat_id: "123".into(),
        delivery_target_type: "chat_id".into(),
        delivery_target_id: "123".into(),
        content: ChannelOutboundContent::text("x"),
        reply_to: None,
        thread_id: None,
    };
    let err = telegram
        .dispatch_outbound(bad)
        .await
        .expect_err("unknown account");
    match err {
        ChannelError::Config(msg) => {
            assert!(
                msg.contains("not registered"),
                "expected 'not registered' error for unknown account, got: {msg}"
            );
        }
        other => panic!("expected Config error for unknown account, got {other:?}"),
    }
}
