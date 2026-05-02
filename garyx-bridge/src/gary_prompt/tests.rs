use super::{
    AutoMemoryLayout, GARY_BASE_INSTRUCTIONS, append_runtime_context_section,
    compose_gary_instructions_with_layout,
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
    assert!(value.contains("Global Auto Memory"));
    assert!(!value.contains("Additional runtime instructions:"));
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

    assert!(value.contains("You are Garyx"));
    assert!(value.contains("Scoped Auto Memory (Workspace)"));
    assert!(value.contains("Additional runtime instructions:"));
    assert!(value.contains("Use concise bullets."));
}

#[test]
fn append_runtime_context_section_renders_expected_fields() {
    let base = "base instructions".to_owned();
    let metadata = HashMap::from([(
        "runtime_context".to_owned(),
        json!({
            "channel": "weixin",
            "account_id": "main",
            "from_id": "user42",
            "is_group": false,
            "workspace_dir": "/tmp/ws",
            "bot_id": "weixin:main",
            "bot": {
                "id": "weixin:main",
                "thread_binding_key": "user42"
            },
            "thread": {
                "id": "thread::abc",
                "label": "Prompt context",
                "bound_bots": ["weixin:main"],
                "channel_bindings": [{
                    "bot_id": "weixin:main",
                    "binding_key": "user42",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "user42",
                    "display_label": "User 42"
                }]
            },
            "task": {
                "task_ref": "#weixin/main/3",
                "title": "Fix context prompt",
                "status": "in_progress",
                "scope": "weixin/main"
            }
        }),
    )]);
    let rendered = append_runtime_context_section(base, "thread::abc", None, &metadata);

    assert!(rendered.contains("Current runtime context:"));
    assert!(rendered.contains("channel: weixin"));
    assert!(rendered.contains("account_id: main"));
    assert!(rendered.contains("from_id: user42"));
    assert!(rendered.contains("bot_id: weixin:main"));
    assert!(rendered.contains("thread_id: thread::abc"));
    assert!(rendered.contains("workspace_dir: /tmp/ws"));
    assert!(rendered.contains("- bot:"));
    assert!(rendered.contains("thread_binding_key: user42"));
    assert!(rendered.contains("- thread:"));
    assert!(rendered.contains("label: Prompt context"));
    assert!(rendered.contains("bound_bots: weixin:main"));
    assert!(rendered.contains(
        "weixin:main binding_key=user42 delivery_target_type=chat_id delivery_target_id=user42 label=User 42"
    ));
    assert!(rendered.contains("- task:"));
    assert!(rendered.contains("task_ref: #weixin/main/3"));
    assert!(rendered.contains("status: in_progress"));
}
