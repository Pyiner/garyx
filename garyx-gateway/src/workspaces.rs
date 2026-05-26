use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use garyx_models::config::{GaryxConfig, WorkspaceConfig};
use garyx_models::config_loader::{ConfigWriteOptions, write_config_value_atomic};
use serde::Deserialize;
use serde_json::json;

use crate::server::AppState;

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

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn workspace_path_key(path: &str) -> String {
    path.trim().replace('\\', "/")
}

fn workspace_display_name(path: &str) -> String {
    path.trim()
        .trim_end_matches(|value| value == '/' || value == '\\')
        .rsplit(|value| value == '/' || value == '\\')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_owned()
}

fn normalize_workspace(entry: &WorkspaceConfig) -> Option<WorkspaceConfig> {
    let path = entry.path.trim();
    if path.is_empty() {
        return None;
    }
    Some(WorkspaceConfig {
        name: trim_optional(entry.name.clone()),
        path: path.to_owned(),
    })
}

fn normalized_workspaces(config: &GaryxConfig) -> Vec<WorkspaceConfig> {
    let mut seen = std::collections::HashSet::<String>::new();
    let mut workspaces = config
        .workspaces
        .iter()
        .filter_map(normalize_workspace)
        .filter(|workspace| seen.insert(workspace_path_key(&workspace.path)))
        .collect::<Vec<_>>();
    workspaces.sort_by(|left, right| {
        workspace_display_name(&left.path)
            .to_lowercase()
            .cmp(&workspace_display_name(&right.path).to_lowercase())
            .then_with(|| left.path.to_lowercase().cmp(&right.path.to_lowercase()))
    });
    workspaces
}

fn workspace_response(workspaces: Vec<WorkspaceConfig>) -> serde_json::Value {
    json!({
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

async fn persist_config(state: &Arc<AppState>, config: GaryxConfig) -> Result<(), String> {
    let value = serde_json::to_value(&config)
        .map_err(|error| format!("failed to normalize config: {error}"))?;
    if let Some(path) = state.ops.config_path.clone() {
        let write_result = tokio::task::spawn_blocking(move || {
            let write_opts = ConfigWriteOptions {
                backup_keep: 3,
                mode: Some(0o600),
            };
            write_config_value_atomic(&path, &value, &write_opts)
        })
        .await
        .map_err(|error| format!("failed to persist config file: {error}"))?;
        write_result.map_err(|error| format!("failed to persist config file: {error}"))?;
    }
    state.replace_config(config);
    Ok(())
}

pub async fn list_workspaces(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config_snapshot();
    Json(workspace_response(normalized_workspaces(&config)))
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

    let _settings_guard = state.ops.settings_mutex.lock().await;
    let mut config = (*state.config_snapshot()).clone();
    let path_key = workspace_path_key(&path);
    let mut replaced = false;
    let mut workspaces = normalized_workspaces(&config)
        .into_iter()
        .map(|mut workspace| {
            if workspace_path_key(&workspace.path) == path_key {
                workspace.path = path.clone();
                workspace.name = name.clone();
                replaced = true;
            }
            workspace
        })
        .collect::<Vec<_>>();
    if !replaced {
        workspaces.push(WorkspaceConfig {
            name: name.clone(),
            path: path.clone(),
        });
    }
    config.workspaces = workspaces;
    if let Err(error) = persist_config(&state, config.clone()).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        );
    }

    (
        StatusCode::OK,
        Json(workspace_response(normalized_workspaces(&config))),
    )
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

    let _settings_guard = state.ops.settings_mutex.lock().await;
    let mut config = (*state.config_snapshot()).clone();
    let path_key = workspace_path_key(&path);
    config.workspaces = normalized_workspaces(&config)
        .into_iter()
        .filter(|workspace| workspace_path_key(&workspace.path) != path_key)
        .collect();
    if let Err(error) = persist_config(&state, config.clone()).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        );
    }

    (
        StatusCode::OK,
        Json(workspace_response(normalized_workspaces(&config))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_workspaces_deduplicates_without_inferred_inputs() {
        let mut config = GaryxConfig::default();
        config.workspaces = vec![
            WorkspaceConfig {
                name: Some(" Repo ".to_owned()),
                path: " /workspace/repo ".to_owned(),
            },
            WorkspaceConfig {
                name: None,
                path: "/workspace/repo".to_owned(),
            },
            WorkspaceConfig {
                name: None,
                path: "   ".to_owned(),
            },
        ];

        let workspaces = normalized_workspaces(&config);

        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].name.as_deref(), Some("Repo"));
        assert_eq!(workspaces[0].path, "/workspace/repo");
    }
}
