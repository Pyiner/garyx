use super::{AutoMemoryLayout, build_auto_memory_user_message_with_layout};
use garyx_models::local_paths::{
    auto_memory_agent_root_file_for_gary_home, auto_memory_automation_root_file_for_gary_home,
};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

#[test]
fn user_message_creates_and_includes_agent_memory() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));
    let metadata = HashMap::from([("agent_id".to_owned(), json!("reviewer"))]);

    let message = build_auto_memory_user_message_with_layout(&metadata, &layout);

    let agent_memory =
        auto_memory_agent_root_file_for_gary_home(&temp.path().join(".gary"), "reviewer");
    assert!(agent_memory.is_file());
    assert!(message.starts_with("<garyx_memory_context>"));
    assert!(message.contains("<agent_memory agent_id=\"reviewer\""));
    assert!(message.contains("Agent ID: `reviewer`"));
    assert!(!message.contains("Global Auto Memory"));
    assert!(!message.contains("Workspace"));
}

#[test]
fn user_message_uses_runtime_thread_agent_id() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));
    let metadata = HashMap::from([(
        "runtime_context".to_owned(),
        json!({
            "thread": {
                "agent_id": "codex"
            }
        }),
    )]);

    let message = build_auto_memory_user_message_with_layout(&metadata, &layout);

    assert!(message.contains("<agent_memory agent_id=\"codex\""));
}

#[test]
fn user_message_includes_agent_and_automation_memory() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));
    let gary_home = temp.path().join(".gary");
    let agent_memory = auto_memory_agent_root_file_for_gary_home(&gary_home, "codex");
    let automation_memory =
        auto_memory_automation_root_file_for_gary_home(&gary_home, "automation::demo");
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
        ("agent_id".to_owned(), json!("codex")),
        ("automation_id".to_owned(), json!("automation::demo")),
    ]);

    let message = build_auto_memory_user_message_with_layout(&metadata, &layout);

    assert!(message.contains("<agent_memory agent_id=\"codex\""));
    assert!(message.contains("agent-memory-visible"));
    assert!(message.contains("<automation_memory automation_id=\"automation::demo\""));
    assert!(message.contains("automation-memory-visible"));
}
