use super::{
    GARY_BASE_INSTRUCTIONS, append_task_suffix_to_user_message,
    compose_gary_instructions_with_layout, prepend_auto_memory_to_user_message, task_cli_env,
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
    assert!(!value.contains("Global Auto Memory"));
    assert!(!value.contains("Scoped Auto Memory"));
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
fn prepend_auto_memory_to_user_message_only_when_requested() {
    let metadata = HashMap::from([("agent_id".to_owned(), json!("reviewer"))]);

    assert_eq!(
        prepend_auto_memory_to_user_message("hello", &metadata, false),
        "hello"
    );

    let rendered = prepend_auto_memory_to_user_message("hello", &metadata, true);
    assert!(rendered.starts_with("<garyx_memory_context>"));
    assert!(rendered.contains("<agent_memory agent_id=\"reviewer\""));
    assert!(rendered.ends_with("hello"));
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
