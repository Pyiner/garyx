use super::*;
use axum::{
    Json, Router, body::Body, extract::DefaultBodyLimit, http::Request, response::IntoResponse,
    routing::post,
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::tempdir;
use tower::ServiceExt;

use crate::garyx_db::PromptAttachmentState;
use crate::server::AppStateBuilder;
use garyx_models::config::GaryxConfig;

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

#[tokio::test]
async fn upload_route_accepts_large_phone_photo_payloads() {
    async fn accept_upload(Json(body): Json<UploadChatAttachmentsBody>) -> impl IntoResponse {
        (
            StatusCode::OK,
            Json(json!({ "fileCount": body.files.len() })),
        )
    }

    let payload = json!({
        "files": [{
            "kind": "image",
            "name": "photo.jpg",
            "mediaType": "image/jpeg",
            "dataBase64": BASE64.encode(vec![7_u8; 2_200_000]),
        }]
    })
    .to_string();
    assert!(payload.len() > 2 * 1024 * 1024);

    let router = Router::new().route(
        "/upload",
        post(accept_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY_BYTES)),
    );
    let request = Request::builder()
        .method("POST")
        .uri("/upload")
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn chat_attachment_upload_creates_scoped_managed_row() {
    let state = AppStateBuilder::new(GaryxConfig::default()).build();
    let scope = IdempotencyScope {
        identity: "upload-contract-test".to_owned(),
        epoch: 7,
    };
    let result = write_uploaded_chat_attachments(
        &state,
        UploadChatAttachmentsBody {
            idempotency_scope: Some(scope.clone()),
            files: vec![UploadChatAttachment {
                kind: PromptAttachmentKind::File,
                name: "notes.txt".to_owned(),
                media_type: Some("text/plain".to_owned()),
                data_base64: BASE64.encode("durable payload"),
            }],
        },
    )
    .await
    .unwrap();

    assert_eq!(result.files.len(), 1);
    let uploaded = &result.files[0];
    assert!(uploaded.attachment_id.starts_with("attachment:"));
    assert!(uploaded.path.contains("prompt-attachments-v1"));
    assert!(Path::new(&uploaded.path).is_file());
    let row = state
        .ops
        .garyx_db
        .prompt_attachment_by_id(&uploaded.attachment_id)
        .unwrap()
        .expect("upload ownership row");
    assert_eq!(row.scope_identity, scope.identity);
    assert_eq!(row.scope_epoch, scope.epoch);
    assert_eq!(row.state, PromptAttachmentState::Ready);
    assert_eq!(row.original_name, "notes.txt");
    assert_eq!(row.media_type, "text/plain");
    assert_eq!(row.byte_size, 15);
}

/// RED reproduction for #TASK-2511.
///
/// The affected transcript stores the original image name/media type next to
/// the managed attachment path, but the iOS echo loader can only dereference
/// that path through the workspace-file preview route. Managed storage names
/// the immutable payload `payload`, so this exercises the real upload ->
/// transcript-path -> preview shape without committing the user's image.
#[tokio::test]
async fn managed_chat_image_remains_previewable_for_transcript_echo() {
    let data_dir = tempdir().unwrap();
    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(data_dir.path().join("data").display().to_string());
    let state = AppStateBuilder::new(config).build();
    let jpeg = vec![0xff, 0xd8, 0xff, 0xd9];

    let result = write_uploaded_chat_attachments(
        &state,
        UploadChatAttachmentsBody {
            idempotency_scope: Some(IdempotencyScope {
                identity: "image-echo-repro".to_owned(),
                epoch: 1,
            }),
            files: vec![UploadChatAttachment {
                kind: PromptAttachmentKind::Image,
                name: "photo-1.jpg".to_owned(),
                media_type: Some("image/jpeg".to_owned()),
                data_base64: BASE64.encode(&jpeg),
            }],
        },
    )
    .await
    .unwrap();

    let uploaded = result.files.first().expect("uploaded image");
    let managed_path = Path::new(&uploaded.path);
    assert_eq!(
        managed_path.file_name().and_then(|name| name.to_str()),
        Some("payload")
    );
    let preview = build_preview(
        managed_path
            .parent()
            .expect("managed attachment directory")
            .display()
            .to_string(),
        Some("payload".to_owned()),
    )
    .await
    .unwrap();

    assert_eq!(
        preview.preview_kind, "image",
        "a managed image referenced by a committed user message must load as an image thumbnail"
    );
    assert_eq!(preview.media_type, "image/jpeg");
    assert_eq!(preview.data_base64, Some(BASE64.encode(jpeg)));
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
    assert!(restricted_entry.has_children);
}

#[test]
fn macos_protected_app_data_detection_is_limited_to_home_library() {
    let home = Path::new("/Users/test");

    assert!(is_macos_protected_app_data_path_for_home(
        Path::new("/Users/test/Library"),
        home
    ));
    assert!(is_macos_protected_app_data_path_for_home(
        Path::new("/Users/test/Library/Application Support/ExampleApp"),
        home
    ));
    assert!(is_macos_protected_app_data_path_for_home(
        Path::new("/Users/test/Library/Containers/com.example.App"),
        home
    ));
    assert!(!is_macos_protected_app_data_path_for_home(
        Path::new("/Users/test/repos/project/Library"),
        home
    ));
    assert!(!is_macos_protected_app_data_path_for_home(
        Path::new("/Users/test/.garyx"),
        home
    ));
}
