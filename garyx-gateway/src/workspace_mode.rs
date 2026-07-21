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

pub(crate) fn worktree_base_dir_for_data_dir(data_dir: &std::path::Path) -> PathBuf {
    data_dir
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.to_path_buf())
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

pub(crate) fn implicit_thread_workspace_dir_for_data_dir(
    data_dir: &std::path::Path,
    thread_id: &str,
) -> PathBuf {
    data_dir
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.to_path_buf())
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

/// Inferred workspace provenance for one thread, from immutable facts: the
/// implicit Garyx-managed thread workspace path embeds the thread's own
/// sanitized id, which cannot exist when a user picks a directory. This is
/// the fallback for records that predate the persisted `workspace_origin`
/// field; new implicit creations write the field explicitly at creation
/// time, and the projection persists whichever value applies.
pub(crate) fn thread_workspace_origin(
    thread_id: &str,
    workspace_dir: Option<&str>,
) -> &'static str {
    let Some(dir) = workspace_dir.map(str::trim).filter(|dir| !dir.is_empty()) else {
        return "implicit";
    };
    let implicit_suffix = format!(
        "/thread-workspaces/{}",
        safe_thread_workspace_segment(thread_id)
    );
    if dir.ends_with(&implicit_suffix) {
        "implicit"
    } else {
        "explicit"
    }
}

/// Resolve a thread's effective provenance: a persisted record value wins;
/// records that predate the field fall back to inference.
pub(crate) fn effective_workspace_origin(
    thread_id: &str,
    workspace_dir: Option<&str>,
    recorded_origin: Option<&str>,
) -> &'static str {
    match recorded_origin.map(str::trim) {
        Some("implicit") => "implicit",
        Some("explicit") => "explicit",
        _ => thread_workspace_origin(thread_id, workspace_dir),
    }
}

/// A fork inherits the source thread's workspace AND its provenance: a fork
/// of an implicit thread stays implicit even though the managed path embeds
/// the source's id, not the fork's.
pub(crate) fn fork_inherited_workspace_origin(
    source_thread_id: &str,
    source_thread_data: &serde_json::Value,
) -> String {
    let source_workspace_dir = source_thread_data
        .get("workspace_dir")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let recorded = source_thread_data
        .get("workspace_origin")
        .and_then(serde_json::Value::as_str);
    effective_workspace_origin(source_thread_id, source_workspace_dir, recorded).to_owned()
}

/// The root-workspace membership of one thread: explicit threads map to
/// their chosen directory, worktree threads map back to the worktree's
/// source workspace, implicit threads map to None. This Rust function is
/// the only derivation — the projection writes its result into the plain
/// `thread_meta.root_workspace_path` column in the same transaction.
pub(crate) fn thread_root_workspace_path(
    origin: &str,
    workspace_dir: Option<&str>,
    worktree: &serde_json::Value,
) -> Option<String> {
    if origin == "implicit" {
        return None;
    }
    let source = worktree
        .get("source_workspace_dir")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    source
        .map(ToOwned::to_owned)
        .or_else(|| workspace_dir.map(str::trim).map(ToOwned::to_owned))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const THREAD_ID: &str = "thread::aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    #[test]
    fn workspace_origin_is_derived_from_immutable_facts() {
        assert_eq!(thread_workspace_origin(THREAD_ID, None), "implicit");
        assert_eq!(thread_workspace_origin(THREAD_ID, Some("  ")), "implicit");
        assert_eq!(
            thread_workspace_origin(
                THREAD_ID,
                Some("/data/thread-workspaces/thread--aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
            ),
            "implicit",
        );
        assert_eq!(
            thread_workspace_origin(THREAD_ID, Some("/workspace/repo")),
            "explicit",
        );
        // A different thread's managed directory is not this thread's
        // implicit workspace.
        assert_eq!(
            thread_workspace_origin(
                THREAD_ID,
                Some("/data/thread-workspaces/thread--11111111-2222-3333-4444-555555555555"),
            ),
            "explicit",
        );
    }

    #[test]
    fn root_workspace_path_maps_worktrees_back_to_their_source() {
        assert_eq!(
            thread_root_workspace_path("explicit", Some("/workspace/repo"), &json!(null)),
            Some("/workspace/repo".to_owned()),
        );
        assert_eq!(
            thread_root_workspace_path(
                "explicit",
                Some("/data/worktrees/repo/thread-aaaa"),
                &json!({ "source_workspace_dir": "/workspace/repo" }),
            ),
            Some("/workspace/repo".to_owned()),
        );
        assert_eq!(
            thread_root_workspace_path(
                "implicit",
                Some("/data/thread-workspaces/thread--aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
                &json!(null),
            ),
            None,
        );
    }

    #[test]
    fn recorded_origin_wins_over_inference() {
        assert_eq!(
            effective_workspace_origin(THREAD_ID, Some("/workspace/repo"), Some("implicit")),
            "implicit",
        );
        assert_eq!(
            effective_workspace_origin(THREAD_ID, None, Some("explicit")),
            "explicit",
        );
        // Unknown or missing recorded values fall back to inference.
        assert_eq!(
            effective_workspace_origin(THREAD_ID, Some("/workspace/repo"), Some("weird")),
            "explicit",
        );
        assert_eq!(
            effective_workspace_origin(THREAD_ID, None, None),
            "implicit",
        );
    }

    #[test]
    fn inference_handles_unusual_but_legal_thread_ids() {
        // Thread ids are only prefix-validated; the sanitizer replaces every
        // non-alphanumeric character. The single Rust derivation must agree
        // with the managed-path layout for these too (the retired SQL
        // generated column replaced only `:` and disagreed here).
        let unusual = "thread::with/slash";
        let managed = format!(
            "/data/thread-workspaces/{}",
            safe_thread_workspace_segment(unusual)
        );
        assert_eq!(safe_thread_workspace_segment(unusual), "thread--with-slash");
        assert_eq!(thread_workspace_origin(unusual, Some(&managed)), "implicit",);
    }
}
