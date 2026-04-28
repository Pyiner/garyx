use super::*;
use garyx_models::config::{GaryxConfig, SlashCommand};

#[test]
fn applies_custom_slash_command_metadata_and_prompt() {
    let mut config = GaryxConfig::default();
    config.commands.push(SlashCommand {
        name: "summary".to_owned(),
        description: "Summarize".to_owned(),
        prompt: Some("Summarize this".to_owned()),
        skill_id: Some("summary-skill".to_owned()),
    });

    let mut metadata = HashMap::new();
    let transformed =
        apply_custom_slash_command(&config, "/summary extra", "original body", &mut metadata);

    assert_eq!(transformed.as_deref(), Some("Summarize this"));
    assert_eq!(
        metadata.get(SLASH_COMMAND_NAME_KEY),
        Some(&Value::String("summary".to_owned()))
    );
    assert_eq!(
        metadata.get(SLASH_COMMAND_SKILL_ID_KEY),
        Some(&Value::String("summary-skill".to_owned()))
    );
    assert_eq!(
        metadata.get(SLASH_COMMAND_TRIGGERED_KEY),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        metadata.get(SLASH_COMMAND_PROMPT_APPLIED_KEY),
        Some(&Value::Bool(true))
    );
}

#[test]
fn preserves_message_when_slash_command_has_no_prompt() {
    let mut config = GaryxConfig::default();
    config.commands.push(SlashCommand {
        name: "triage".to_owned(),
        description: "Triage".to_owned(),
        prompt: None,
        skill_id: Some("triage-skill".to_owned()),
    });

    let mut metadata = HashMap::new();
    let transformed = apply_custom_slash_command(&config, "/triage", "body", &mut metadata);

    assert_eq!(transformed.as_deref(), Some("body"));
    assert_eq!(metadata.get(SLASH_COMMAND_PROMPT_APPLIED_KEY), None);
}

#[test]
fn annotates_message_metadata_from_slash_command_extra() {
    let mut metadata = HashMap::new();
    metadata.insert(
        SLASH_COMMAND_NAME_KEY.to_owned(),
        Value::String("summary".to_owned()),
    );
    metadata.insert(SLASH_COMMAND_TRIGGERED_KEY.to_owned(), Value::Bool(true));

    let mut message_metadata = MessageMetadata::default();
    annotate_slash_command_metadata(&mut message_metadata, &metadata);

    assert_eq!(
        message_metadata.extra.get(SLASH_COMMAND_NAME_KEY),
        Some(&Value::String("summary".to_owned()))
    );
    assert_eq!(
        message_metadata.extra.get(SLASH_COMMAND_TRIGGERED_KEY),
        Some(&Value::Bool(true))
    );
}
