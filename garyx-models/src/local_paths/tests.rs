use super::{
    auto_memory_automation_dir_for_gary_home, auto_memory_workspace_dir_for_gary_home,
    auto_memory_workspace_key,
};
use std::fs;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn migrate_moves_legacy_garyx_state_into_gary() {
    // Test the migrate_path helper directly instead of modifying HOME,
    // which causes race conditions with other concurrent tests.
    use super::migrate_path;

    let temp = tempdir().unwrap();
    let legacy_root = temp.path().join("legacy");
    let gary_root = temp.path().join("gary");

    // Set up legacy directory structure
    fs::create_dir_all(legacy_root.join("skills").join("alpha")).unwrap();
    fs::create_dir_all(legacy_root.join("logs")).unwrap();
    fs::write(
        legacy_root.join("skills").join("alpha").join("SKILL.md"),
        "# Alpha\n",
    )
    .unwrap();
    fs::write(legacy_root.join("mcp-sync-state.json"), "{}").unwrap();
    fs::write(legacy_root.join("logs").join("gary.log"), "hello\n").unwrap();

    // Migrate each path (mirrors what migrate_legacy_homes does)
    fs::create_dir_all(&gary_root).unwrap();
    for name in ["skills", "logs", "mcp-sync-state.json"] {
        migrate_path(&legacy_root.join(name), &gary_root.join(name)).unwrap();
    }

    // Verify migration results
    assert_eq!(
        fs::read_to_string(gary_root.join("skills").join("alpha").join("SKILL.md")).unwrap(),
        "# Alpha\n"
    );
    assert_eq!(
        fs::read_to_string(gary_root.join("logs").join("gary.log")).unwrap(),
        "hello\n"
    );
    assert!(gary_root.join("mcp-sync-state.json").is_file());
    // Legacy dirs should be empty/removed after migration
    assert!(!legacy_root.join("skills").exists());
    assert!(!legacy_root.join("logs").exists());
}

#[test]
fn auto_memory_workspace_key_is_stable_and_safe() {
    let path = Path::new("/tmp/Gary Bot");
    let key = auto_memory_workspace_key(path);
    assert_eq!(key, auto_memory_workspace_key(path));
    assert!(key.starts_with("gary-bot-"));
    assert!(
        key.chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    );
}

#[test]
fn auto_memory_workspace_dir_uses_workspace_key() {
    let temp = tempdir().unwrap();
    let workspace = Path::new("/tmp/Repo One");
    let dir = auto_memory_workspace_dir_for_gary_home(&temp.path().join(".gary"), workspace);
    assert!(
        dir.starts_with(
            temp.path()
                .join(".gary")
                .join("auto-memory")
                .join("workspaces")
        )
    );
    assert_eq!(
        dir.file_name().and_then(|value| value.to_str()),
        Some(auto_memory_workspace_key(workspace).as_str())
    );
}

#[test]
fn auto_memory_automation_dir_sanitizes_id() {
    let temp = tempdir().unwrap();
    let dir = auto_memory_automation_dir_for_gary_home(
        &temp.path().join(".gary"),
        "automation::Morning Digest",
    );
    assert!(
        dir.starts_with(
            temp.path()
                .join(".gary")
                .join("auto-memory")
                .join("automations")
        )
    );
    assert_eq!(
        dir.file_name().and_then(|value| value.to_str()),
        Some("automation-morning-digest")
    );
}
