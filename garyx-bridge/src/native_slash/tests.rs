use super::*;

#[test]
fn builds_native_skill_prompt_from_skill_id() {
    let metadata = HashMap::from([(
        "slash_command_skill_id".to_owned(),
        Value::String("proof-skill".to_owned()),
    )]);

    assert_eq!(
        build_native_skill_prompt("Use the skill.", &metadata).as_deref(),
        Some("/proof-skill\n\nUse the skill.")
    );
}

#[test]
fn preserves_existing_slash_command_message() {
    let metadata = HashMap::from([(
        "slash_command_skill_id".to_owned(),
        Value::String("proof-skill".to_owned()),
    )]);

    assert_eq!(
        build_native_skill_prompt("/proof-skill arg", &metadata).as_deref(),
        Some("/proof-skill arg")
    );
}

#[test]
fn returns_none_without_skill_metadata() {
    assert!(build_native_skill_prompt("hello", &HashMap::new()).is_none());
}

#[test]
fn ignores_gary_alias_and_uses_downstream_skill_id() {
    let metadata = HashMap::from([
        (
            "slash_command_name".to_owned(),
            Value::String("alias-command".to_owned()),
        ),
        (
            "slash_command_skill_id".to_owned(),
            Value::String("real-skill-id".to_owned()),
        ),
    ]);

    assert_eq!(
        build_native_skill_prompt("body", &metadata),
        Some("/real-skill-id\n\nbody".to_owned())
    );
    assert_ne!(
        build_native_skill_prompt("body", &metadata),
        Some("/alias-command\n\nbody".to_owned())
    );
}

#[test]
fn builds_minimal_prompt_for_empty_message() {
    let metadata = HashMap::from([(
        "slash_command_skill_id".to_owned(),
        Value::String("proof-skill".to_owned()),
    )]);

    assert_eq!(
        build_native_skill_prompt("   ", &metadata),
        Some("/proof-skill".to_owned())
    );
}
