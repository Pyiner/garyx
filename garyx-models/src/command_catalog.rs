use serde::{Deserialize, Serialize};

pub const COMMAND_CATALOG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandCatalog {
    pub version: u32,
    pub revision: String,
    pub commands: Vec<CommandCatalogEntry>,
    #[serde(default)]
    pub warnings: Vec<CommandWarning>,
}

impl CommandCatalog {
    pub fn from_entries(commands: Vec<CommandCatalogEntry>) -> Self {
        Self::from_parts(commands, Vec::new())
    }

    pub fn from_parts(commands: Vec<CommandCatalogEntry>, warnings: Vec<CommandWarning>) -> Self {
        let revision = revision_for(&commands, &warnings);
        Self {
            version: COMMAND_CATALOG_VERSION,
            revision,
            commands,
            warnings,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandCatalogEntry {
    pub id: String,
    pub name: String,
    pub slash: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub description: String,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_hint: Option<String>,
    pub kind: CommandKind,
    pub source: CommandSource,
    #[serde(default)]
    pub surfaces: Vec<CommandSurface>,
    pub dispatch: CommandDispatch,
    pub visibility: CommandVisibility,
    #[serde(default)]
    pub warnings: Vec<CommandWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    ChannelNative,
    Shortcut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    Builtin,
    Config,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandSurface {
    Router,
    GatewayApi,
    DesktopComposer,
    Telegram,
    ApiChat,
    Plugin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandDispatch {
    RouterNative { key: String },
    PromptTemplate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandVisibility {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandWarning {
    pub code: String,
    pub message: String,
}

impl CommandWarning {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
pub struct CommandCatalogOptions {
    #[serde(default)]
    pub surface: Option<CommandSurface>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub include_hidden: bool,
}

pub fn command_name_from_text(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let token = trimmed.split_whitespace().next().unwrap_or(trimmed);
    let without_slash = token.strip_prefix('/')?;
    let command = without_slash
        .split('@')
        .next()
        .unwrap_or(without_slash)
        .trim();
    if command.is_empty() {
        None
    } else {
        Some(command)
    }
}

pub fn normalize_shortcut_command_name(name: &str) -> String {
    name.trim()
        .trim_start_matches('/')
        .split('@')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

pub fn is_valid_shortcut_command_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn revision_for(commands: &[CommandCatalogEntry], warnings: &[CommandWarning]) -> String {
    let payload = serde_json::json!({
        "version": COMMAND_CATALOG_VERSION,
        "commands": commands,
        "warnings": warnings,
    })
    .to_string();
    format!("v1:{:016x}", fnv1a64(payload.as_bytes()))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
