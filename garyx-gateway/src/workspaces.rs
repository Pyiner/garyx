use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::fs;

use crate::garyx_db::{GaryxDbError, WorkspaceDraft, WorkspaceListEntry};
use crate::server::AppState;

const MAX_WORKSPACE_DIRECTORY_ENTRIES: usize = 500;

#[derive(Debug, Deserialize)]
pub struct WorkspaceMutationRequest {
    #[serde(default, alias = "workspaceDir", alias = "workspace_dir")]
    path: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceDeleteParams {
    #[serde(default, alias = "workspaceDir", alias = "workspace_dir")]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspacePinRequest {
    #[serde(default)]
    path: Option<String>,
    pinned: bool,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceRenameRequest {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceDirectoryParams {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDirectoryEntry {
    name: String,
    path: String,
    git_repo: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDirectoryListing {
    path: String,
    parent_path: Option<String>,
    entries: Vec<WorkspaceDirectoryEntry>,
}

/// Typed directory-listing failures. A request without `path` starts at the
/// gateway home; once a path is provided there are no silent fallbacks — the
/// client renders the error inline and stays where it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryListingErrorCode {
    InvalidPath,
    NotFound,
    NotADirectory,
    PermissionDenied,
}

impl DirectoryListingErrorCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::InvalidPath => "invalid_path",
            Self::NotFound => "not_found",
            Self::NotADirectory => "not_a_directory",
            Self::PermissionDenied => "permission_denied",
        }
    }
}

#[derive(Debug)]
enum DirectoryListingError {
    Typed {
        code: DirectoryListingErrorCode,
        message: String,
    },
    Io(std::io::Error),
}

impl DirectoryListingError {
    fn typed(code: DirectoryListingErrorCode, message: impl Into<String>) -> Self {
        Self::Typed {
            code,
            message: message.into(),
        }
    }

    fn from_io(error: std::io::Error, path: &Path) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::typed(
                DirectoryListingErrorCode::NotFound,
                format!("directory does not exist: {}", path.display()),
            ),
            std::io::ErrorKind::PermissionDenied => Self::typed(
                DirectoryListingErrorCode::PermissionDenied,
                format!("permission denied: {}", path.display()),
            ),
            _ => Self::Io(error),
        }
    }
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn workspace_path_key(path: &str) -> String {
    path.trim().replace('\\', "/")
}

fn is_absolute_workspace_path(path: &str) -> bool {
    let normalized = workspace_path_key(path);
    if normalized.starts_with('/') || normalized.starts_with("//") {
        return true;
    }
    let bytes = normalized.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn push_workspace_draft(
    drafts: &mut BTreeMap<String, WorkspaceDraft>,
    path: Option<&str>,
    name: Option<&str>,
) {
    let Some(path) = path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.replace('\\', "/"))
    else {
        return;
    };
    if !is_absolute_workspace_path(&path) {
        return;
    }
    let key = workspace_path_key(&path);
    drafts.entry(key).or_insert_with(|| WorkspaceDraft {
        name: name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        path,
    });
}

fn configured_workspace_drafts(state: &Arc<AppState>) -> BTreeMap<String, WorkspaceDraft> {
    let config = state.config_snapshot();
    let mut drafts = BTreeMap::new();
    for channel in config.channels.plugins.values() {
        for account in channel.accounts.values() {
            push_workspace_draft(
                &mut drafts,
                account.workspace_dir.as_deref(),
                account.name.as_deref(),
            );
        }
    }
    for job in &config.cron.jobs {
        push_workspace_draft(
            &mut drafts,
            job.workspace_dir.as_deref(),
            job.label.as_deref(),
        );
    }
    drafts
}

async fn seed_workspaces_from_configuration_if_empty(
    state: &Arc<AppState>,
) -> Result<(), GaryxDbError> {
    if state
        .ops
        .garyx_db
        .run_blocking(|db| db.count_workspace_rows())
        .await?
        > 0
    {
        return Ok(());
    }

    let mut drafts = configured_workspace_drafts(state);
    if let Some(service) = state.ops.cron_service.as_ref() {
        for job in service.list().await {
            push_workspace_draft(
                &mut drafts,
                job.workspace_dir.as_deref(),
                job.label.as_deref(),
            );
        }
    }
    let drafts: Vec<_> = drafts.into_values().collect();
    state
        .ops
        .garyx_db
        .run_blocking(move |db| db.seed_workspaces_if_empty(drafts))
        .await?;
    Ok(())
}

pub(crate) fn workspace_display_name(path: &str) -> String {
    crate::garyx_db::workspace_path_display_name(path)
}

fn rfc3339_from_micros(micros: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_micros(micros)
        .map(|timestamp| timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

async fn workspace_git_repo_flag(path: &str) -> bool {
    fs::metadata(Path::new(path).join(".git")).await.is_ok()
}

async fn workspace_response(
    workspaces: Vec<WorkspaceListEntry>,
    workspace_state_initialized: bool,
) -> serde_json::Value {
    let mut entries = Vec::with_capacity(workspaces.len());
    for workspace in workspaces {
        let git_repo = workspace_git_repo_flag(&workspace.path).await;
        entries.push(json!({
            "name": workspace.display_name(),
            "path": workspace.path,
            "pinned": workspace.pinned_at.is_some(),
            "thread_count": workspace.thread_count,
            "last_activity_at": workspace.last_activity_us.and_then(rfc3339_from_micros),
            "git_repo": git_repo,
        }));
    }
    json!({
        "workspace_state_initialized": workspace_state_initialized,
        "gateway_home": fallback_directory().to_string_lossy(),
        "workspaces": entries,
    })
}

fn workspace_error_response(error: GaryxDbError) -> (StatusCode, Json<serde_json::Value>) {
    let status = match error {
        GaryxDbError::BadRequest(_) => StatusCode::BAD_REQUEST,
        GaryxDbError::NotFound(_) => StatusCode::NOT_FOUND,
        GaryxDbError::ThreadArchived(_) => StatusCode::GONE,
        GaryxDbError::LockPoisoned
        | GaryxDbError::Join(_)
        | GaryxDbError::Configuration(_)
        | GaryxDbError::DataDirLocked { .. }
        | GaryxDbError::ParentHandoffTimedOut { .. }
        | GaryxDbError::Io(_)
        | GaryxDbError::Sqlite(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": error.to_string() })))
}

async fn workspace_list_response(state: &Arc<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(error) = seed_workspaces_from_configuration_if_empty(state).await {
        return workspace_error_response(error);
    }
    let listed = state
        .ops
        .garyx_db
        .run_blocking(|db| {
            let count = db.count_workspace_rows()?;
            let workspaces = db.list_workspaces_with_stats()?;
            Ok((count, workspaces))
        })
        .await;
    let (workspace_state_initialized, workspaces) = match listed {
        Ok((count, workspaces)) => (count > 0, workspaces),
        Err(error) => return workspace_error_response(error),
    };
    (
        StatusCode::OK,
        Json(workspace_response(workspaces, workspace_state_initialized).await),
    )
}

pub async fn list_workspaces(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    workspace_list_response(&state).await
}

pub async fn list_workspace_directories(
    Query(params): Query<WorkspaceDirectoryParams>,
) -> impl IntoResponse {
    match build_directory_listing(params.path).await {
        Ok(listing) => (StatusCode::OK, Json(json!(listing))).into_response(),
        Err(DirectoryListingError::Typed { code, message }) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": message, "code": code.as_str() })),
        )
            .into_response(),
        Err(DirectoryListingError::Io(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

pub async fn upsert_workspace(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceMutationRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let path = match trim_optional(body.path) {
        Some(path) => path,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "workspace path is required" })),
            );
        }
    };
    let name = trim_optional(body.name);

    if let Err(error) = state
        .ops
        .garyx_db
        .run_blocking(move |db| db.upsert_workspace(WorkspaceDraft { name, path }))
        .await
    {
        return workspace_error_response(error);
    }

    workspace_list_response(&state).await
}

/// Active-row-only point mutation: never creates or revives a workspace row.
pub async fn pin_workspace(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspacePinRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let path = match trim_optional(body.path) {
        Some(path) => path,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "workspace path is required" })),
            );
        }
    };
    let pinned = body.pinned;
    if let Err(error) = state
        .ops
        .garyx_db
        .run_blocking(move |db| db.set_workspace_pinned(&path, pinned))
        .await
    {
        return workspace_error_response(error);
    }
    workspace_list_response(&state).await
}

/// Active-row-only point mutation: never creates or revives a workspace row.
pub async fn rename_workspace(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceRenameRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let path = match trim_optional(body.path) {
        Some(path) => path,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "workspace path is required" })),
            );
        }
    };
    let name = match trim_optional(body.name) {
        Some(name) => name,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "workspace name is required" })),
            );
        }
    };
    if let Err(error) = state
        .ops
        .garyx_db
        .run_blocking(move |db| db.rename_workspace(&path, &name))
        .await
    {
        return workspace_error_response(error);
    }
    workspace_list_response(&state).await
}

pub async fn delete_workspace(
    State(state): State<Arc<AppState>>,
    Query(params): Query<WorkspaceDeleteParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    let path = match trim_optional(params.path) {
        Some(path) => path,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "workspace path is required" })),
            );
        }
    };

    let path_key = workspace_path_key(&path);
    if let Err(error) = state
        .ops
        .garyx_db
        .run_blocking(move |db| db.delete_workspace(&path_key).map(|_| ()))
        .await
    {
        return workspace_error_response(error);
    }

    workspace_list_response(&state).await
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_display_name_uses_path_tail() {
        assert_eq!(workspace_display_name("/workspace/repo/"), "repo");
        assert_eq!(workspace_display_name("C:\\workspace\\repo"), "repo");
    }

    #[tokio::test]
    async fn directory_listing_returns_visible_child_directories_only() {
        let temp = tempfile::tempdir().expect("temp dir");
        fs::create_dir(temp.path().join("alpha")).await.unwrap();
        fs::create_dir(temp.path().join(".hidden")).await.unwrap();
        fs::write(temp.path().join("note.txt"), "not a directory")
            .await
            .unwrap();

        let listing = build_directory_listing(Some(temp.path().to_string_lossy().to_string()))
            .await
            .unwrap();

        assert_eq!(listing.path, temp.path().to_string_lossy().to_string());
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "alpha");
        assert_eq!(
            listing.entries[0].path,
            temp.path().join("alpha").to_string_lossy().to_string()
        );
        assert!(!listing.entries[0].git_repo);
    }

    #[tokio::test]
    async fn directory_listing_flags_git_repository_roots() {
        let temp = tempfile::tempdir().expect("temp dir");
        fs::create_dir_all(temp.path().join("repo/.git")).await.unwrap();
        fs::create_dir(temp.path().join("plain")).await.unwrap();

        let listing = build_directory_listing(Some(temp.path().to_string_lossy().to_string()))
            .await
            .unwrap();
        let flags: Vec<(&str, bool)> = listing
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.git_repo))
            .collect();
        assert_eq!(flags, vec![("plain", false), ("repo", true)]);
    }

    fn typed_code(error: DirectoryListingError) -> &'static str {
        match error {
            DirectoryListingError::Typed { code, .. } => code.as_str(),
            DirectoryListingError::Io(error) => panic!("expected typed error, got io: {error}"),
        }
    }

    #[tokio::test]
    async fn directory_listing_rejects_bad_paths_with_typed_codes() {
        let temp = tempfile::tempdir().expect("temp dir");
        fs::write(temp.path().join("note.txt"), "file").await.unwrap();

        let relative = build_directory_listing(Some("relative/path".to_owned()))
            .await
            .expect_err("relative path is rejected");
        assert_eq!(typed_code(relative), "invalid_path");

        let missing = build_directory_listing(Some(
            temp.path().join("missing").to_string_lossy().to_string(),
        ))
        .await
        .expect_err("missing path is rejected");
        assert_eq!(typed_code(missing), "not_found");

        let file = build_directory_listing(Some(
            temp.path().join("note.txt").to_string_lossy().to_string(),
        ))
        .await
        .expect_err("file path is rejected");
        assert_eq!(typed_code(file), "not_a_directory");
    }

    #[tokio::test]
    async fn directory_listing_without_path_starts_at_the_gateway_home() {
        let listing = build_directory_listing(None).await.expect("home listing");
        assert_eq!(
            listing.path,
            fallback_directory().to_string_lossy().to_string()
        );
    }
}

fn fallback_directory() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// No `path` → the listing starts at the gateway home. A provided path is
/// validated strictly: relative, missing, non-directory, and unreadable
/// paths are typed 400s, never silent fallbacks.
async fn resolve_directory_path(path: Option<String>) -> Result<PathBuf, DirectoryListingError> {
    let requested = path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(requested) = requested else {
        return Ok(fallback_directory());
    };
    let candidate = PathBuf::from(requested);
    if !candidate.is_absolute() {
        return Err(DirectoryListingError::typed(
            DirectoryListingErrorCode::InvalidPath,
            format!("path must be absolute: {requested}"),
        ));
    }
    let metadata = fs::metadata(&candidate)
        .await
        .map_err(|error| DirectoryListingError::from_io(error, &candidate))?;
    if !metadata.is_dir() {
        return Err(DirectoryListingError::typed(
            DirectoryListingErrorCode::NotADirectory,
            format!("not a directory: {}", candidate.display()),
        ));
    }
    Ok(candidate)
}

async fn build_directory_listing(
    path: Option<String>,
) -> Result<WorkspaceDirectoryListing, DirectoryListingError> {
    let directory_path = resolve_directory_path(path).await?;
    let parent_path = directory_path
        .parent()
        .filter(|parent| *parent != directory_path.as_path())
        .map(|parent| parent.to_string_lossy().to_string());
    let mut reader = fs::read_dir(&directory_path)
        .await
        .map_err(|error| DirectoryListingError::from_io(error, &directory_path))?;
    let mut entries = Vec::new();

    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|error| DirectoryListingError::from_io(error, &directory_path))?
    {
        if entries.len() >= MAX_WORKSPACE_DIRECTORY_ENTRIES {
            break;
        }
        let file_type = match entry.file_type().await {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let git_repo = workspace_git_repo_flag(&entry.path().to_string_lossy()).await;
        entries.push(WorkspaceDirectoryEntry {
            name,
            path: entry.path().to_string_lossy().to_string(),
            git_repo,
        });
    }

    entries.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });

    Ok(WorkspaceDirectoryListing {
        path: directory_path.to_string_lossy().to_string(),
        parent_path,
        entries,
    })
}
