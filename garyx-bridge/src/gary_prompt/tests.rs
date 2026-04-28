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
            "workspace_dir": "/tmp/ws"
        }),
    )]);
    let rendered = append_runtime_context_section(base, "thread::abc", None, &metadata);

    assert!(rendered.contains("Current runtime context:"));
    assert!(rendered.contains("channel: weixin"));
    assert!(rendered.contains("thread_id: thread::abc"));
    assert!(rendered.contains("workspace_dir: /tmp/ws"));
}
