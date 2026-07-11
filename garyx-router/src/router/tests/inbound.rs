use super::*;
use garyx_models::command_catalog::{
    CommandCatalogOptions, CommandKind, CommandSource, CommandSurface,
};
use garyx_models::config::SlashCommand;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_is_native_command_text_recognizes_thread_commands() {
    assert!(is_native_command_text("/threads", "telegram"));
    assert!(is_native_command_text("/newthread", "telegram"));
    assert!(is_native_command_text("/bindthread 1", "telegram"));
    assert!(is_native_command_text("/threadprev", "telegram"));
    assert!(is_native_command_text("/threadnext", "telegram"));
    assert!(!is_native_command_text("/loop", "telegram"));
    assert!(!is_native_command_text(
        "/goal ship the feature",
        "telegram"
    ));
}

#[test]
fn test_start_is_not_a_native_command() {
    assert!(!is_native_command_text("/start", "telegram"));
    assert!(!is_native_command_text("/start", "feishu"));
    assert!(!is_native_command_text("hello", "telegram"));
}

#[test]
fn test_inbound_command_classifier_reads_metadata_text() {
    let mut extra_metadata = HashMap::new();
    extra_metadata.insert(
        NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        json!("/threads"),
    );

    let request = InboundRequest {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        from_id: "user1".to_owned(),
        is_group: false,
        thread_binding_key: "user1".to_owned(),
        message: "ignored".to_owned(),
        run_id: "run-1".to_owned(),
        reply_to_message_id: None,
        images: Vec::new(),
        extra_metadata,
        file_paths: Vec::new(),
    };

    let command_text =
        crate::router::inbound::InboundCommandClassifier::command_text(&request).unwrap();
    assert_eq!(command_text, "/threads");
}

#[test]
fn test_channel_native_catalog_exposes_telegram_menu_commands() {
    let catalog = crate::command_catalog_for_config(
        &GaryxConfig::default(),
        CommandCatalogOptions {
            surface: Some(CommandSurface::Telegram),
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            include_hidden: false,
        },
    );

    let names = catalog
        .commands
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["newthread", "threads", "bindthread"]);
    assert_eq!(
        catalog.commands[1].args_hint.as_deref(),
        Some("[page|next|prev]")
    );
    assert_eq!(catalog.commands[2].args_hint.as_deref(), Some("<n>"));
    assert!(
        catalog
            .commands
            .iter()
            .all(|entry| entry.kind == CommandKind::ChannelNative)
    );
    assert!(
        catalog
            .commands
            .iter()
            .all(|entry| entry.source == CommandSource::Builtin)
    );
    assert!(
        catalog
            .commands
            .iter()
            .all(|entry| entry.surfaces.contains(&CommandSurface::Telegram))
    );

    let hidden = crate::command_catalog_for_config(
        &GaryxConfig::default(),
        CommandCatalogOptions {
            surface: Some(CommandSurface::Telegram),
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            include_hidden: true,
        },
    );
    assert!(hidden.commands.iter().any(|entry| {
        entry.name == "threadprev"
            && entry.visibility == garyx_models::command_catalog::CommandVisibility::Hidden
    }));
}

#[test]
fn test_default_command_catalog_exposes_only_shortcuts() {
    let mut config = GaryxConfig::default();
    config.commands.push(SlashCommand {
        name: "summary".to_owned(),
        description: "Summarize the active thread".to_owned(),
        prompt: Some("Please summarize the active thread.".to_owned()),
        skill_id: None,
    });

    let catalog = crate::command_catalog_for_config(&config, CommandCatalogOptions::default());
    let names = catalog
        .commands
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["summary"]);
    assert!(
        catalog
            .commands
            .iter()
            .all(|entry| entry.kind == CommandKind::Shortcut)
    );
    assert!(
        catalog
            .commands
            .iter()
            .all(|entry| entry.source == CommandSource::Config)
    );
}

#[test]
fn test_command_catalog_allows_former_loop_goal_shortcuts() {
    let mut config = GaryxConfig::default();
    config.commands.push(SlashCommand {
        name: "loop".to_owned(),
        description: "Custom loop".to_owned(),
        prompt: Some("custom loop prompt".to_owned()),
        skill_id: None,
    });
    config.commands.push(SlashCommand {
        name: "goal".to_owned(),
        description: "Custom goal".to_owned(),
        prompt: Some("custom goal prompt".to_owned()),
        skill_id: None,
    });

    let catalog = crate::command_catalog_for_config(
        &config,
        CommandCatalogOptions {
            surface: Some(CommandSurface::Telegram),
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            include_hidden: true,
        },
    );

    let loop_entry = catalog
        .commands
        .iter()
        .find(|entry| entry.name == "loop")
        .expect("custom /loop shortcut");
    assert_eq!(loop_entry.kind, CommandKind::Shortcut);
    assert_eq!(loop_entry.source, CommandSource::Config);

    let goal_entry = catalog
        .commands
        .iter()
        .find(|entry| entry.name == "goal")
        .expect("custom /goal shortcut");
    assert_eq!(goal_entry.kind, CommandKind::Shortcut);
    assert_eq!(goal_entry.source, CommandSource::Config);

    assert!(
        catalog
            .warnings
            .iter()
            .all(|warning| warning.code != "reserved_command_name")
    );
}
