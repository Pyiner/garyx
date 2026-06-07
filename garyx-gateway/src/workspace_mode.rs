use std::path::PathBuf;

use garyx_models::config::GaryxConfig;
use garyx_models::local_paths::gary_home_dir;

pub(crate) fn worktree_base_dir_for_config(config: &GaryxConfig) -> PathBuf {
    config
        .sessions
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(gary_home_dir)
        .join("worktrees")
}

pub(crate) fn implicit_thread_workspace_dir_for_config(
    config: &GaryxConfig,
    thread_id: &str,
) -> PathBuf {
    config
        .sessions
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(gary_home_dir)
        .join("thread-workspaces")
        .join(safe_thread_workspace_segment(thread_id))
}

pub(crate) async fn ensure_implicit_thread_workspace_for_config(
    config: &GaryxConfig,
    thread_id: &str,
) -> Result<String, String> {
    let workspace_dir = implicit_thread_workspace_dir_for_config(config, thread_id);
    tokio::fs::create_dir_all(&workspace_dir)
        .await
        .map_err(|error| {
            format!(
                "failed to create implicit thread workspace {}: {error}",
                workspace_dir.display()
            )
        })?;
    Ok(workspace_dir.display().to_string())
}

fn safe_thread_workspace_segment(thread_id: &str) -> String {
    let sanitized: String = thread_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "thread".to_owned()
    } else {
        trimmed.to_owned()
    }
}
