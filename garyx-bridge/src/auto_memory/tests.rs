use super::{AutoMemoryLayout, build_auto_memory_prompt_section};
use garyx_models::local_paths::{
    auto_memory_automation_root_file_for_gary_home, auto_memory_root_file_for_gary_home,
    auto_memory_workspace_root_file_for_gary_home,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn prompt_section_creates_global_memory_md() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));

    let prompt = build_auto_memory_prompt_section(&layout, None, None);

    let root_file = auto_memory_root_file_for_gary_home(&temp.path().join(".gary"));
    assert!(root_file.is_file());
    assert!(prompt.contains("~/.garyx/auto-memory/memory.md"));
    assert!(prompt.contains("# Auto Memory"));
}

#[test]
fn prompt_section_includes_workspace_memory_contents() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));
    let workspace = temp.path().join("repo");
    fs::create_dir_all(&workspace).unwrap();

    let workspace_memory =
        auto_memory_workspace_root_file_for_gary_home(&temp.path().join(".gary"), &workspace);
    fs::create_dir_all(workspace_memory.parent().unwrap()).unwrap();
    fs::write(
        &workspace_memory,
        "# Auto Memory\n\n## Durable Notes\n- Marker: workspace-memory-visible\n",
    )
    .unwrap();

    let prompt = build_auto_memory_prompt_section(&layout, Some(&workspace), None);

    assert!(prompt.contains("Scoped Auto Memory (Workspace)"));
    assert!(prompt.contains("workspace-memory-visible"));
}

#[test]
fn prompt_section_prefers_automation_memory_over_workspace() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));
    let workspace = temp.path().join("repo");
    fs::create_dir_all(&workspace).unwrap();

    let automation_memory = auto_memory_automation_root_file_for_gary_home(
        &temp.path().join(".gary"),
        "automation::demo",
    );
    fs::create_dir_all(automation_memory.parent().unwrap()).unwrap();
    fs::write(
        &automation_memory,
        "# Auto Memory\n\n## Durable Notes\n- Marker: automation-memory-visible\n",
    )
    .unwrap();

    let prompt =
        build_auto_memory_prompt_section(&layout, Some(&workspace), Some("automation::demo"));

    assert!(prompt.contains("Scoped Auto Memory (Automation)"));
    assert!(prompt.contains("automation-memory-visible"));
    assert!(!prompt.contains("Scoped Auto Memory (Workspace)"));
}

#[test]
fn automation_prompt_includes_memory_protocol() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));

    let prompt = build_auto_memory_prompt_section(&layout, None, Some("automation::test"));

    assert!(prompt.contains("Automation Memory Protocol"));
    assert!(prompt.contains("BEFORE starting the task"));
    assert!(prompt.contains("AFTER completing the task"));
}

#[test]
fn workspace_prompt_does_not_include_automation_protocol() {
    let temp = tempdir().unwrap();
    let layout = AutoMemoryLayout::from_gary_home(temp.path().join(".gary"));
    let workspace = temp.path().join("repo");
    fs::create_dir_all(&workspace).unwrap();

    let prompt = build_auto_memory_prompt_section(&layout, Some(&workspace), None);

    assert!(!prompt.contains("Automation Memory Protocol"));
}
