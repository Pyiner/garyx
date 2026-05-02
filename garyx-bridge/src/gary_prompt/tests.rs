use super::{
    AutoMemoryLayout, GARY_BASE_INSTRUCTIONS, append_task_suffix_to_user_message,
    compose_gary_instructions_with_layout, task_cli_env,
};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

#[test]
fn compose_without_extra_returns_base_and_auto_memory() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));

    let value = compose_gary_instructions_with_layout(None, None, None, &layout);

    assert!(value.starts_with(GARY_BASE_INSTRUCTIONS.trim_end()));
    assert!(value.contains("Garyx has a built-in Auto Memory system."));
    assert!(value.contains("Task workflow:"));
    assert!(value.contains("Use the `garyx task` CLI"));
    assert!(value.contains("Global Auto Memory"));
    assert!(!value.contains("Additional runtime instructions:"));
    assert!(!value.contains("Current runtime context:"));
}

#[test]
fn compose_with_extra_appends_section() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));
    let workspace = temp.path().join("repo");
    fs::create_dir_all(&workspace).unwrap();

    let value = compose_gary_instructions_with_layout(
        Some("Use concise bullets."),
        Some(&workspace),
        None,
        &layout,
    );

    assert!(value.contains("Operate as a durable, self-improving agent:"));
    assert!(value.contains("Scoped Auto Memory (Workspace)"));
    assert!(value.contains("Additional runtime instructions:"));
    assert!(value.contains("Use concise bullets."));
    assert!(!value.contains("Current runtime context:"));
}

#[test]
fn append_task_suffix_to_user_message_renders_live_task_snapshot() {
    let metadata = HashMap::from([(
        "runtime_context".to_owned(),
        json!({
            "task": {
                "task_ref": "#TASK-3",
                "title": "Fix context prompt",
                "status": "in_progress",
                "scope": "weixin/main",
                "assignee": { "kind": "agent", "agent_id": "codex" }
            }
        }),
    )]);
    let rendered = append_task_suffix_to_user_message("继续", &metadata);

    assert_eq!(
        rendered,
        "继续 [task #TASK-3 status=in_progress assignee=agent:codex]"
    );
}

#[test]
fn append_task_suffix_to_user_message_leaves_non_task_messages_unchanged() {
    let metadata = HashMap::new();

    assert_eq!(
        append_task_suffix_to_user_message("plain message\n", &metadata),
        "plain message\n"
    );
}

#[test]
fn task_cli_env_exports_current_agent_and_task_identity() {
    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("reviewer")),
        (
            "runtime_context".to_owned(),
            json!({
                "thread_id": "thread::abc",
                "task": {
                    "task_ref": "#TASK-3",
                    "status": "in_progress",
                    "scope": "weixin/main"
                }
            }),
        ),
    ]);
    let env = task_cli_env(&metadata);

    assert_eq!(
        env.get("GARYX_THREAD_ID").map(String::as_str),
        Some("thread::abc")
    );
    assert_eq!(
        env.get("GARYX_AGENT_ID").map(String::as_str),
        Some("reviewer")
    );
    assert_eq!(
        env.get("GARYX_ACTOR").map(String::as_str),
        Some("agent:reviewer")
    );
    assert_eq!(
        env.get("GARYX_TASK_REF").map(String::as_str),
        Some("#TASK-3")
    );
    assert_eq!(
        env.get("GARYX_TASK_STATUS").map(String::as_str),
        Some("in_progress")
    );
}
