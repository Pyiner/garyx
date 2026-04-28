use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

#[tokio::test]
async fn builds_sorted_directory_listing() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("docs")).await.unwrap();
    fs::write(root.join("README.md"), "# Garyx\n")
        .await
        .unwrap();
    fs::write(root.join("docs").join("guide.html"), "<h1>Guide</h1>")
        .await
        .unwrap();

    let listing = build_listing(root.display().to_string(), None)
        .await
        .unwrap();
    assert_eq!(listing.directory_path, "");
    assert_eq!(listing.entries.len(), 2);
    assert_eq!(listing.entries[0].entry_type, "directory");
    assert_eq!(listing.entries[0].name, "docs");
    assert_eq!(listing.entries[1].entry_type, "file");
    assert_eq!(listing.entries[1].name, "README.md");
    assert_eq!(
        listing.entries[1].media_type.as_deref(),
        Some("text/markdown")
    );
}

#[tokio::test]
async fn rejects_escape_paths() {
    let temp = tempdir().unwrap();
    let error = build_listing(temp.path().display().to_string(), Some("../etc".to_owned()))
        .await
        .unwrap_err();

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(error.message.contains("workspace root"));
}

#[tokio::test]
async fn previews_markdown_and_pdf() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    fs::write(
        root.join("README.md"),
        "# Hello\n```mermaid\ngraph TD\nA-->B\n```\n",
    )
    .await
    .unwrap();
    fs::write(root.join("deck.pdf"), b"%PDF-1.4\n%gary\n")
        .await
        .unwrap();

    let markdown = build_preview(root.display().to_string(), Some("README.md".to_owned()))
        .await
        .unwrap();
    assert_eq!(markdown.preview_kind, "markdown");
    assert!(markdown.text.unwrap().contains("mermaid"));

    let pdf = build_preview(root.display().to_string(), Some("deck.pdf".to_owned()))
        .await
        .unwrap();
    assert_eq!(pdf.preview_kind, "pdf");
    assert!(pdf.data_base64.is_some());
}

#[tokio::test]
async fn uploads_files_into_workspace_directory() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("uploads")).await.unwrap();

    let result = write_uploaded_files(UploadWorkspaceFilesBody {
        workspace_dir: root.display().to_string(),
        path: Some("uploads".to_owned()),
        files: vec![UploadWorkspaceFile {
            name: "note.txt".to_owned(),
            media_type: Some("text/plain".to_owned()),
            data_base64: BASE64.encode("hello world"),
        }],
    })
    .await
    .unwrap();

    assert_eq!(result.directory_path, "uploads");
    assert_eq!(result.uploaded_paths, vec!["uploads/note.txt"]);
    let saved = fs::read_to_string(root.join("uploads").join("note.txt"))
        .await
        .unwrap();
    assert_eq!(saved, "hello world");
}

#[cfg(unix)]
#[tokio::test]
async fn listing_tolerates_unreadable_child_directories() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    let restricted_dir = root.join("restricted");
    fs::create_dir_all(restricted_dir.join("nested"))
        .await
        .unwrap();
    fs::write(root.join("visible.txt"), "ok").await.unwrap();

    let mut permissions = std::fs::metadata(&restricted_dir).unwrap().permissions();
    permissions.set_mode(0o000);
    std::fs::set_permissions(&restricted_dir, permissions).unwrap();

    let listing = build_listing(root.display().to_string(), None)
        .await
        .unwrap();

    let mut restore_permissions = std::fs::metadata(&restricted_dir).unwrap().permissions();
    restore_permissions.set_mode(0o755);
    std::fs::set_permissions(&restricted_dir, restore_permissions).unwrap();

    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.name == "visible.txt")
    );
    let restricted_entry = listing
        .entries
        .iter()
        .find(|entry| entry.name == "restricted")
        .expect("restricted directory should still be listed");
    assert_eq!(restricted_entry.entry_type, "directory");
    assert!(!restricted_entry.has_children);
}
