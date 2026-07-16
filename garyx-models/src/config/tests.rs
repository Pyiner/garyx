use super::*;

#[test]
fn test_default_config_roundtrip() {
    let cfg = GaryxConfig::default();
    let json = serde_json::to_value(&cfg).unwrap();
    let _back: GaryxConfig = serde_json::from_value(json).unwrap();
}

#[test]
fn test_gateway_defaults() {
    let gw = GatewayConfig::default();
    assert_eq!(gw.port, 31337);
    assert_eq!(gw.host, "0.0.0.0");
    assert_eq!(gw.meetings.join_retry_window_secs, 300);
    assert_eq!(gw.meetings.read_page_bytes, 65_536);
    assert_eq!(gw.meetings.effective_join_retry_window_secs(), 300);
    assert_eq!(gw.meetings.effective_read_page_bytes(), 65_536);
}

#[test]
fn meeting_config_is_serde_defaulted_and_clamps_runtime_floors() {
    let config: GaryxConfig = serde_json::from_value(serde_json::json!({
        "gateway": {
            "meetings": {
                "join_retry_window_secs": 0,
                "read_page_bytes": 1
            }
        }
    }))
    .expect("config");
    assert_eq!(config.gateway.meetings.join_retry_window_secs, 0);
    assert_eq!(
        config.gateway.meetings.effective_join_retry_window_secs(),
        1
    );
    assert_eq!(config.gateway.meetings.effective_read_page_bytes(), 4_096);
}

#[test]
fn test_agent_provider_defaults() {
    let ap = AgentProviderConfig::default();
    assert_eq!(ap.provider_type, "claude_code");
    assert_eq!(ap.claude_cli_mode, "native");
    assert_eq!(ap.max_turns, None);
    assert_eq!(ap.permission_mode, "bypassPermissions");
    assert_eq!(ap.default_model, "");
    assert_eq!(ap.model, "");
}

#[test]
fn test_api_channels_default_empty() {
    let cfg = GaryxConfig::default();
    assert!(cfg.channels.api.accounts.is_empty());
}

#[test]
fn test_default_config_has_empty_commands_and_mcp_servers() {
    let cfg = GaryxConfig::default();
    assert!(cfg.commands.is_empty());
    assert!(cfg.mcp_servers.is_empty());
}

#[test]
fn test_resolve_slash_command_from_config() {
    let mut cfg = GaryxConfig::default();
    cfg.commands.push(SlashCommand {
        name: "summary".to_owned(),
        description: "Summarize the thread".to_owned(),
        prompt: Some("Please summarize the conversation".to_owned()),
        skill_id: Some("summary-skill".to_owned()),
    });

    let resolved = cfg.resolve_slash_command("/summary@garyx_bot").unwrap();
    assert_eq!(resolved.name, "summary");
    assert_eq!(
        resolved.prompt.as_deref(),
        Some("Please summarize the conversation")
    );
    assert_eq!(resolved.skill_id.as_deref(), Some("summary-skill"));
}

#[test]
fn test_command_catalog_serializes_target_shape() {
    use crate::command_catalog::{
        CommandCatalog, CommandCatalogEntry, CommandDispatch, CommandKind, CommandSource,
        CommandSurface, CommandVisibility,
    };

    let catalog = CommandCatalog::from_entries(vec![CommandCatalogEntry {
        id: "builtin.router.newthread".to_owned(),
        name: "newthread".to_owned(),
        slash: "/newthread".to_owned(),
        aliases: Vec::new(),
        description: "Start a new thread".to_owned(),
        category: "Thread".to_owned(),
        args_hint: None,
        kind: CommandKind::ChannelNative,
        source: CommandSource::Builtin,
        surfaces: vec![CommandSurface::Plugin, CommandSurface::Telegram],
        dispatch: CommandDispatch::RouterNative {
            key: "router.native.newthread".to_owned(),
        },
        visibility: CommandVisibility::Visible,
        warnings: Vec::new(),
    }]);

    let value = serde_json::to_value(&catalog).unwrap();
    assert_eq!(value["version"], 1);
    assert!(value["revision"].as_str().unwrap().starts_with("v1:"));
    assert_eq!(value["commands"][0]["source"], "builtin");
    assert_eq!(value["commands"][0]["kind"], "channel_native");
    assert_eq!(value["commands"][0]["surfaces"][1], "telegram");
    assert_eq!(value["commands"][0]["dispatch"]["type"], "router_native");
    assert_eq!(
        value["commands"][0]["dispatch"]["key"],
        "router.native.newthread"
    );
}

#[test]
fn test_api_channels_deserialize() {
    let value = serde_json::json!({
        "channels": {
            "api": {
                "accounts": {
                    "main": {
                        "enabled": true,
                        "name": "API Bot",
                        "agent_id": "codex",
                        "workspace_dir": "/tmp/codex-workspace",
                        "workspace_mode": "worktree"
                    }
                }
            }
        }
    });
    let cfg: GaryxConfig = serde_json::from_value(value).unwrap();
    let account = cfg.channels.api.accounts.get("main").unwrap();
    assert!(account.enabled);
    assert_eq!(account.name.as_deref(), Some("API Bot"));
    assert_eq!(account.agent_id.as_deref(), Some("codex"));
    assert_eq!(
        account.workspace_dir.as_deref(),
        Some("/tmp/codex-workspace")
    );
    assert_eq!(account.workspace_mode.as_deref(), Some("worktree"));
}

#[test]
fn test_channels_serialize_flattens_channel_entries() {
    let mut cfg = GaryxConfig::default();
    cfg.channels.plugins.insert(
        "telegram".to_owned(),
        PluginChannelConfig {
            accounts: HashMap::from([(
                "main".to_owned(),
                telegram_account_to_plugin_entry(&TelegramAccount {
                    token: "telegram-token".to_owned(),
                    enabled: true,
                    name: Some("Telegram".to_owned()),
                    agent_id: Some("claude".to_owned()),
                    workspace_dir: None,
                    owner_target: None,
                    groups: HashMap::new(),
                }),
            )]),
        },
    );

    let value = serde_json::to_value(&cfg).unwrap();
    assert!(value["channels"].get("plugins").is_none());
    assert!(value["channels"].get("telegram").is_some());
}

#[test]
fn test_builtin_channels_deserialize_from_flattened_shape() {
    let value = serde_json::json!({
        "channels": {
            "weixin": {
                "accounts": {
                    "main": {
                        "enabled": true,
                        "config": {
                            "token": "wx-token",
                            "uin": "MTIz"
                        }
                    }
                }
            }
        }
    });
    let cfg: GaryxConfig = serde_json::from_value(value).unwrap();
    let resolved = cfg.channels.resolved_weixin_config().unwrap();
    let account = resolved.accounts.get("main").unwrap();
    assert_eq!(account.token, "wx-token");
    assert_eq!(account.uin, "MTIz");
}

#[test]
fn test_discord_channel_deserialize_from_flattened_shape() {
    let value = serde_json::json!({
        "channels": {
            "discord": {
                "accounts": {
                    "main": {
                        "enabled": true,
                        "name": "Discord",
                        "agent_id": "codex",
                        "workspace_dir": "/tmp/test-workspace",
                        "config": {
                            "token": "discord-token"
                        }
                    }
                }
            }
        }
    });
    let cfg: GaryxConfig = serde_json::from_value(value).unwrap();
    let resolved = cfg.channels.resolved_discord_config().unwrap();
    let account = resolved.accounts.get("main").unwrap();
    assert_eq!(account.token, "discord-token");
    assert_eq!(account.name.as_deref(), Some("Discord"));
    assert_eq!(account.agent_id.as_deref(), Some("codex"));
    assert_eq!(
        account.workspace_dir.as_deref(),
        Some("/tmp/test-workspace")
    );
}

#[test]
fn test_flattened_channel_entries_override_legacy_plugins_bucket() {
    let value = serde_json::json!({
        "channels": {
            "plugins": {
                "telegram": {
                    "accounts": {
                        "main": {
                            "enabled": true,
                            "config": { "token": "legacy-token" }
                        }
                    }
                }
            },
            "telegram": {
                "accounts": {
                    "main": {
                        "enabled": true,
                        "config": { "token": "new-token" }
                    }
                }
            }
        }
    });

    let cfg: GaryxConfig = serde_json::from_value(value).unwrap();
    let resolved = cfg.channels.resolved_telegram_config().unwrap();
    let account = resolved.accounts.get("main").unwrap();
    assert_eq!(account.token, "new-token");
}

#[test]
fn test_feishu_legacy_secret_fields_are_ignored_on_load_and_serialize() {
    let value = serde_json::json!({
        "channels": {
            "plugins": {
                "feishu": {
                    "accounts": {
                        "work": {
                            "enabled": true,
                            "config": {
                                "app_id": "cli_app",
                                "app_secret": "top-secret",
                                "verification_token": "legacy-verification",
                                "encrypt_key": "legacy-encrypt"
                            }
                        }
                    }
                }
            }
        }
    });

    let cfg: GaryxConfig = serde_json::from_value(value).unwrap();
    let resolved = cfg.channels.resolved_feishu_config().unwrap();
    let account = resolved.accounts.get("work").unwrap();
    assert_eq!(account.app_id, "cli_app");
    assert_eq!(account.app_secret, "top-secret");
    assert!(account.meeting_entities);

    let serialized = serde_json::to_value(account).unwrap();
    assert!(serialized.get("verification_token").is_none());
    assert!(serialized.get("encrypt_key").is_none());
}

#[test]
fn test_weixin_channels_deserialize() {
    let value = serde_json::json!({
        "channels": {
            "plugins": {
                "weixin": {
                    "accounts": {
                        "main": {
                            "enabled": true,
                            "config": {
                                "token": "wx-token",
                                "uin": "MTIz"
                            }
                        }
                    }
                }
            }
        }
    });
    let cfg: GaryxConfig = serde_json::from_value(value).unwrap();
    let resolved = cfg.channels.resolved_weixin_config().unwrap();
    let account = resolved.accounts.get("main").unwrap();
    assert_eq!(account.token, "wx-token");
    assert_eq!(account.uin, "MTIz");
    assert_eq!(account.base_url, "https://ilinkai.weixin.qq.com");
    assert!(account.enabled);
}

#[test]
fn telegram_topic_default_matches_serde_defaults() {
    let from_default = TelegramTopicConfig::default();
    let from_serde: TelegramTopicConfig = serde_json::from_value(serde_json::json!({})).unwrap();

    assert!(from_default.enabled);
    assert_eq!(from_default.enabled, from_serde.enabled);
    assert_eq!(from_default.require_mention, from_serde.require_mention);
    assert_eq!(from_default.allow_from, from_serde.allow_from);
    assert_eq!(from_default.system_prompt, from_serde.system_prompt);
}

#[test]
fn api_account_default_matches_serde_defaults() {
    let from_default = ApiAccount::default();
    let from_serde: ApiAccount = serde_json::from_value(serde_json::json!({})).unwrap();

    assert!(from_default.enabled);
    assert_eq!(from_default.enabled, from_serde.enabled);
    assert_eq!(from_default.name, from_serde.name);
    assert_eq!(from_default.agent_id, from_serde.agent_id);
    assert_eq!(from_default.workspace_dir, from_serde.workspace_dir);
    assert_eq!(from_default.workspace_mode, from_serde.workspace_mode);
}

#[test]
fn channel_account_agent_id_preserves_none_and_explicit_claude_across_all_five_types() {
    fn assert_agent_state<T>(base: serde_json::Value)
    where
        T: serde::de::DeserializeOwned + serde::Serialize,
    {
        let missing: T = serde_json::from_value(base.clone()).expect("missing agent_id");
        let missing_json = serde_json::to_value(&missing).expect("serialize missing agent_id");
        assert!(missing_json.get("agent_id").is_none());

        let mut explicit = base;
        explicit
            .as_object_mut()
            .unwrap()
            .insert("agent_id".to_owned(), serde_json::json!("claude"));
        let explicit: T = serde_json::from_value(explicit).expect("explicit claude");
        let explicit_json = serde_json::to_value(&explicit).expect("serialize explicit claude");
        assert_eq!(explicit_json["agent_id"], "claude");
    }

    assert_agent_state::<ApiAccount>(serde_json::json!({}));
    assert_agent_state::<TelegramAccount>(serde_json::json!({"token": "${TOKEN}"}));
    assert_agent_state::<DiscordAccount>(serde_json::json!({"token": "${TOKEN}"}));
    assert_agent_state::<FeishuAccount>(serde_json::json!({
        "app_id": "test_app",
        "app_secret": "${APP_SECRET}"
    }));
    assert_agent_state::<WeixinAccount>(serde_json::json!({"token": "${TOKEN}"}));
}
