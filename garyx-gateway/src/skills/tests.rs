use super::*;
use tempfile::tempdir;

fn test_service() -> (tempfile::TempDir, SkillsService) {
    let temp = tempdir().unwrap();
    let user_dir = temp.path().join(".garyx").join("skills");
    (temp, SkillsService::new(user_dir, None))
}

#[test]
fn parses_frontmatter_when_present() {
    let frontmatter =
        parse_skill_frontmatter("---\nname: Example\ndescription: Sample skill\n---\n\n# Body\n");

    assert_eq!(frontmatter.name, "Example");
    assert_eq!(frontmatter.description, "Sample skill");
}

#[test]
fn defaults_when_frontmatter_is_missing() {
    let frontmatter = parse_skill_frontmatter("# No frontmatter\n");
    assert!(frontmatter.name.is_empty());
    assert!(frontmatter.description.is_empty());
}

#[test]
fn rewrite_skill_markdown_updates_frontmatter_and_heading() {
    let updated = rewrite_skill_markdown(
        "---\nname: Example\ndescription: Sample skill\n---\n\n# Example\n\nBody\n",
        "example",
        "Example Prime",
        "Updated skill",
    )
    .unwrap();

    assert!(updated.contains("name: Example Prime"));
    assert!(updated.contains("description: Updated skill"));
    assert!(updated.contains("# Example Prime"));
    assert!(updated.contains("Body"));
}

#[test]
fn skill_editor_state_lists_nested_entries() {
    let (_temp, service) = test_service();
    service
        .create_skill("example", "Example", "Example skill", "# Example\n\nBody\n")
        .unwrap();
    service
        .create_skill_entry("example", "scripts/read.mjs", "file")
        .unwrap();
    service
        .create_skill_entry("example", "references/guide.md", "file")
        .unwrap();

    let editor = service.skill_editor_state("example").unwrap();
    assert_eq!(editor.skill.id, "example");
    assert_eq!(editor.entries.len(), 3);
    assert_eq!(editor.entries[0].entry_type, "directory");
    assert_eq!(editor.entries[0].path, "references");
    assert_eq!(editor.entries[0].children[0].path, "references/guide.md");
    assert_eq!(editor.entries[1].entry_type, "directory");
    assert_eq!(editor.entries[1].path, "scripts");
    assert_eq!(editor.entries[1].children[0].path, "scripts/read.mjs");
    assert_eq!(editor.entries[2].path, "SKILL.md");
}

#[test]
fn read_write_skill_file_round_trips_and_refreshes_skill_info() {
    let (_temp, service) = test_service();
    service
        .create_skill("example", "Example", "Example skill", "# Example\n\nBody\n")
        .unwrap();
    service
        .create_skill_entry("example", "scripts/read.mjs", "file")
        .unwrap();

    let updated_skill = service
        .write_skill_file(
            "example",
            "SKILL.md",
            "---\nname: Example Prime\ndescription: Updated description\n---\n\n# Example Prime\n\nBody\n",
        )
        .unwrap();
    assert_eq!(updated_skill.skill.name, "Example Prime");
    assert_eq!(updated_skill.skill.description, "Updated description");

    service
        .write_skill_file("example", "scripts/read.mjs", "console.log('hi')\n")
        .unwrap();
    let document = service
        .read_skill_file("example", "scripts/read.mjs")
        .unwrap();
    assert_eq!(document.path, "scripts/read.mjs");
    assert_eq!(document.content, "console.log('hi')\n");
}

#[test]
fn read_skill_file_supports_image_previews() {
    let (temp, service) = test_service();
    service
        .create_skill("example", "Example", "Example skill", "# Example\n\nBody\n")
        .unwrap();
    service
        .create_skill_entry("example", "assets/preview.png", "file")
        .unwrap();

    let image_path = temp
        .path()
        .join(".garyx")
        .join("skills")
        .join("example")
        .join("assets")
        .join("preview.png");
    let image_bytes = b"\x89PNG\r\n\x1a\nfake-png-preview";
    fs::write(&image_path, image_bytes).unwrap();

    let document = service
        .read_skill_file("example", "assets/preview.png")
        .unwrap();
    assert_eq!(document.path, "assets/preview.png");
    assert_eq!(document.media_type, "image/png");
    assert_eq!(document.preview_kind, "image");
    assert!(!document.editable);
    assert!(document.content.is_empty());
    assert_eq!(document.data_base64, Some(BASE64.encode(image_bytes)));
}

#[test]
fn write_skill_file_rejects_binary_preview_entries() {
    let (temp, service) = test_service();
    service
        .create_skill("example", "Example", "Example skill", "# Example\n\nBody\n")
        .unwrap();
    service
        .create_skill_entry("example", "assets/preview.png", "file")
        .unwrap();

    let image_path = temp
        .path()
        .join(".garyx")
        .join("skills")
        .join("example")
        .join("assets")
        .join("preview.png");
    fs::write(&image_path, b"\x89PNG\r\n\x1a\nfake-png-preview").unwrap();

    let error = service
        .write_skill_file("example", "assets/preview.png", "not really an image")
        .unwrap_err();
    assert!(
        matches!(error, SkillStoreError::Validation(message) if message.contains("not editable text"))
    );
}

#[test]
fn rejects_parent_traversal_for_skill_entries() {
    let (_temp, service) = test_service();
    service
        .create_skill("example", "Example", "Example skill", "# Example\n\nBody\n")
        .unwrap();

    let error = service
        .read_skill_file("example", "../secret.txt")
        .unwrap_err();
    assert!(
        matches!(error, SkillStoreError::Validation(message) if message.contains("inside the skill directory"))
    );
}

#[test]
fn does_not_allow_deleting_skill_markdown() {
    let (_temp, service) = test_service();
    service
        .create_skill("example", "Example", "Example skill", "# Example\n\nBody\n")
        .unwrap();

    let error = service
        .delete_skill_entry("example", "SKILL.md")
        .unwrap_err();
    assert!(
        matches!(error, SkillStoreError::Validation(message) if message.contains("cannot be deleted"))
    );
}

#[test]
fn create_skill_persists_authored_body() {
    let (_temp, service) = test_service();
    service
        .create_skill(
            "example",
            "Example",
            "Example skill",
            "## Workflow\n\n1. Do the thing.\n",
        )
        .unwrap();

    let document = service.read_skill_file("example", "SKILL.md").unwrap();
    assert!(document.content.contains("name: Example"));
    assert!(document.content.contains("description: Example skill"));
    assert!(document.content.contains("## Workflow"));
    assert!(
        !document
            .content
            .contains("Describe how this skill should behave.")
    );
}

#[test]
fn create_skill_requires_non_empty_body() {
    let (_temp, service) = test_service();
    let error = service
        .create_skill("example", "Example", "Example skill", "   \n\t  ")
        .unwrap_err();

    assert!(
        matches!(error, SkillStoreError::Validation(message) if message.contains("content is required"))
    );
}

#[test]
fn sync_external_user_skills_repairs_missing_external_links() {
    let (temp, service) = test_service();
    service
        .create_skill("example", "Example", "Example skill", "# Example\n\nBody\n")
        .unwrap();

    let codex_skill = temp.path().join(".codex").join("skills").join("example");
    fs::remove_file(&codex_skill).unwrap();
    assert!(!codex_skill.exists());

    service.sync_external_user_skills().unwrap();

    let metadata = fs::symlink_metadata(&codex_skill).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(
        fs::read_to_string(codex_skill.join("SKILL.md")).unwrap(),
        "---\nname: Example\ndescription: Example skill\n---\n\n# Example\n\nBody\n"
    );
}
