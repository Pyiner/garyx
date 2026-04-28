use super::*;
use tempfile::tempdir;

fn create_skill(root: &Path, id: &str, body: &str) {
    let dir = root.join(id);
    fs::create_dir_all(dir.join("scripts")).unwrap();
    fs::write(dir.join("SKILL.md"), body).unwrap();
    fs::write(dir.join("scripts").join("run.sh"), "echo ok\n").unwrap();
}

#[test]
fn sync_mirrors_enabled_user_skills_to_claude_and_codex() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let user_dir = home.join(".garyx").join("skills");
    fs::create_dir_all(&user_dir).unwrap();
    create_skill(&user_dir, "alpha", "# Alpha\n");

    let mut state = HashMap::new();
    state.insert("alpha".to_owned(), true);
    sync_external_user_skills(&user_dir, &state).unwrap();

    for target in [
        home.join(".claude").join("skills").join("alpha"),
        home.join(".codex").join("skills").join("alpha"),
    ] {
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "# Alpha\n"
        );
        assert_eq!(
            fs::read_to_string(target.join("scripts").join("run.sh")).unwrap(),
            "echo ok\n"
        );
    }
}

#[test]
fn sync_removes_disabled_and_deleted_skills_from_targets() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let user_dir = home.join(".garyx").join("skills");
    fs::create_dir_all(&user_dir).unwrap();
    create_skill(&user_dir, "alpha", "# Alpha\n");
    create_skill(&user_dir, "beta", "# Beta\n");

    let mut state = HashMap::new();
    state.insert("alpha".to_owned(), true);
    state.insert("beta".to_owned(), true);
    sync_external_user_skills(&user_dir, &state).unwrap();

    state.insert("beta".to_owned(), false);
    fs::remove_dir_all(user_dir.join("alpha")).unwrap();
    sync_external_user_skills(&user_dir, &state).unwrap();

    for root in [
        home.join(".claude").join("skills"),
        home.join(".codex").join("skills"),
    ] {
        assert!(!root.join("alpha").exists());
        assert!(!root.join("beta").exists());
    }
}

#[test]
fn sync_uses_explicit_home_when_user_dir_is_not_the_process_home() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let user_dir = home.join(".garyx").join("skills");
    fs::create_dir_all(&user_dir).unwrap();
    create_skill(&user_dir, "alpha", "# Alpha\n");

    sync_external_user_skills(&user_dir, &HashMap::from([("alpha".to_owned(), true)])).unwrap();

    assert!(home.join(".claude").join("skills").join("alpha").is_dir());
    assert!(home.join(".codex").join("skills").join("alpha").is_dir());
}

#[test]
fn sync_replaces_existing_copied_directories_with_symlinks() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let user_dir = home.join(".garyx").join("skills");
    let claude_alpha = home.join(".claude").join("skills").join("alpha");
    let codex_alpha = home.join(".codex").join("skills").join("alpha");
    fs::create_dir_all(&user_dir).unwrap();
    create_skill(&user_dir, "alpha", "# Alpha\n");

    for target in [&claude_alpha, &codex_alpha] {
        fs::create_dir_all(target).unwrap();
        fs::write(target.join("SKILL.md"), "# stale\n").unwrap();
    }

    sync_external_user_skills(&user_dir, &HashMap::from([("alpha".to_owned(), true)])).unwrap();

    for target in [&claude_alpha, &codex_alpha] {
        let metadata = fs::symlink_metadata(target).unwrap();
        assert!(metadata.file_type().is_symlink());
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "# Alpha\n"
        );
    }
}

#[test]
fn sync_supports_garyx_skills_dir_and_state_path() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let user_dir = home.join(".garyx").join("skills");
    fs::create_dir_all(&user_dir).unwrap();
    create_skill(&user_dir, "alpha", "# Alpha\n");

    sync_external_user_skills(&user_dir, &HashMap::from([("alpha".to_owned(), true)])).unwrap();

    for target in [
        home.join(".claude").join("skills").join("alpha"),
        home.join(".codex").join("skills").join("alpha"),
    ] {
        let metadata = fs::symlink_metadata(&target).unwrap();
        assert!(metadata.file_type().is_symlink());
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "# Alpha\n"
        );
    }

    let managed_state_path = home.join(".garyx").join("skills-sync-state.json");
    assert!(managed_state_path.is_file());
    assert_eq!(
        read_managed_ids(&managed_state_path).unwrap(),
        HashSet::from(["alpha".to_owned()])
    );
}
