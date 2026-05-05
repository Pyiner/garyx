use super::{
    GARY_BASE_INSTRUCTIONS, compose_gary_instructions_with_layout,
    prepend_initial_context_to_user_message, prepend_memory_context_to_user_message,
    prepend_runtime_metadata_to_user_message, task_cli_env,
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn compose_without_extra_returns_base_only() {
    let value = compose_gary_instructions_with_layout(None);

    assert!(value.starts_with(GARY_BASE_INSTRUCTIONS.trim_end()));
    assert!(value.contains("Self-evolution:"));
    assert!(value.contains("System capabilities:"));
    assert!(value.contains("garyx task create"));
    assert!(value.contains("garyx automation create"));
    assert!(!value.contains("Global Memory"));
    assert!(!value.contains("Workspace Memory"));
    assert!(!value.contains("Additional runtime instructions:"));
    assert!(!value.contains("Current runtime context:"));
}

#[test]
fn compose_with_extra_appends_section() {
    let value = compose_gary_instructions_with_layout(Some("Use concise bullets."));

    assert!(value.contains("Garyx runtime guidance:"));
    assert!(value.contains("Additional runtime instructions:"));
    assert!(value.contains("Use concise bullets."));
    assert!(!value.contains("Current runtime context:"));
}

#[test]
fn prepend_memory_context_to_user_message_only_when_requested() {
    let metadata = HashMap::from([("agent_id".to_owned(), json!("reviewer"))]);

    assert_eq!(
        prepend_memory_context_to_user_message("hello", &metadata, false),
        "hello"
    );

    let rendered = prepend_memory_context_to_user_message("hello", &metadata, true);
    assert!(rendered.starts_with("<garyx_memory_context>"));
    assert!(rendered.contains("<agent_memory agent_id=\"reviewer\""));
    assert!(rendered.ends_with("hello"));
}

#[test]
fn prepend_memory_context_skips_builtin_provider_agents() {
    for agent_id in ["claude", "codex", "gemini"] {
        let metadata = HashMap::from([("agent_id".to_owned(), json!(agent_id))]);

        assert_eq!(
            prepend_memory_context_to_user_message("hello", &metadata, true),
            "hello",
            "{agent_id} should not receive agent memory guidance"
        );
    }
}

#[test]
fn prepend_runtime_metadata_to_user_message_renders_stable_thread_task_and_bot_ids() {
    let metadata = HashMap::from([(
        "runtime_context".to_owned(),
        json!({
            "thread_id": "thread::abc",
            "bot_id": "telegram:main",
            "workspace_dir": "/tmp/project",
            "task": {
                "task_id": "#TASK-3",
                "title": "Fix context prompt",
                "status": "in_progress",
                "assignee": { "kind": "agent", "agent_id": "codex" }
            }
        }),
    )]);
    let rendered = prepend_runtime_metadata_to_user_message("继续", &metadata);

    assert!(rendered.starts_with("<garyx_thread_metadata>"));
    assert!(rendered.contains("thread_id: thread::abc"));
    assert!(rendered.contains("bot_id: telegram:main"));
    assert!(rendered.contains("workspace_dir: /tmp/project"));
    assert!(rendered.contains("task_id: #TASK-3"));
    assert!(!rendered.contains("status"));
    assert!(!rendered.contains("assignee"));
    assert!(rendered.ends_with("继续"));
}

#[test]
fn prepend_runtime_metadata_to_user_message_leaves_unknown_context_unchanged() {
    let metadata = HashMap::new();

    assert_eq!(
        prepend_runtime_metadata_to_user_message("plain message\n", &metadata),
        "plain message\n"
    );
}

#[test]
fn prepend_initial_context_to_user_message_combines_runtime_metadata_and_memory_once() {
    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("reviewer")),
        (
            "runtime_context".to_owned(),
            json!({
                "thread_id": "thread::abc",
                "bot_id": "telegram:main",
                "task": {
                    "task_id": "#TASK-3",
                    "status": "done"
                }
            }),
        ),
    ]);

    assert_eq!(
        prepend_initial_context_to_user_message("hello", &metadata, false),
        "hello"
    );

    let rendered = prepend_initial_context_to_user_message("hello", &metadata, true);
    assert!(rendered.starts_with("<garyx_thread_metadata>"));
    assert!(rendered.contains("thread_id: thread::abc"));
    assert!(rendered.contains("bot_id: telegram:main"));
    assert!(rendered.contains("task_id: #TASK-3"));
    assert!(!rendered.contains("status: done"));
    assert!(rendered.contains("<garyx_memory_context>"));
    assert!(rendered.contains("<agent_memory agent_id=\"reviewer\""));
    assert!(rendered.ends_with("hello"));
}

#[test]
fn task_cli_env_exports_current_agent_and_task_identity() {
    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("reviewer")),
        (
            "runtime_context".to_owned(),
            json!({
                "thread_id": "thread::abc",
                "bot_id": "telegram:main",
                "channel": "telegram",
                "account_id": "main",
                "task": {
                    "task_id": "#TASK-3",
                    "status": "in_progress"
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
        env.get("GARYX_TASK_ID").map(String::as_str),
        Some("#TASK-3")
    );
    assert_eq!(
        env.get("GARYX_TASK_STATUS").map(String::as_str),
        Some("in_progress")
    );
    assert_eq!(
        env.get("GARYX_BOT_ID").map(String::as_str),
        Some("telegram:main")
    );
    assert_eq!(
        env.get("GARYX_CHANNEL").map(String::as_str),
        Some("telegram")
    );
    assert_eq!(
        env.get("GARYX_ACCOUNT_ID").map(String::as_str),
        Some("main")
    );
}
