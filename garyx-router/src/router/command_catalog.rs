use std::collections::HashSet;

use garyx_models::command_catalog::{
    CommandCatalog, CommandCatalogEntry, CommandCatalogOptions, CommandDispatch, CommandKind,
    CommandSource, CommandSurface, CommandVisibility, CommandWarning, command_name_from_text,
    is_valid_shortcut_command_name, normalize_shortcut_command_name,
};
use garyx_models::config::{GaryxConfig, SlashCommand};

use super::inbound::{NativeCommand, NativeThreadCommand};

const DISPATCH_THREADS: &str = "router.native.threads";
const DISPATCH_NEWTHREAD: &str = "router.native.newthread";
const DISPATCH_THREADPREV: &str = "router.native.threadprev";
const DISPATCH_THREADNEXT: &str = "router.native.threadnext";
const DISPATCH_LOOP: &str = "router.native.loop";

struct NativeCommandDef {
    name: &'static str,
    description: &'static str,
    category: &'static str,
    dispatch_key: &'static str,
    telegram: bool,
}

const NATIVE_COMMANDS: &[NativeCommandDef] = &[
    NativeCommandDef {
        name: "newthread",
        description: "Start a new thread",
        category: "Thread",
        dispatch_key: DISPATCH_NEWTHREAD,
        telegram: true,
    },
    NativeCommandDef {
        name: "threads",
        description: "List all threads",
        category: "Thread",
        dispatch_key: DISPATCH_THREADS,
        telegram: true,
    },
    NativeCommandDef {
        name: "threadprev",
        description: "Switch to previous thread",
        category: "Thread",
        dispatch_key: DISPATCH_THREADPREV,
        telegram: true,
    },
    NativeCommandDef {
        name: "threadnext",
        description: "Switch to next thread",
        category: "Thread",
        dispatch_key: DISPATCH_THREADNEXT,
        telegram: true,
    },
    NativeCommandDef {
        name: "loop",
        description: "Toggle loop mode for this thread",
        category: "Thread",
        dispatch_key: DISPATCH_LOOP,
        telegram: true,
    },
];

pub fn reserved_command_names() -> HashSet<&'static str> {
    NATIVE_COMMANDS.iter().map(|command| command.name).collect()
}

pub fn native_command_from_text(text: &str, channel: &str) -> Option<NativeCommand> {
    let name = command_name_from_text(text)?.to_ascii_lowercase();
    let definition = NATIVE_COMMANDS
        .iter()
        .find(|command| command.name == name.as_str())?;
    if !native_command_available_for_channel(definition, channel) {
        return None;
    }
    native_command_from_dispatch_key(definition.dispatch_key)
}

pub fn native_command_name(command: NativeCommand) -> &'static str {
    match command {
        NativeCommand::Thread(NativeThreadCommand::Threads) => "/threads",
        NativeCommand::Thread(NativeThreadCommand::New) => "/newthread",
        NativeCommand::Thread(NativeThreadCommand::ThreadPrev) => "/threadprev",
        NativeCommand::Thread(NativeThreadCommand::ThreadNext) => "/threadnext",
        NativeCommand::Loop => "/loop",
    }
}

pub fn command_catalog_for_config(
    config: &GaryxConfig,
    options: CommandCatalogOptions,
) -> CommandCatalog {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    let reserved = reserved_command_names();

    entries.extend(
        NATIVE_COMMANDS
            .iter()
            .filter(|definition| command_matches_options(definition, &options))
            .map(native_entry),
    );

    let mut seen_shortcut_names = HashSet::new();
    for command in &config.commands {
        let name = normalize_shortcut_command_name(&command.name);
        if !is_valid_shortcut_command_name(&name) {
            warnings.push(CommandWarning::new(
                "invalid_shortcut_name",
                format!("shortcut '{}' has an invalid command name", command.name),
            ));
            continue;
        }
        if reserved.contains(name.as_str()) {
            warnings.push(CommandWarning::new(
                "reserved_command_name",
                format!("shortcut '/{name}' collides with a channel-native command"),
            ));
            continue;
        }
        if !seen_shortcut_names.insert(name.clone()) {
            warnings.push(CommandWarning::new(
                "duplicate_shortcut",
                format!("duplicate shortcut '/{name}'"),
            ));
            continue;
        }
        let Some(_prompt) = command
            .prompt
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            warnings.push(CommandWarning::new(
                "invalid_shortcut_prompt",
                format!("shortcut '/{name}' must map to a prompt"),
            ));
            continue;
        };
        if let Some(entry) = shortcut_entry(command, &name, &options) {
            entries.push(entry);
        }
    }

    CommandCatalog::from_parts(entries, warnings)
}

fn native_command_available_for_channel(definition: &NativeCommandDef, channel: &str) -> bool {
    if channel == "telegram" {
        return definition.telegram;
    }
    true
}

fn native_command_from_dispatch_key(key: &str) -> Option<NativeCommand> {
    Some(match key {
        DISPATCH_THREADS => NativeCommand::Thread(NativeThreadCommand::Threads),
        DISPATCH_NEWTHREAD => NativeCommand::Thread(NativeThreadCommand::New),
        DISPATCH_THREADPREV => NativeCommand::Thread(NativeThreadCommand::ThreadPrev),
        DISPATCH_THREADNEXT => NativeCommand::Thread(NativeThreadCommand::ThreadNext),
        DISPATCH_LOOP => NativeCommand::Loop,
        _ => return None,
    })
}

fn command_matches_options(definition: &NativeCommandDef, options: &CommandCatalogOptions) -> bool {
    if let Some(channel) = options.channel.as_deref()
        && !native_command_available_for_channel(definition, channel)
    {
        return false;
    }
    match options.surface.as_ref() {
        Some(CommandSurface::Telegram) => definition.telegram,
        Some(CommandSurface::Plugin) => options
            .channel
            .as_deref()
            .map(|channel| native_command_available_for_channel(definition, channel))
            .unwrap_or(false),
        Some(CommandSurface::Router)
        | Some(CommandSurface::DesktopComposer)
        | Some(CommandSurface::ApiChat)
        | Some(CommandSurface::GatewayApi)
        | None => false,
    }
}

fn native_entry(definition: &NativeCommandDef) -> CommandCatalogEntry {
    let mut surfaces = vec![CommandSurface::Plugin];
    if definition.telegram {
        surfaces.push(CommandSurface::Telegram);
    }

    CommandCatalogEntry {
        id: format!("builtin.router.{}", definition.name),
        name: definition.name.to_owned(),
        slash: format!("/{}", definition.name),
        aliases: Vec::new(),
        description: definition.description.to_owned(),
        category: definition.category.to_owned(),
        args_hint: None,
        kind: CommandKind::ChannelNative,
        source: CommandSource::Builtin,
        surfaces,
        dispatch: CommandDispatch::RouterNative {
            key: definition.dispatch_key.to_owned(),
        },
        visibility: CommandVisibility::Visible,
        warnings: Vec::new(),
    }
}

fn shortcut_entry(
    command: &SlashCommand,
    name: &str,
    options: &CommandCatalogOptions,
) -> Option<CommandCatalogEntry> {
    let surfaces = vec![
        CommandSurface::Router,
        CommandSurface::Plugin,
        CommandSurface::Telegram,
        CommandSurface::ApiChat,
        CommandSurface::DesktopComposer,
        CommandSurface::GatewayApi,
    ];
    if let Some(surface) = options.surface.as_ref()
        && !surfaces.contains(surface)
    {
        return None;
    }

    Some(CommandCatalogEntry {
        id: format!("config.shortcut.{name}"),
        name: name.to_owned(),
        slash: format!("/{name}"),
        aliases: Vec::new(),
        description: command.description.clone(),
        category: "Shortcut".to_owned(),
        args_hint: None,
        kind: CommandKind::Shortcut,
        source: CommandSource::Config,
        surfaces,
        dispatch: CommandDispatch::PromptTemplate,
        visibility: CommandVisibility::Visible,
        warnings: Vec::new(),
    })
}
