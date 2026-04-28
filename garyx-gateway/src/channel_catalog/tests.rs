use super::*;
use garyx_models::config::{FeishuAccount, GaryxConfig, TelegramAccount};

#[test]
fn builtin_catalog_has_one_entry_per_channel() {
    let config = GaryxConfig::default();
    let catalog = builtin_channel_catalog(&config.channels);
    assert_eq!(catalog.len(), 3);
    let ids: Vec<&str> = catalog.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"telegram"));
    assert!(ids.contains(&"feishu"));
    assert!(ids.contains(&"weixin"));
    assert!(catalog.iter().all(|entry| {
        entry
            .icon_data_url
            .as_deref()
            .is_some_and(|icon| icon.starts_with("data:image/"))
    }));
}

#[test]
fn builtin_catalog_entries_match_shared_descriptor_contract() {
    let config = GaryxConfig::default();
    let catalog = builtin_channel_catalog(&config.channels);

    for descriptor in garyx_channels::builtin_catalog::builtin_channel_descriptors() {
        let entry = catalog
            .iter()
            .find(|entry| entry.id == descriptor.id)
            .unwrap_or_else(|| panic!("missing builtin catalog entry {}", descriptor.id));
        assert_eq!(entry.display_name, descriptor.display_name);
        assert_eq!(entry.description, descriptor.description);
        assert_eq!(entry.schema, descriptor.schema());
        assert_eq!(entry.auth_flows, descriptor.auth_flows());
        assert_eq!(entry.config_methods, descriptor.config_methods());
        assert_eq!(
            entry.account_root_behavior,
            descriptor.account_root_behavior
        );
    }
}

#[test]
fn builtin_catalog_exposes_account_ids_but_honours_enabled_flag() {
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "bot_a".into(),
            garyx_models::config::telegram_account_to_plugin_entry(&TelegramAccount {
                token: "secret".into(),
                enabled: false,
                name: None,
                agent_id: "claude".into(),
                workspace_dir: None,
                owner_target: None,
                groups: Default::default(),
            }),
        );

    let catalog = builtin_channel_catalog(&config.channels);
    let telegram = catalog.iter().find(|e| e.id == "telegram").unwrap();
    assert_eq!(telegram.accounts.len(), 1);
    assert_eq!(telegram.accounts[0].id, "bot_a");
    assert!(!telegram.accounts[0].enabled, "disabled flag must survive");
    assert_eq!(
        telegram.accounts[0].config["token"], "secret",
        "token must flow through verbatim"
    );
    assert!(
        telegram.accounts[0].config.get("groups").is_none(),
        "catalog account config must only expose schema properties"
    );
}

#[test]
fn feishu_schema_declares_required_app_creds() {
    let config = GaryxConfig::default();
    let catalog = builtin_channel_catalog(&config.channels);
    let feishu = catalog.iter().find(|e| e.id == "feishu").unwrap();
    let required = &feishu.schema["required"];
    assert_eq!(required[0], "app_id");
    assert_eq!(required[1], "app_secret");
    assert_eq!(feishu.auth_flows.len(), 1);
    assert_eq!(feishu.auth_flows[0].id, "device_code");
}

#[test]
fn feishu_enum_schema_matches_rust_serialization() {
    use garyx_models::config::{FeishuDomain, TopicSessionMode};

    let config = GaryxConfig::default();
    let catalog = builtin_channel_catalog(&config.channels);
    let feishu = catalog.iter().find(|e| e.id == "feishu").unwrap();

    assert!(
        feishu.schema["properties"].get("dm_policy").is_none(),
        "feishu dm_policy should not be user-facing config"
    );
    assert!(
        feishu.schema["properties"].get("group_policy").is_none(),
        "feishu group_policy should not be user-facing config"
    );
    assert!(
        feishu.schema["properties"].get("allow_from").is_none(),
        "feishu allow_from should not be user-facing config"
    );
    assert!(
        feishu.schema["properties"]
            .get("group_allow_from")
            .is_none(),
        "feishu group_allow_from should not be user-facing config"
    );

    let expected_domain: Vec<String> = [FeishuDomain::Feishu, FeishuDomain::Lark]
        .iter()
        .map(|v| {
            serde_json::to_value(v)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let schema_domain: Vec<String> = feishu.schema["properties"]["domain"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(schema_domain, expected_domain, "feishu domain mismatch");

    let expected_topic: Vec<String> = [TopicSessionMode::Disabled, TopicSessionMode::Enabled]
        .iter()
        .map(|v| {
            serde_json::to_value(v)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let schema_topic: Vec<String> = feishu.schema["properties"]["topic_session_mode"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        schema_topic, expected_topic,
        "feishu topic_session_mode mismatch"
    );
}

#[test]
fn weixin_schema_declares_required_token_uin() {
    let config = GaryxConfig::default();
    let catalog = builtin_channel_catalog(&config.channels);
    let weixin = catalog.iter().find(|e| e.id == "weixin").unwrap();
    let required = &weixin.schema["required"];
    assert_eq!(required[0], "token");
    assert_eq!(required[1], "uin");
}

#[test]
fn schemas_round_trip_through_serde_cleanly() {
    let config = GaryxConfig::default();
    let catalog = builtin_channel_catalog(&config.channels);
    for entry in &catalog {
        let as_json = serde_json::to_value(entry).expect("catalog entry must serialize");
        assert_eq!(as_json["id"], entry.id);
        assert!(
            as_json["schema"].is_object(),
            "schema must be an object on the wire"
        );
        assert!(
            as_json["config_methods"].is_array(),
            "config_methods must serialize as an array on the wire"
        );
    }
}

#[test]
fn builtin_catalog_config_methods_match_channel_contract() {
    let config = GaryxConfig::default();
    let catalog = builtin_channel_catalog(&config.channels);

    let telegram = catalog.iter().find(|e| e.id == "telegram").unwrap();
    let feishu = catalog.iter().find(|e| e.id == "feishu").unwrap();
    let weixin = catalog.iter().find(|e| e.id == "weixin").unwrap();

    let methods =
        |entry: &SubprocessPluginCatalogEntry| serde_json::to_value(&entry.config_methods).unwrap();

    assert_eq!(methods(telegram), json!([{"kind": "form"}]));
    assert_eq!(
        methods(feishu),
        json!([{"kind": "form"}, {"kind": "auto_login"}])
    );
    assert_eq!(
        methods(weixin),
        json!([{"kind": "form"}, {"kind": "auto_login"}])
    );
}

#[test]
fn feishu_accounts_omit_hidden_owner_target_and_group_topology_from_catalog_config() {
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("feishu")
        .accounts
        .insert(
            "bot_lark".into(),
            garyx_models::config::feishu_account_to_plugin_entry(&FeishuAccount {
                app_id: "cli_x".into(),
                app_secret: "secret".into(),
                enabled: true,
                domain: Default::default(),
                name: None,
                agent_id: "claude".into(),
                workspace_dir: None,
                owner_target: Some(garyx_models::config::OwnerTargetConfig {
                    target_type: "thread".into(),
                    target_id: "thread::internal".into(),
                }),
                require_mention: true,
                topic_session_mode: Default::default(),
            }),
        );

    let catalog = builtin_channel_catalog(&config.channels);
    let feishu = catalog.iter().find(|e| e.id == "feishu").unwrap();
    let config = &feishu.accounts[0].config;
    assert_eq!(config["app_id"], "cli_x");
    assert!(config.get("owner_target").is_none());
    assert!(config.get("groups").is_none());
}
