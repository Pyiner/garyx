//! Workspace git status handler.

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitStatusParams {
    #[serde(default, alias = "workspace_dir")]
    pub workspace_dir: String,
}

/// GET /api/workspaces/git-status - report whether a workspace can use worktree mode
pub async fn workspace_git_status(
    Query(params): Query<WorkspaceGitStatusParams>,
) -> impl IntoResponse {
    let workspace_dir = params.workspace_dir.trim();
    if workspace_dir.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "workspace_dir is required" })),
        );
    }
    match router_workspace_git_status(workspace_dir).await {
        Ok(status) => (StatusCode::OK, Json(json!(status))),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
    }
}
