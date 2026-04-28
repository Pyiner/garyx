use std::collections::HashMap;

use garyx_models::{config::GaryxConfig, messages::MessageMetadata};
use serde_json::Value;

pub const SLASH_COMMAND_NAME_KEY: &str = "slash_command_name";
pub const SLASH_COMMAND_SKILL_ID_KEY: &str = "slash_command_skill_id";
pub const SLASH_COMMAND_TRIGGERED_KEY: &str = "slash_command_triggered";
pub const SLASH_COMMAND_PROMPT_APPLIED_KEY: &str = "slash_command_prompt_applied";

const SLASH_COMMAND_METADATA_KEYS: [&str; 4] = [
    SLASH_COMMAND_NAME_KEY,
    SLASH_COMMAND_SKILL_ID_KEY,
    SLASH_COMMAND_TRIGGERED_KEY,
    SLASH_COMMAND_PROMPT_APPLIED_KEY,
];

pub fn apply_custom_slash_command(
    config: &GaryxConfig,
    command_text: &str,
    message: &str,
    metadata: &mut HashMap<String, Value>,
) -> Option<String> {
    let command = config.resolve_slash_command(command_text)?;

    metadata.insert(
        SLASH_COMMAND_NAME_KEY.to_owned(),
        Value::String(command.name),
    );
    metadata.insert(SLASH_COMMAND_TRIGGERED_KEY.to_owned(), Value::Bool(true));
    if let Some(skill_id) = command.skill_id {
        metadata.insert(
            SLASH_COMMAND_SKILL_ID_KEY.to_owned(),
            Value::String(skill_id),
        );
    }
    if let Some(prompt) = command.prompt {
        metadata.insert(
            SLASH_COMMAND_PROMPT_APPLIED_KEY.to_owned(),
            Value::Bool(true),
        );
        return Some(prompt);
    }

    Some(message.to_owned())
}

pub fn annotate_slash_command_metadata(
    message_metadata: &mut MessageMetadata,
    metadata: &HashMap<String, Value>,
) {
    for key in SLASH_COMMAND_METADATA_KEYS {
        if let Some(value) = metadata.get(key) {
            message_metadata.extra.insert(key.to_owned(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests;
