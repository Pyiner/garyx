use super::{MemoryContextLayout, build_memory_context_user_message_with_layout};
use garyx_models::local_paths::{
    agent_memory_root_file_for_gary_home, automation_memory_root_file_for_gary_home,
};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

#[test]
fn user_message_creates_and_includes_agent_memory() {
    let temp = tempdir().unwrap();
    let layout = MemoryContextLayout::from_gary_home(temp.path().join(".gary"));
    let metadata = HashMap::from([("agent_id".to_owned(), json!("reviewer"))]);

    let message = build_memory_context_user_message_with_layout(&metadata, &layout)
        .expect("custom agent memory context");

    let agent_memory = agent_memory_root_file_for_gary_home(&temp.path().join(".gary"), "reviewer");
    assert!(agent_memory.is_file());
    assert!(agent_memory.starts_with(temp.path().join(".gary").join("agents")));
    assert!(message.starts_with("<garyx_memory_context>"));
    assert!(message.contains("<agent_memory agent_id=\"reviewer\""));
    assert!(message.contains("Agent ID: `reviewer`"));
    assert!(!message.contains("Global Memory"));
    assert!(!message.contains("Workspace"));
}

#[test]
fn user_message_uses_runtime_thread_agent_id_for_custom_agents() {
    let temp = tempdir().unwrap();
    let layout = MemoryContextLayout::from_gary_home(temp.path().join(".gary"));
    let metadata = HashMap::from([(
        "runtime_context".to_owned(),
        json!({
            "thread": {
                "agent_id": "reviewer"
            }
        }),
    )]);

    let message = build_memory_context_user_message_with_layout(&metadata, &layout)
        .expect("custom runtime agent memory context");

    assert!(message.contains("<agent_memory agent_id=\"reviewer\""));
}

#[test]
fn user_message_includes_agent_and_automation_memory() {
    let temp = tempdir().unwrap();
    let layout = MemoryContextLayout::from_gary_home(temp.path().join(".gary"));
    let gary_home = temp.path().join(".gary");
    let agent_memory = agent_memory_root_file_for_gary_home(&gary_home, "reviewer");
    let automation_memory =
        automation_memory_root_file_for_gary_home(&gary_home, "automation::demo");
    fs::create_dir_all(agent_memory.parent().unwrap()).unwrap();
    fs::create_dir_all(automation_memory.parent().unwrap()).unwrap();
    fs::write(
        &agent_memory,
        "# Agent Memory\n\n## Durable Notes\n- Marker: agent-memory-visible\n",
    )
    .unwrap();
    fs::write(
        &automation_memory,
        "# Automation Memory\n\n## Durable Notes\n- Marker: automation-memory-visible\n",
    )
    .unwrap();
    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("reviewer")),
        ("automation_id".to_owned(), json!("automation::demo")),
    ]);

    let message = build_memory_context_user_message_with_layout(&metadata, &layout)
        .expect("custom agent and automation memory context");

    assert!(message.contains("<agent_memory agent_id=\"reviewer\""));
    assert!(message.contains("agent-memory-visible"));
    assert!(message.contains("<automation_memory automation_id=\"automation::demo\""));
    assert!(message.contains("automation-memory-visible"));
}

#[test]
fn builtin_provider_agents_do_not_get_agent_memory() {
    let temp = tempdir().unwrap();
    let gary_home = temp.path().join(".gary");
    let layout = MemoryContextLayout::from_gary_home(&gary_home);
    for agent_id in ["claude", "codex", "gemini"] {
        let metadata = HashMap::from([("agent_id".to_owned(), json!(agent_id))]);

        let message = build_memory_context_user_message_with_layout(&metadata, &layout);

        assert!(
            message.is_none(),
            "{agent_id} should not receive memory context"
        );
        let agent_memory = agent_memory_root_file_for_gary_home(&gary_home, agent_id);
        assert!(
            !agent_memory.exists(),
            "{agent_id} should not scaffold agent memory"
        );
    }
}

#[test]
fn builtin_provider_agents_still_receive_automation_memory() {
    let temp = tempdir().unwrap();
    let gary_home = temp.path().join(".gary");
    let layout = MemoryContextLayout::from_gary_home(&gary_home);
    let automation_memory =
        automation_memory_root_file_for_gary_home(&gary_home, "automation::demo");
    fs::create_dir_all(automation_memory.parent().unwrap()).unwrap();
    fs::write(
        &automation_memory,
        "# Automation Memory\n\n## Durable Notes\n- Marker: automation-only\n",
    )
    .unwrap();
    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("codex")),
        ("automation_id".to_owned(), json!("automation::demo")),
    ]);

    let message = build_memory_context_user_message_with_layout(&metadata, &layout)
        .expect("automation memory context");

    assert!(!message.contains("<agent_memory"));
    assert!(!message.contains("Agent memory belongs"));
    assert!(message.contains("<automation_memory automation_id=\"automation::demo\""));
    assert!(message.contains("automation-only"));
    assert!(!agent_memory_root_file_for_gary_home(&gary_home, "codex").exists());
}

#[test]
fn missing_agent_and_automation_metadata_does_not_create_default_agent_memory() {
    let temp = tempdir().unwrap();
    let gary_home = temp.path().join(".gary");
    let layout = MemoryContextLayout::from_gary_home(&gary_home);
    let metadata = HashMap::new();

    let message = build_memory_context_user_message_with_layout(&metadata, &layout);

    assert!(message.is_none());
    assert!(!agent_memory_root_file_for_gary_home(&gary_home, "garyx").exists());
}
