use std::path::{Path, PathBuf};

use garyx_models::local_paths::gary_home_dir;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::process::Command;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    #[default]
    Direct,
    Worktree,
}

impl WorkspaceMode {
    pub fn is_worktree(self) -> bool {
        matches!(self, Self::Worktree)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceGitStatus {
    pub workspace_dir: String,
    pub is_git_repo: bool,
    pub repo_root: Option<String>,
    pub current_branch: Option<String>,
    pub is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWorktree {
    pub worktree_dir: String,
    pub metadata: Value,
}

pub async fn workspace_git_status(
    workspace_dir: impl AsRef<Path>,
) -> Result<WorkspaceGitStatus, String> {
    let workspace_dir = workspace_dir.as_ref();
    let workspace_display = workspace_dir.display().to_string();
    let canonical_workspace = match std::fs::canonicalize(workspace_dir) {
        Ok(path) => path,
        Err(_) => {
            return Ok(WorkspaceGitStatus {
                workspace_dir: workspace_display,
                is_git_repo: false,
                repo_root: None,
                current_branch: None,
                is_dirty: false,
            });
        }
    };
    let repo_root_output =
        match git_output(&canonical_workspace, &["rev-parse", "--show-toplevel"]).await {
            Ok(value) => value,
            Err(_) => {
                return Ok(WorkspaceGitStatus {
                    workspace_dir: canonical_workspace.display().to_string(),
                    is_git_repo: false,
                    repo_root: None,
                    current_branch: None,
                    is_dirty: false,
                });
            }
        };
    let repo_root = PathBuf::from(repo_root_output.trim());
    let canonical_repo_root = std::fs::canonicalize(&repo_root).unwrap_or(repo_root);
    let is_git_repo = canonical_repo_root == canonical_workspace;
    let current_branch = git_output(&canonical_repo_root, &["branch", "--show-current"])
        .await
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let is_dirty = git_output(&canonical_repo_root, &["status", "--porcelain=v1"])
        .await
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    Ok(WorkspaceGitStatus {
        workspace_dir: canonical_workspace.display().to_string(),
        is_git_repo,
        repo_root: Some(canonical_repo_root.display().to_string()),
        current_branch,
        is_dirty,
    })
}

pub async fn prepare_thread_worktree(
    thread_id: &str,
    workspace_dir: &str,
    worktree_base_dir: Option<&Path>,
) -> Result<PreparedWorktree, String> {
    let status = workspace_git_status(workspace_dir).await?;
    if !status.is_git_repo {
        return Err(
            "workspace_mode=worktree requires workspace_dir to be a git repository root".to_owned(),
        );
    }
    let source_repo_root = status.repo_root.clone().ok_or_else(|| {
        "workspace_mode=worktree requires workspace_dir to be a git repository root".to_owned()
    })?;
    let source_repo_root_path = PathBuf::from(&source_repo_root);
    let base_commit = git_output(&source_repo_root_path, &["rev-parse", "HEAD"]).await?;
    let repo_hash = short_hash(&source_repo_root);
    let safe_thread_id = safe_path_segment(thread_id);
    let root = worktree_base_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| gary_home_dir().join("worktrees"));
    let worktree_dir = root.join(repo_hash).join(safe_thread_id);
    if let Some(parent) = worktree_dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("failed to create worktree parent directory: {error}"))?;
    }
    let branch = next_available_branch(&source_repo_root_path, thread_id).await?;
    let worktree_dir_string = worktree_dir.display().to_string();
    if let Err(error) = git_output(
        &source_repo_root_path,
        &["worktree", "add", "-b", &branch, &worktree_dir_string],
    )
    .await
    {
        let rollback = cleanup_failed_worktree(&source_repo_root_path, &worktree_dir).await;
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback_error) => format!("{error}; rollback failed: {rollback_error}"),
        });
    }

    Ok(PreparedWorktree {
        worktree_dir: worktree_dir_string.clone(),
        metadata: json!({
            "enabled": true,
            "source_repo_root": source_repo_root,
            "source_branch": status.current_branch,
            "base_commit": base_commit,
            "branch": branch,
            "worktree_dir": worktree_dir_string,
        }),
    })
}

async fn next_available_branch(repo: &Path, thread_id: &str) -> Result<String, String> {
    let short = short_thread_id(thread_id);
    for suffix in 0..100 {
        let branch = if suffix == 0 {
            format!("garyx/{short}")
        } else {
            format!("garyx/{short}-{}", suffix + 1)
        };
        if !git_ref_exists(repo, &format!("refs/heads/{branch}")).await? {
            return Ok(branch);
        }
    }
    Err(format!(
        "failed to allocate a unique worktree branch for {thread_id}"
    ))
}

async fn git_ref_exists(repo: &Path, reference: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", reference])
        .output()
        .await
        .map_err(|error| format!("failed to run git show-ref: {error}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git show-ref failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

async fn cleanup_failed_worktree(repo: &Path, worktree_dir: &Path) -> Result<(), String> {
    let worktree_dir_string = worktree_dir.display().to_string();
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "remove", "--force", &worktree_dir_string])
        .output()
        .await
        .map_err(|error| format!("failed to run git worktree remove: {error}"))?;
    if output.status.success() || !worktree_dir.exists() {
        return Ok(());
    }
    tokio::fs::remove_dir_all(worktree_dir)
        .await
        .map_err(|error| format!("failed to remove partial worktree directory: {error}"))
}

async fn git_output(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .await
        .map_err(|error| format!("failed to run git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn short_thread_id(thread_id: &str) -> String {
    thread_id
        .trim_start_matches("thread::")
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn safe_path_segment(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "thread".to_owned()
    } else {
        out.to_owned()
    }
}
