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

use crate::garyx_db::{GaryxDbError, WorkspaceDraft, WorkspaceRecord};
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
pub struct WorkspaceDirectoryParams {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDirectoryEntry {
    name: String,
    path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDirectoryListing {
    path: String,
    parent_path: Option<String>,
    entries: Vec<WorkspaceDirectoryEntry>,
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
    if state.ops.garyx_db.count_workspace_rows()? > 0 {
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
    state
        .ops
        .garyx_db
        .seed_workspaces_if_empty(drafts.into_values().collect())?;
    Ok(())
}

pub(crate) fn workspace_display_name(path: &str) -> String {
    path.trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_owned()
}

fn workspace_response(
    workspaces: Vec<WorkspaceRecord>,
    workspace_state_initialized: bool,
) -> serde_json::Value {
    json!({
        "workspace_state_initialized": workspace_state_initialized,
        "workspaces": workspaces
            .into_iter()
            .map(|workspace| {
                let display_name = workspace
                    .name
                    .clone()
                    .unwrap_or_else(|| workspace_display_name(&workspace.path));
                json!({
                    "name": display_name,
                    "path": workspace.path,
                })
            })
            .collect::<Vec<_>>()
    })
}

fn workspace_error_response(error: GaryxDbError) -> (StatusCode, Json<serde_json::Value>) {
    let status = match error {
        GaryxDbError::BadRequest(_) => StatusCode::BAD_REQUEST,
        GaryxDbError::LockPoisoned
        | GaryxDbError::Join(_)
        | GaryxDbError::Io(_)
        | GaryxDbError::Sqlite(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": error.to_string() })))
}

async fn workspace_list_response(state: &Arc<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(error) = seed_workspaces_from_configuration_if_empty(state).await {
        return workspace_error_response(error);
    }
    let workspace_state_initialized = match state.ops.garyx_db.count_workspace_rows() {
        Ok(count) => count > 0,
        Err(error) => return workspace_error_response(error),
    };
    let workspaces = match state.ops.garyx_db.list_workspaces() {
        Ok(workspaces) => workspaces,
        Err(error) => return workspace_error_response(error),
    };
    (
        StatusCode::OK,
        Json(workspace_response(workspaces, workspace_state_initialized)),
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
        Err(error) => (
            StatusCode::BAD_REQUEST,
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
        .upsert_workspace(WorkspaceDraft { name, path })
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

    if let Err(error) = state
        .ops
        .garyx_db
        .delete_workspace(&workspace_path_key(&path))
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
    }
}

fn fallback_directory() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

async fn resolve_directory_path(path: Option<String>) -> std::io::Result<PathBuf> {
    let fallback = fallback_directory();
    let requested = path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let candidate = requested
        .map(PathBuf::from)
        .filter(|value| value.is_absolute())
        .unwrap_or(fallback);
    match fs::metadata(&candidate).await {
        Ok(metadata) if metadata.is_dir() => Ok(candidate),
        Ok(_) => Ok(candidate
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(fallback_directory)),
        Err(_) => Ok(fallback_directory()),
    }
}

async fn build_directory_listing(
    path: Option<String>,
) -> std::io::Result<WorkspaceDirectoryListing> {
    let directory_path = resolve_directory_path(path).await?;
    let parent_path = directory_path
        .parent()
        .filter(|parent| *parent != directory_path.as_path())
        .map(|parent| parent.to_string_lossy().to_string());
    let mut reader = fs::read_dir(&directory_path).await?;
    let mut entries = Vec::new();

    while let Some(entry) = reader.next_entry().await? {
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
        entries.push(WorkspaceDirectoryEntry {
            name,
            path: entry.path().to_string_lossy().to_string(),
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
