pub mod slash_commands;

pub use slash_commands::{
    SLASH_COMMAND_NAME_KEY, SLASH_COMMAND_PROMPT_APPLIED_KEY, SLASH_COMMAND_SKILL_ID_KEY,
    SLASH_COMMAND_TRIGGERED_KEY, annotate_slash_command_metadata, apply_custom_slash_command,
};
