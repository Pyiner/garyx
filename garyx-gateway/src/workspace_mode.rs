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
