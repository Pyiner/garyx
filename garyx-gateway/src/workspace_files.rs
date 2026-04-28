use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use garyx_models::provider::PromptAttachmentKind;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt;

use crate::server::AppState;

const MAX_DIRECTORY_ENTRIES: usize = 500;
const MAX_TEXT_PREVIEW_BYTES: usize = 512 * 1024;
const MAX_BINARY_PREVIEW_BYTES: usize = 12 * 1024 * 1024;
const MAX_UPLOAD_FILE_BYTES: usize = 25 * 1024 * 1024;
const MAX_UPLOAD_FILES: usize = 24;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileQuery {
    pub workspace_dir: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadWorkspaceFilesBody {
    pub workspace_dir: String,
    #[serde(default)]
    pub path: Option<String>,
    pub files: Vec<UploadWorkspaceFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadChatAttachmentsBody {
    pub files: Vec<UploadChatAttachment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadWorkspaceFile {
    pub name: String,
    #[serde(default)]
    pub media_type: Option<String>,
    pub data_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadChatAttachment {
    pub kind: PromptAttachmentKind,
    pub name: String,
    #[serde(default)]
    pub media_type: Option<String>,
    pub data_base64: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileEntry {
    pub path: String,
    pub name: String,
    pub entry_type: String,
    pub size: Option<u64>,
    pub modified_at: Option<String>,
    pub media_type: Option<String>,
    pub has_children: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileListing {
    pub workspace_dir: String,
    pub directory_path: String,
    pub entries: Vec<WorkspaceFileEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilePreview {
    pub workspace_dir: String,
    pub path: String,
    pub name: String,
    pub media_type: String,
    pub preview_kind: String,
    pub size: u64,
    pub modified_at: Option<String>,
    pub truncated: bool,
    pub text: Option<String>,
    pub data_base64: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadWorkspaceFilesResult {
    pub workspace_dir: String,
    pub directory_path: String,
    pub uploaded_paths: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadChatAttachmentsResult {
    pub files: Vec<UploadedChatAttachment>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedChatAttachment {
    pub kind: PromptAttachmentKind,
    pub path: String,
    pub name: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewKind {
    Markdown,
    Html,
    Text,
    Pdf,
    Image,
    Unsupported,
}

pub async fn list_workspace_files(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<WorkspaceFileQuery>,
) -> impl IntoResponse {
    match build_listing(query.workspace_dir, query.path).await {
        Ok(listing) => (StatusCode::OK, Json(json!(listing))).into_response(),
        Err(error) => error.into_response(),
    }
}

pub async fn preview_workspace_file(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<WorkspaceFileQuery>,
) -> impl IntoResponse {
    match build_preview(query.workspace_dir, query.path).await {
        Ok(preview) => (StatusCode::OK, Json(json!(preview))).into_response(),
        Err(error) => error.into_response(),
    }
}

pub async fn upload_workspace_files(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<UploadWorkspaceFilesBody>,
) -> impl IntoResponse {
    match write_uploaded_files(body).await {
        Ok(result) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(error) => error.into_response(),
    }
}

pub async fn upload_chat_attachments(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<UploadChatAttachmentsBody>,
) -> impl IntoResponse {
    match write_uploaded_chat_attachments(body).await {
        Ok(result) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn build_listing(
    workspace_dir: String,
    directory_path: Option<String>,
) -> Result<WorkspaceFileListing, WorkspaceFileError> {
    let workspace_root = resolve_workspace_root(&workspace_dir).await?;
    let relative = normalize_relative_path(directory_path.as_deref())?;
    let directory = resolve_existing_path(&workspace_root, &relative, true).await?;
    let canonical_directory = fs::canonicalize(&directory)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;
    ensure_within_root(&workspace_root, &canonical_directory)?;

    let mut entries = Vec::new();
    let mut reader = fs::read_dir(&canonical_directory)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;

    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?
    {
        if entries.len() >= MAX_DIRECTORY_ENTRIES {
            break;
        }

        let Some(item) = describe_directory_entry(&workspace_root, &entry.path()).await? else {
            continue;
        };
        entries.push(item);
    }

    entries.sort_by(|left, right| {
        left.entry_type
            .cmp(&right.entry_type)
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then_with(|| left.path.cmp(&right.path))
    });

    Ok(WorkspaceFileListing {
        workspace_dir: workspace_root.display().to_string(),
        directory_path: relative_path_string(&relative),
        entries,
    })
}

async fn build_preview(
    workspace_dir: String,
    file_path: Option<String>,
) -> Result<WorkspaceFilePreview, WorkspaceFileError> {
    let workspace_root = resolve_workspace_root(&workspace_dir).await?;
    let relative = normalize_relative_path(file_path.as_deref())?;
    if relative.as_os_str().is_empty() {
        return Err(WorkspaceFileError::bad_request("file path is required"));
    }

    let file_path = resolve_existing_path(&workspace_root, &relative, false).await?;
    let canonical_file = fs::canonicalize(&file_path)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;
    ensure_within_root(&workspace_root, &canonical_file)?;

    let metadata = fs::metadata(&canonical_file)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;
    if !metadata.is_file() {
        return Err(WorkspaceFileError::bad_request(
            "file preview requires a file path",
        ));
    }

    let name = canonical_file
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("file")
        .to_owned();
    let media_type = detect_media_type(&name);
    let preview_kind = detect_preview_kind(&name, &media_type);
    let modified_at = metadata.modified().ok().map(format_system_time);
    let size = metadata.len();

    match preview_kind {
        PreviewKind::Markdown | PreviewKind::Html | PreviewKind::Text => {
            let (buf, truncated) =
                read_limited_file(&canonical_file, MAX_TEXT_PREVIEW_BYTES).await?;
            let text = String::from_utf8_lossy(&buf).into_owned();
            Ok(WorkspaceFilePreview {
                workspace_dir: workspace_root.display().to_string(),
                path: relative_path_string(&relative),
                name,
                media_type,
                preview_kind: preview_kind_label(preview_kind).to_owned(),
                size,
                modified_at,
                truncated,
                text: Some(text),
                data_base64: None,
            })
        }
        PreviewKind::Pdf | PreviewKind::Image => {
            let (buf, truncated) =
                read_limited_file(&canonical_file, MAX_BINARY_PREVIEW_BYTES).await?;
            Ok(WorkspaceFilePreview {
                workspace_dir: workspace_root.display().to_string(),
                path: relative_path_string(&relative),
                name,
                media_type,
                preview_kind: preview_kind_label(preview_kind).to_owned(),
                size,
                modified_at,
                truncated,
                text: None,
                data_base64: Some(BASE64.encode(buf)),
            })
        }
        PreviewKind::Unsupported => Ok(WorkspaceFilePreview {
            workspace_dir: workspace_root.display().to_string(),
            path: relative_path_string(&relative),
            name,
            media_type,
            preview_kind: preview_kind_label(preview_kind).to_owned(),
            size,
            modified_at,
            truncated: false,
            text: None,
            data_base64: None,
        }),
    }
}

async fn write_uploaded_files(
    body: UploadWorkspaceFilesBody,
) -> Result<UploadWorkspaceFilesResult, WorkspaceFileError> {
    if body.files.is_empty() {
        return Err(WorkspaceFileError::bad_request(
            "at least one file is required",
        ));
    }
    if body.files.len() > MAX_UPLOAD_FILES {
        return Err(WorkspaceFileError::bad_request(
            "too many files in one upload",
        ));
    }

    let workspace_root = resolve_workspace_root(&body.workspace_dir).await?;
    let relative = normalize_relative_path(body.path.as_deref())?;
    let target_dir = resolve_existing_path(&workspace_root, &relative, true).await?;
    let canonical_target_dir = fs::canonicalize(&target_dir)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;
    ensure_within_root(&workspace_root, &canonical_target_dir)?;

    let mut uploaded_paths = Vec::with_capacity(body.files.len());

    for file in body.files {
        let file_name = validate_upload_name(&file.name)?;
        let decoded = decode_uploaded_bytes(&file.data_base64)?;

        let destination = canonical_target_dir.join(&file_name);
        if destination.exists() {
            return Err(WorkspaceFileError::conflict(format!(
                "{file_name} already exists"
            )));
        }

        fs::write(&destination, decoded)
            .await
            .map_err(|error| WorkspaceFileError::io(StatusCode::INTERNAL_SERVER_ERROR, error))?;

        let rel = destination
            .strip_prefix(&workspace_root)
            .unwrap_or(destination.as_path());
        uploaded_paths.push(relative_path_string(rel));
    }

    Ok(UploadWorkspaceFilesResult {
        workspace_dir: workspace_root.display().to_string(),
        directory_path: relative_path_string(&relative),
        uploaded_paths,
    })
}

async fn write_uploaded_chat_attachments(
    body: UploadChatAttachmentsBody,
) -> Result<UploadChatAttachmentsResult, WorkspaceFileError> {
    if body.files.is_empty() {
        return Err(WorkspaceFileError::bad_request(
            "at least one file is required",
        ));
    }
    if body.files.len() > MAX_UPLOAD_FILES {
        return Err(WorkspaceFileError::bad_request(
            "too many uploaded files in a single request",
        ));
    }

    let target_dir = std::env::temp_dir()
        .join("garyx-gateway")
        .join("prompt-attachments");
    fs::create_dir_all(&target_dir)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::INTERNAL_SERVER_ERROR, error))?;

    let mut uploaded = Vec::with_capacity(body.files.len());
    for file in body.files {
        let file_name = validate_upload_name(&file.name)?;
        let decoded = decode_uploaded_bytes(&file.data_base64)?;
        let media_type = file
            .media_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| detect_media_type(&file_name));
        let destination = target_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), file_name));

        fs::write(&destination, decoded)
            .await
            .map_err(|error| WorkspaceFileError::io(StatusCode::INTERNAL_SERVER_ERROR, error))?;

        uploaded.push(UploadedChatAttachment {
            kind: file.kind,
            path: destination.to_string_lossy().to_string(),
            name: file_name,
            media_type: media_type.clone(),
        });
    }

    Ok(UploadChatAttachmentsResult { files: uploaded })
}

async fn describe_directory_entry(
    workspace_root: &Path,
    entry_path: &Path,
) -> Result<Option<WorkspaceFileEntry>, WorkspaceFileError> {
    let file_type = match fs::symlink_metadata(entry_path).await {
        Ok(metadata) => metadata.file_type(),
        Err(error) if is_permission_error(&error) => return Ok(None),
        Err(error) => return Err(WorkspaceFileError::io(StatusCode::BAD_REQUEST, error)),
    };

    let canonical = if file_type.is_symlink() {
        let canonical = match fs::canonicalize(entry_path).await {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        if !canonical.starts_with(workspace_root) {
            return Ok(None);
        }
        canonical
    } else {
        entry_path.to_path_buf()
    };

    ensure_within_root(workspace_root, &canonical)?;
    let metadata = match fs::metadata(&canonical).await {
        Ok(metadata) => metadata,
        Err(error) if is_permission_error(&error) => return Ok(None),
        Err(error) => return Err(WorkspaceFileError::io(StatusCode::BAD_REQUEST, error)),
    };

    let relative = entry_path
        .strip_prefix(workspace_root)
        .unwrap_or(entry_path);
    let name = entry_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_owned();

    if metadata.is_dir() {
        return Ok(Some(WorkspaceFileEntry {
            path: relative_path_string(relative),
            name,
            entry_type: "directory".to_owned(),
            size: None,
            modified_at: metadata.modified().ok().map(format_system_time),
            media_type: None,
            has_children: directory_has_children(&canonical).await?,
        }));
    }

    Ok(Some(WorkspaceFileEntry {
        path: relative_path_string(relative),
        name: name.clone(),
        entry_type: "file".to_owned(),
        size: Some(metadata.len()),
        modified_at: metadata.modified().ok().map(format_system_time),
        media_type: Some(detect_media_type(&name)),
        has_children: false,
    }))
}

async fn directory_has_children(path: &Path) -> Result<bool, WorkspaceFileError> {
    let mut reader = match fs::read_dir(path).await {
        Ok(reader) => reader,
        Err(error) if is_permission_error(&error) => return Ok(false),
        Err(error) => return Err(WorkspaceFileError::io(StatusCode::BAD_REQUEST, error)),
    };
    match reader.next_entry().await {
        Ok(entry) => Ok(entry.is_some()),
        Err(error) if is_permission_error(&error) => Ok(false),
        Err(error) => Err(WorkspaceFileError::io(StatusCode::BAD_REQUEST, error)),
    }
}

async fn resolve_workspace_root(workspace_dir: &str) -> Result<PathBuf, WorkspaceFileError> {
    let trimmed = workspace_dir.trim();
    if trimmed.is_empty() {
        return Err(WorkspaceFileError::bad_request("workspace_dir is required"));
    }

    let canonical = fs::canonicalize(trimmed)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;
    let metadata = fs::metadata(&canonical)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;
    if !metadata.is_dir() {
        return Err(WorkspaceFileError::bad_request(
            "workspace_dir must point to a directory",
        ));
    }
    Ok(canonical)
}

fn normalize_relative_path(path: Option<&str>) -> Result<PathBuf, WorkspaceFileError> {
    let trimmed = path.unwrap_or("").trim();
    if trimmed.is_empty() {
        return Ok(PathBuf::new());
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(WorkspaceFileError::bad_request(
                    "path must stay within the workspace root",
                ));
            }
        }
    }
    Ok(normalized)
}

async fn resolve_existing_path(
    workspace_root: &Path,
    relative: &Path,
    expect_directory: bool,
) -> Result<PathBuf, WorkspaceFileError> {
    let path = if relative.as_os_str().is_empty() {
        workspace_root.to_path_buf()
    } else {
        workspace_root.join(relative)
    };

    let metadata = fs::metadata(&path)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;

    if expect_directory && !metadata.is_dir() {
        return Err(WorkspaceFileError::bad_request(
            "path must point to a directory",
        ));
    }
    if !expect_directory && !metadata.is_file() {
        return Err(WorkspaceFileError::bad_request("path must point to a file"));
    }

    Ok(path)
}

fn ensure_within_root(workspace_root: &Path, candidate: &Path) -> Result<(), WorkspaceFileError> {
    if candidate.starts_with(workspace_root) {
        Ok(())
    } else {
        Err(WorkspaceFileError::bad_request(
            "path must stay within the workspace root",
        ))
    }
}

async fn read_limited_file(
    path: &Path,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool), WorkspaceFileError> {
    let file = File::open(path)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;
    let mut buf = Vec::new();
    file.take((max_bytes + 1) as u64)
        .read_to_end(&mut buf)
        .await
        .map_err(|error| WorkspaceFileError::io(StatusCode::BAD_REQUEST, error))?;
    let truncated = buf.len() > max_bytes;
    if truncated {
        buf.truncate(max_bytes);
    }
    Ok((buf, truncated))
}

fn decode_uploaded_bytes(data_base64: &str) -> Result<Vec<u8>, WorkspaceFileError> {
    let decoded = BASE64
        .decode(data_base64.trim())
        .map_err(|_| WorkspaceFileError::bad_request("uploaded file is not valid base64"))?;
    if decoded.is_empty() {
        return Err(WorkspaceFileError::bad_request("uploaded file is empty"));
    }
    if decoded.len() > MAX_UPLOAD_FILE_BYTES {
        return Err(WorkspaceFileError::bad_request(
            "uploaded file exceeds the size limit",
        ));
    }
    Ok(decoded)
}

fn validate_upload_name(name: &str) -> Result<String, WorkspaceFileError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(WorkspaceFileError::bad_request(
            "uploaded file name is required",
        ));
    }

    let path = Path::new(trimmed);
    let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
        return Err(WorkspaceFileError::bad_request(
            "uploaded file name is invalid",
        ));
    };
    if file_name != trimmed || file_name == "." || file_name == ".." {
        return Err(WorkspaceFileError::bad_request(
            "uploaded file name is invalid",
        ));
    }
    Ok(file_name.to_owned())
}

fn relative_path_string(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        String::new()
    } else {
        path.to_string_lossy().replace('\\', "/")
    }
}

fn format_system_time(value: std::time::SystemTime) -> String {
    let dt: DateTime<Utc> = value.into();
    dt.to_rfc3339()
}

fn preview_kind_label(kind: PreviewKind) -> &'static str {
    match kind {
        PreviewKind::Markdown => "markdown",
        PreviewKind::Html => "html",
        PreviewKind::Text => "text",
        PreviewKind::Pdf => "pdf",
        PreviewKind::Image => "image",
        PreviewKind::Unsupported => "unsupported",
    }
}

fn detect_preview_kind(name: &str, media_type: &str) -> PreviewKind {
    let lower_name = name.trim().to_ascii_lowercase();
    if lower_name.ends_with(".md")
        || lower_name.ends_with(".markdown")
        || media_type == "text/markdown"
    {
        return PreviewKind::Markdown;
    }
    if lower_name.ends_with(".html") || lower_name.ends_with(".htm") || media_type == "text/html" {
        return PreviewKind::Html;
    }
    if lower_name.ends_with(".pdf") || media_type == "application/pdf" {
        return PreviewKind::Pdf;
    }
    if media_type.starts_with("image/") {
        return PreviewKind::Image;
    }
    if media_type.starts_with("text/")
        || matches!(
            lower_name.rsplit('.').next(),
            Some(
                "txt"
                    | "json"
                    | "jsonl"
                    | "yaml"
                    | "yml"
                    | "toml"
                    | "csv"
                    | "tsv"
                    | "log"
                    | "rs"
                    | "ts"
                    | "tsx"
                    | "js"
                    | "jsx"
                    | "css"
                    | "scss"
                    | "py"
                    | "go"
                    | "java"
                    | "kt"
                    | "swift"
                    | "sh"
                    | "sql"
                    | "xml"
            )
        )
    {
        return PreviewKind::Text;
    }
    PreviewKind::Unsupported
}

fn detect_media_type(name: &str) -> String {
    let lower_name = name.trim().to_ascii_lowercase();
    if lower_name.ends_with(".md") || lower_name.ends_with(".markdown") {
        return "text/markdown".to_owned();
    }
    if lower_name.ends_with(".html") || lower_name.ends_with(".htm") {
        return "text/html".to_owned();
    }
    if lower_name.ends_with(".pdf") {
        return "application/pdf".to_owned();
    }
    if lower_name.ends_with(".png") {
        return "image/png".to_owned();
    }
    if lower_name.ends_with(".jpg") || lower_name.ends_with(".jpeg") {
        return "image/jpeg".to_owned();
    }
    if lower_name.ends_with(".gif") {
        return "image/gif".to_owned();
    }
    if lower_name.ends_with(".webp") {
        return "image/webp".to_owned();
    }
    if lower_name.ends_with(".svg") {
        return "image/svg+xml".to_owned();
    }
    if lower_name.ends_with(".json") || lower_name.ends_with(".jsonl") {
        return "application/json".to_owned();
    }
    if lower_name.ends_with(".yaml") || lower_name.ends_with(".yml") {
        return "application/yaml".to_owned();
    }
    if lower_name.ends_with(".xml") {
        return "application/xml".to_owned();
    }
    if lower_name.ends_with(".csv") || lower_name.ends_with(".tsv") {
        return "text/csv".to_owned();
    }
    if matches!(
        lower_name.rsplit('.').next(),
        Some(
            "txt"
                | "toml"
                | "log"
                | "rs"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "css"
                | "scss"
                | "py"
                | "go"
                | "java"
                | "kt"
                | "swift"
                | "sh"
                | "sql"
        )
    ) {
        return "text/plain".to_owned();
    }
    "application/octet-stream".to_owned()
}

#[derive(Debug)]
struct WorkspaceFileError {
    status: StatusCode,
    message: String,
}

impl WorkspaceFileError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn io(status: StatusCode, error: std::io::Error) -> Self {
        Self {
            status,
            message: if is_permission_error(&error) {
                "Access to this file or folder is blocked by macOS permissions.".to_owned()
            } else {
                error.to_string()
            },
        }
    }
}

fn is_permission_error(error: &std::io::Error) -> bool {
    matches!(error.kind(), std::io::ErrorKind::PermissionDenied)
        || matches!(error.raw_os_error(), Some(1 | 13))
}

impl IntoResponse for WorkspaceFileError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests;
